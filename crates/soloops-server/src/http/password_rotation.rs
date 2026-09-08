//! Periodic Owner password rotation with email delivery.
//!
//! The service periodically generates a strong random password, emails it to
//! the configured recipient, and only then replaces the stored Argon2id hash
//! and revokes every active session. Emailing before committing the new hash
//! guarantees the owner always holds a working credential: if delivery fails
//! the previous password stays active and the cycle retries.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use rand::{Rng, distr::Alphanumeric};
use soloops_storage::{Database, PasswordRotationRecord, now_ms};
use tokio::sync::watch;
use tracing::{error, info, warn};

use super::{PasswordRotationConfig, mail::MailDelivery, telemetry::Metrics};

const CHECK_INTERVAL: Duration = Duration::from_secs(60);
const GENERATED_PASSWORD_LEN: usize = 24;

#[derive(Clone)]
pub struct PasswordRotationService {
    database: Database,
    config: PasswordRotationConfig,
    delivery: MailDelivery,
    metrics: Arc<Metrics>,
}

impl PasswordRotationService {
    pub(super) fn new(
        database: Database,
        config: &PasswordRotationConfig,
        metrics: Arc<Metrics>,
        delivery: MailDelivery,
    ) -> Self {
        Self {
            database,
            config: config.clone(),
            delivery,
            metrics,
        }
    }

    #[cfg(test)]
    fn with_delivery(
        database: Database,
        config: PasswordRotationConfig,
        delivery: MailDelivery,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            database,
            config,
            delivery,
            metrics,
        }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        if !self.config.enabled {
            info!("password rotation disabled");
            return;
        }
        if !self.delivery.smtp_configured().await {
            warn!(
                "password rotation is enabled but SMTP is not configured yet; \
                 set it up in Settings to receive rotated passwords"
            );
        }
        info!(
            interval_hours = self.config.interval_hours,
            recipient = %self.config.recipient_email,
            "password rotation monitor started"
        );
        loop {
            if *shutdown.borrow() {
                break;
            }
            if let Err(caught) = self.run_once().await {
                error!(%caught, "password rotation cycle failed");
            }
            tokio::select! {
                _ = tokio::time::sleep(CHECK_INTERVAL) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
        info!("password rotation monitor stopped");
    }

    /// Runs one due check and rotates the owner password when the interval
    /// elapsed or the state table has no record yet (bootstrap rotation).
    pub(super) async fn run_once(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        if !self.delivery.smtp_configured().await {
            return Ok(());
        }
        let Some(owner) = self.database.find_first_owner().await? else {
            return Ok(());
        };
        let interval_ms = self.config.interval_hours * 60 * 60 * 1000;
        let state = self.database.get_password_rotation_state(&owner.id).await?;
        if !rotation_due(&state, now_ms(), interval_ms) {
            return Ok(());
        }
        let password = generate_password();
        let rotated_at = now_ms();
        let subject = "[SoloOps] Owner 密码已自动轮换".to_owned();
        let body = format!(
            "SoloOps 已自动轮换 Owner 密码。\n\n用户名：{username}\n新密码：{password}\n轮换时间：{rotated_at}\n下次轮换：约 {interval_hours} 小时后\n\n旧密码已失效，所有现有会话已被撤销；请使用新密码登录 SoloOps。\n此邮件包含敏感凭据，请妥善保管并及时删除。\n",
            username = owner.username,
            rotated_at = super::notifications::format_timestamp(rotated_at),
            interval_hours = self.config.interval_hours,
        );
        match self
            .delivery
            .send_email(&self.config.recipient_email, subject, body)
            .await
        {
            Ok(()) => {
                let password_hash = hash_password_blocking(password.as_bytes()).await?;
                self.database
                    .replace_owner_password(&owner.id, &password_hash, "system", "auth.password_rotated")
                    .await?;
                self.database
                    .record_password_rotation_success(&owner.id, rotated_at)
                    .await?;
                self.metrics
                    .password_rotations
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                info!(owner_id = %owner.id, "owner password rotated and emailed");
            }
            Err(caught) => {
                self.database
                    .record_password_rotation_failure(&owner.id, now_ms(), &caught.to_string())
                    .await?;
                self.metrics
                    .password_rotation_email_failures
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                warn!(owner_id = %owner.id, %caught, "password rotation email failed; previous password kept");
            }
        }
        Ok(())
    }
}

fn rotation_due(state: &PasswordRotationRecord, now: i64, interval_ms: i64) -> bool {
    match state.last_rotated_at {
        None => true,
        Some(last_rotated_at) => now >= last_rotated_at.saturating_add(interval_ms),
    }
}

fn generate_password() -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(GENERATED_PASSWORD_LEN)
        .map(char::from)
        .collect()
}

async fn hash_password_blocking(password: &[u8]) -> Result<String> {
    let password = password.to_vec();
    tokio::task::spawn_blocking(move || -> Result<String> {
        let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>())
            .map_err(|caught| anyhow::anyhow!("failed to generate password salt: {caught}"))?;
        Ok(Argon2::default()
            .hash_password(&password, &salt)
            .map_err(|caught| anyhow::anyhow!("failed to hash password: {caught}"))?
            .to_string())
    })
    .await
    .context("password hashing task failed")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::{PasswordHash, PasswordVerifier};
    use soloops_storage::PasswordRotationRecord;
    use std::{collections::HashSet, sync::Mutex};

    struct FakeSink {
        sent: Mutex<Vec<(String, String, String)>>,
        failures: Mutex<HashSet<String>>,
    }

    impl FakeSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sent: Mutex::new(Vec::new()),
                failures: Mutex::new(HashSet::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl super::super::mail::MailSink for FakeSink {
        async fn send_raw(&self, recipient: &str, subject: String, body: String) -> Result<()> {
            if self.failures.lock().unwrap().contains(recipient) {
                anyhow::bail!("forced SMTP failure");
            }
            self.sent
                .lock()
                .unwrap()
                .push((recipient.to_owned(), subject, body));
            Ok(())
        }
    }

    fn enabled_config(recipient: &str) -> PasswordRotationConfig {
        PasswordRotationConfig {
            enabled: true,
            interval_hours: 168,
            recipient_email: recipient.to_owned(),
        }
    }

    fn test_service(
        database: Database,
        config: PasswordRotationConfig,
        sink: Arc<dyn super::super::mail::MailSink>,
        metrics: Arc<Metrics>,
    ) -> PasswordRotationService {
        PasswordRotationService::with_delivery(database, config, MailDelivery::fixed(sink), metrics)
    }

    async fn migrated_database() -> Database {
        let database = Database::connect(":memory:").await.unwrap();
        database.migrate().await.unwrap();
        database
    }

    fn extract_password(body: &str) -> String {
        body.split("新密码：")
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .expect("email body must contain the new password")
            .to_owned()
    }

    #[test]
    fn rotation_is_due_on_bootstrap_and_after_the_interval() {
        let mut state = PasswordRotationRecord {
            owner_id: "owner".into(),
            last_rotated_at: None,
            last_email_at: None,
            last_email_error: None,
        };
        assert!(rotation_due(&state, 1_000, 60_000));
        state.last_rotated_at = Some(1_000);
        assert!(!rotation_due(&state, 60_000, 60_000));
        assert!(rotation_due(&state, 61_000, 60_000));
    }

    #[tokio::test]
    async fn bootstrap_rotation_emails_password_updates_hash_and_revokes_sessions() {
        let database = migrated_database().await;
        let owner = database.create_owner("owner", "old-hash").await.unwrap();
        database
            .create_session_with_audit(&owner.id, "token-a", now_ms() + 60_000, serde_json::json!({}))
            .await
            .unwrap();
        let sink = FakeSink::new();
        let service = test_service(
            database.clone(),
            enabled_config("owner@example.com"),
            sink.clone(),
            Arc::new(Metrics::default()),
        );

        service.run_once().await.unwrap();

        let password = {
            let sent = sink.sent.lock().unwrap();
            assert_eq!(sent.len(), 1, "bootstrap rotation must email exactly once");
            assert_eq!(sent[0].0, "owner@example.com");
            assert!(sent[0].1.contains("密码已自动轮换"));
            assert!(sent[0].2.contains("用户名：owner"));
            let password = extract_password(&sent[0].2);
            assert_eq!(password.chars().count(), GENERATED_PASSWORD_LEN);
            password
        };

        let user = database.find_first_owner().await.unwrap().unwrap();
        assert_ne!(user.password_hash, "old-hash");
        let parsed = PasswordHash::new(&user.password_hash).unwrap();
        assert!(
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        );
        assert!(database.find_session_owner("token-a").await.unwrap().is_none());

        let state = database.get_password_rotation_state(&owner.id).await.unwrap();
        assert!(state.last_rotated_at.is_some());
        assert!(state.last_email_error.is_none());

        service.run_once().await.unwrap();
        assert_eq!(
            sink.sent.lock().unwrap().len(),
            1,
            "second cycle must not rotate before the interval elapses"
        );
    }

    #[tokio::test]
    async fn email_failure_keeps_previous_password_and_schedules_retry() {
        let database = migrated_database().await;
        let owner = database.create_owner("owner", "old-hash").await.unwrap();
        let sink = FakeSink::new();
        sink.failures.lock().unwrap().insert("owner@example.com".into());
        let service = test_service(
            database.clone(),
            enabled_config("owner@example.com"),
            sink.clone(),
            Arc::new(Metrics::default()),
        );

        service.run_once().await.unwrap();

        assert_eq!(sink.sent.lock().unwrap().len(), 0);
        let user = database.find_first_owner().await.unwrap().unwrap();
        assert_eq!(user.password_hash, "old-hash", "hash must be untouched");
        let state = database.get_password_rotation_state(&owner.id).await.unwrap();
        assert_eq!(state.last_rotated_at, None);
        assert!(state.last_email_error.is_some());

        sink.failures.lock().unwrap().clear();
        service.run_once().await.unwrap();
        assert_eq!(sink.sent.lock().unwrap().len(), 1);
        let user = database.find_first_owner().await.unwrap().unwrap();
        assert_ne!(user.password_hash, "old-hash");
    }

    #[tokio::test]
    async fn missing_owner_or_disabled_config_is_a_noop() {
        let database = migrated_database().await;
        let sink = FakeSink::new();
        let service = test_service(
            database,
            enabled_config("owner@example.com"),
            sink.clone(),
            Arc::new(Metrics::default()),
        );
        service.run_once().await.unwrap();
        assert_eq!(sink.sent.lock().unwrap().len(), 0);

        let database = migrated_database().await;
        let _ = database.create_owner("owner", "old-hash").await.unwrap();
        let disabled = test_service(
            database,
            PasswordRotationConfig {
                enabled: false,
                interval_hours: 168,
                recipient_email: String::new(),
            },
            sink.clone(),
            Arc::new(Metrics::default()),
        );
        disabled.run_once().await.unwrap();
        assert_eq!(sink.sent.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn metrics_count_rotations_and_email_failures() {
        let database = migrated_database().await;
        let _ = database.create_owner("owner", "old-hash").await.unwrap();
        let metrics = Arc::new(Metrics::default());
        let sink = FakeSink::new();
        sink.failures.lock().unwrap().insert("owner@example.com".into());
        let failing = PasswordRotationService::with_delivery(
            database,
            enabled_config("owner@example.com"),
            MailDelivery::fixed(sink.clone()),
            metrics.clone(),
        );
        failing.run_once().await.unwrap();
        assert_eq!(
            metrics
                .password_rotation_email_failures
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            metrics
                .password_rotations
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );

        sink.failures.lock().unwrap().clear();
        failing.run_once().await.unwrap();
        assert_eq!(
            metrics
                .password_rotations
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }
}
