use std::{net::Ipv4Addr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use soloops_domain::{IpNotificationRecipientStatus, IpNotificationSettings, TestIpNotificationResponse};
use soloops_storage::{AuditEntry, Database, IpNotificationRecord, now_ms};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::{Notify, watch};
use tracing::{error, info, warn};

use super::{AppConfig, SmtpConfig, SmtpSecurity, telemetry::Metrics};

#[async_trait]
trait PublicIpSource: Send + Sync {
    async fn fetch_ipv4(&self) -> Result<Ipv4Addr>;
}

struct HttpPublicIpSource {
    client: reqwest::Client,
    endpoint: String,
}

#[async_trait]
impl PublicIpSource for HttpPublicIpSource {
    async fn fetch_ipv4(&self) -> Result<Ipv4Addr> {
        let mut response = self
            .client
            .get(&self.endpoint)
            .send()
            .await
            .context("public IP request failed")?
            .error_for_status()
            .context("public IP endpoint returned an error status")?;
        if response.content_length().is_some_and(|length| length > 64) {
            anyhow::bail!("public IP response exceeds 64 bytes");
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("failed to read public IP response")?
        {
            if body.len() + chunk.len() > 64 {
                anyhow::bail!("public IP response exceeds 64 bytes");
            }
            body.extend_from_slice(&chunk);
        }
        parse_ipv4_body(&body)
    }
}

fn parse_ipv4_body(body: &[u8]) -> Result<Ipv4Addr> {
    if body.len() > 64 {
        anyhow::bail!("public IP response exceeds 64 bytes");
    }
    let value = std::str::from_utf8(body)
        .context("public IP response is not UTF-8")?
        .trim();
    value
        .parse::<Ipv4Addr>()
        .context("public IP response is not an IPv4 address")
}

#[async_trait]
trait MailSender: Send + Sync {
    async fn send_ip_notification(
        &self,
        recipient: &str,
        current: Ipv4Addr,
        previous: Option<&str>,
        detected_at: i64,
    ) -> Result<()>;
    async fn send_test(&self, recipient: &str) -> Result<()>;
}

struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    fn new(config: &SmtpConfig) -> Result<Self> {
        let mut builder = match config.security {
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
            SmtpSecurity::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host),
        }
        .context("invalid SMTP relay configuration")?
        .port(config.port);
        if let (Some(username), Some(password_env)) = (&config.username, &config.password_env) {
            let password = std::env::var(password_env).with_context(|| {
                format!("configured SMTP password environment variable {password_env} is not set")
            })?;
            if password.is_empty() {
                anyhow::bail!("configured SMTP password environment variable {password_env} is empty");
            }
            builder = builder.credentials(Credentials::new(username.clone(), password));
        }
        Ok(Self {
            transport: builder.build(),
            from: config.from.parse().context("invalid SMTP sender mailbox")?,
        })
    }

    async fn send(&self, recipient: &str, subject: String, body: String) -> Result<()> {
        let message = Message::builder()
            .from(self.from.clone())
            .to(recipient
                .parse::<Mailbox>()
                .context("invalid recipient mailbox")?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .context("failed to build email")?;
        self.transport.send(message).await.context("SMTP send failed")?;
        Ok(())
    }
}

#[async_trait]
impl MailSender for SmtpMailer {
    async fn send_ip_notification(
        &self,
        recipient: &str,
        current: Ipv4Addr,
        previous: Option<&str>,
        detected_at: i64,
    ) -> Result<()> {
        let first = previous.is_none();
        let subject = if first {
            format!("[SoloOps] 当前公网 IPv4：{current}")
        } else {
            format!("[SoloOps] 公网 IPv4 已变更：{current}")
        };
        let detected_at = format_timestamp(detected_at);
        let body = format!(
            "SoloOps 检测到公网 IPv4。\n\n当前 IPv4：{current}\n此前 IPv4：{}\n检测时间：{detected_at}\n",
            previous.unwrap_or("未记录")
        );
        self.send(recipient, subject, body).await
    }

    async fn send_test(&self, recipient: &str) -> Result<()> {
        self.send(
            recipient,
            "[SoloOps] 公网 IP 通知测试".into(),
            "这是一封 SoloOps 测试邮件。公网 IPv4 通知邮件配置工作正常。\n".into(),
        )
        .await
    }
}

fn format_timestamp(timestamp_ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(timestamp_ms) * 1_000_000)
        .ok()
        .and_then(|value| value.format(&Rfc3339).ok())
        .unwrap_or_else(|| timestamp_ms.to_string())
}

#[derive(Clone)]
pub struct NotificationService {
    database: Database,
    source: Arc<dyn PublicIpSource>,
    mailer: Option<Arc<dyn MailSender>>,
    metrics: Arc<Metrics>,
    wake: Arc<Notify>,
    poll_interval: Duration,
}

impl NotificationService {
    pub(super) fn new(database: Database, config: &AppConfig, metrics: Arc<Metrics>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("failed to construct public IP HTTP client")?;
        let mailer = config
            .smtp
            .as_ref()
            .map(SmtpMailer::new)
            .transpose()?
            .map(|mailer| Arc::new(mailer) as Arc<dyn MailSender>);
        Ok(Self {
            database,
            source: Arc::new(HttpPublicIpSource {
                client,
                endpoint: config.public_ip_endpoint.clone(),
            }),
            mailer,
            metrics,
            wake: Arc::new(Notify::new()),
            poll_interval: Duration::from_secs(config.public_ip_poll_seconds),
        })
    }

    pub fn smtp_configured(&self) -> bool {
        self.mailer.is_some()
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub async fn settings(&self, owner_id: &str) -> Result<IpNotificationSettings> {
        Ok(settings_response(
            self.database.get_ip_notification_settings(owner_id).await?,
            self.smtp_configured(),
        ))
    }

    pub async fn test(&self, owner_id: &str) -> Result<TestIpNotificationResponse> {
        let mailer = self.mailer.as_ref().context("SMTP is not configured")?;
        let settings = self.database.get_ip_notification_settings(owner_id).await?;
        let mut sent_count = 0;
        let mut failed_recipients = Vec::new();
        for recipient in settings.recipients {
            match mailer.send_test(&recipient.email).await {
                Ok(()) => sent_count += 1,
                Err(error) => {
                    warn!(owner_id, error = %error, "test notification email failed");
                    failed_recipients.push(recipient.email);
                }
            }
        }
        self.database
            .write_audit(AuditEntry {
                actor_type: "owner",
                actor_id: Some(owner_id.to_owned()),
                action: "settings.ip_notifications.test",
                object_type: Some("ip_notification_settings"),
                object_id: Some(owner_id.to_owned()),
                outcome: if failed_recipients.is_empty() {
                    "success"
                } else {
                    "failure"
                },
                context: serde_json::json!({
                    "sentCount": sent_count,
                    "failedCount": failed_recipients.len(),
                }),
            })
            .await?;
        Ok(TestIpNotificationResponse {
            sent_count,
            failed_recipients,
        })
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            self.run_once().await;
            tokio::select! {
                _ = tokio::time::sleep(self.poll_interval) => {}
                _ = self.wake.notified() => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        }
        info!("public IP notification monitor stopped");
    }

    async fn run_once(&self) {
        let Some(mailer) = &self.mailer else { return };
        let settings = match self.database.enabled_ip_notification_settings().await {
            Ok(settings) => settings,
            Err(error) => {
                error!(%error, "failed to load public IP notification settings");
                return;
            }
        };
        if settings.iter().all(|setting| setting.recipients.is_empty()) {
            return;
        }
        self.metrics
            .public_ip_checks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let current = match self.source.fetch_ipv4().await {
            Ok(ip) => ip,
            Err(error) => {
                self.metrics
                    .public_ip_check_failures
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                warn!(%error, "public IPv4 check failed");
                return;
            }
        };
        let detected_at = now_ms();
        for setting in settings {
            if let Err(error) = self
                .notify_owner(&setting, mailer.as_ref(), current, detected_at)
                .await
            {
                error!(owner_id = setting.owner_id, %error, "public IP notification cycle failed");
            }
        }
    }

    async fn notify_owner(
        &self,
        setting: &IpNotificationRecord,
        mailer: &dyn MailSender,
        current: Ipv4Addr,
        detected_at: i64,
    ) -> Result<()> {
        let current_text = current.to_string();
        self.database
            .record_public_ipv4(&setting.owner_id, &current_text, detected_at)
            .await?;
        for recipient in &setting.recipients {
            if recipient.last_notified_ipv4.as_deref() == Some(current_text.as_str()) {
                continue;
            }
            match mailer
                .send_ip_notification(
                    &recipient.email,
                    current,
                    recipient.last_notified_ipv4.as_deref(),
                    detected_at,
                )
                .await
            {
                Ok(()) => {
                    self.database
                        .record_ip_notification_success(
                            &setting.owner_id,
                            &recipient.email,
                            &current_text,
                            detected_at,
                        )
                        .await?;
                    self.metrics
                        .ip_notification_emails_sent
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if let Err(error) = self
                        .write_delivery_audit(
                            setting,
                            recipient.last_notified_ipv4.as_deref(),
                            &current_text,
                            "success",
                        )
                        .await
                    {
                        warn!(owner_id = setting.owner_id, %error, "failed to audit public IP notification delivery");
                    }
                }
                Err(error) => {
                    warn!(owner_id = setting.owner_id, error = %error, "public IP notification email failed");
                    self.database
                        .record_ip_notification_failure(
                            &setting.owner_id,
                            &recipient.email,
                            detected_at,
                            "smtp_send_failed",
                        )
                        .await?;
                    self.metrics
                        .ip_notification_email_failures
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if let Err(error) = self
                        .write_delivery_audit(
                            setting,
                            recipient.last_notified_ipv4.as_deref(),
                            &current_text,
                            "failure",
                        )
                        .await
                    {
                        warn!(owner_id = setting.owner_id, %error, "failed to audit public IP notification failure");
                    }
                }
            }
        }
        Ok(())
    }

    async fn write_delivery_audit(
        &self,
        setting: &IpNotificationRecord,
        previous: Option<&str>,
        current: &str,
        outcome: &'static str,
    ) -> Result<()> {
        self.database
            .write_audit(AuditEntry {
                actor_type: "system",
                actor_id: None,
                action: "public_ip.notification",
                object_type: Some("ip_notification_settings"),
                object_id: Some(setting.owner_id.clone()),
                outcome,
                context: serde_json::json!({ "previousIpv4": previous, "currentIpv4": current }),
            })
            .await?;
        Ok(())
    }
}

fn settings_response(record: IpNotificationRecord, smtp_configured: bool) -> IpNotificationSettings {
    IpNotificationSettings {
        enabled: record.enabled,
        smtp_configured,
        current_ipv4: record.current_ipv4,
        last_checked_at: record.last_checked_at,
        last_changed_at: record.last_changed_at,
        recipients: record
            .recipients
            .into_iter()
            .map(|recipient| IpNotificationRecipientStatus {
                email: recipient.email,
                last_notified_ipv4: recipient.last_notified_ipv4,
                last_notified_at: recipient.last_notified_at,
                last_attempt_at: recipient.last_attempt_at,
                last_error: recipient.last_error,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashSet, sync::Mutex};

    struct FakeSource(Mutex<Ipv4Addr>);

    #[async_trait]
    impl PublicIpSource for FakeSource {
        async fn fetch_ipv4(&self) -> Result<Ipv4Addr> {
            Ok(*self.0.lock().unwrap())
        }
    }

    #[derive(Default)]
    struct FakeMailer {
        sent: Mutex<Vec<(String, Ipv4Addr)>>,
        failures: Mutex<HashSet<String>>,
    }

    #[async_trait]
    impl MailSender for FakeMailer {
        async fn send_ip_notification(
            &self,
            recipient: &str,
            current: Ipv4Addr,
            _previous: Option<&str>,
            _detected_at: i64,
        ) -> Result<()> {
            self.sent.lock().unwrap().push((recipient.to_owned(), current));
            if self.failures.lock().unwrap().contains(recipient) {
                anyhow::bail!("forced SMTP failure");
            }
            Ok(())
        }

        async fn send_test(&self, _recipient: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn formats_utc_timestamp_as_rfc3339() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn parses_only_bounded_ipv4_responses() {
        assert_eq!(
            parse_ipv4_body(b" 203.0.113.7\n").unwrap(),
            Ipv4Addr::new(203, 0, 113, 7)
        );
        assert!(parse_ipv4_body(b"2001:db8::1").is_err());
        assert!(parse_ipv4_body(&[b'1'; 65]).is_err());
    }

    #[tokio::test]
    async fn sends_initial_and_changed_ip_and_retries_only_failed_recipients() {
        let database = Database::connect(":memory:").await.unwrap();
        database.migrate().await.unwrap();
        let owner = database.create_owner("owner", "hash").await.unwrap();
        let recipients = vec!["a@example.com".into(), "b@example.com".into()];
        database
            .update_ip_notification_settings(&owner.id, true, &recipients)
            .await
            .unwrap();
        let source = Arc::new(FakeSource(Mutex::new(Ipv4Addr::new(203, 0, 113, 1))));
        let mailer = Arc::new(FakeMailer::default());
        let service = NotificationService {
            database: database.clone(),
            source: source.clone(),
            mailer: Some(mailer.clone()),
            metrics: Arc::new(Metrics::default()),
            wake: Arc::new(Notify::new()),
            poll_interval: Duration::from_secs(300),
        };

        service.run_once().await;
        assert_eq!(mailer.sent.lock().unwrap().len(), 2);
        service.run_once().await;
        assert_eq!(mailer.sent.lock().unwrap().len(), 2);

        *source.0.lock().unwrap() = Ipv4Addr::new(203, 0, 113, 2);
        mailer.failures.lock().unwrap().insert("b@example.com".into());
        service.run_once().await;
        assert_eq!(mailer.sent.lock().unwrap().len(), 4);
        let settings = database.get_ip_notification_settings(&owner.id).await.unwrap();
        assert_eq!(
            settings.recipients[0].last_notified_ipv4.as_deref(),
            Some("203.0.113.2")
        );
        assert_eq!(
            settings.recipients[1].last_notified_ipv4.as_deref(),
            Some("203.0.113.1")
        );
        assert_eq!(
            settings.recipients[1].last_error.as_deref(),
            Some("smtp_send_failed")
        );

        mailer.failures.lock().unwrap().clear();
        service.run_once().await;
        assert_eq!(mailer.sent.lock().unwrap().len(), 5);
        let settings = database.get_ip_notification_settings(&owner.id).await.unwrap();
        assert!(settings.recipients.iter().all(|recipient| {
            recipient.last_notified_ipv4.as_deref() == Some("203.0.113.2") && recipient.last_error.is_none()
        }));
    }
}
