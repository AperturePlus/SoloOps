//! Owner-managed mail delivery.
//!
//! `MailDelivery` resolves the effective SMTP transport on every cycle:
//! values saved from the WebUI (database) take precedence, otherwise the
//! `SOLOOPS_SMTP_*` environment configuration is used as the bootstrap
//! fallback. Rebuilt mailers are cached by a configuration signature so the
//! services only reconnect when the configuration actually changes.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use soloops_storage::{Database, SmtpSettingsRecord};
use tokio::sync::RwLock;
use tracing::warn;

use super::{SmtpConfig, SmtpSecurity};

/// A fully resolved SMTP endpoint. The password is plaintext and never
/// serialized; it only lives long enough to build the transport.
pub(super) struct SmtpEndpoint {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub from: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl SmtpEndpoint {
    fn signature(&self) -> String {
        format!(
            "{}|{}|{:?}|{}|{}|{}",
            self.host,
            self.port,
            self.security,
            self.from,
            self.username.as_deref().unwrap_or("-"),
            self.password.is_some(),
        )
    }
}

pub(super) struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    pub(super) fn new(endpoint: &SmtpEndpoint) -> Result<Self> {
        let mut builder = match endpoint.security {
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&endpoint.host),
            SmtpSecurity::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&endpoint.host),
        }
        .context("invalid SMTP relay configuration")?
        .port(endpoint.port);
        if let (Some(username), Some(password)) = (&endpoint.username, &endpoint.password) {
            builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from: endpoint.from.parse().context("invalid SMTP sender mailbox")?,
        })
    }

    pub(super) async fn send(&self, recipient: &str, subject: String, body: String) -> Result<()> {
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

/// Minimal send abstraction so tests can substitute fake transports.
#[async_trait]
pub(super) trait MailSink: Send + Sync {
    async fn send_raw(&self, recipient: &str, subject: String, body: String) -> Result<()>;
}

#[async_trait]
impl MailSink for SmtpMailer {
    async fn send_raw(&self, recipient: &str, subject: String, body: String) -> Result<()> {
        self.send(recipient, subject, body).await
    }
}

/// Effective SMTP values for the settings API; the credential is summarized
/// as a boolean instead of being exposed.
pub(super) struct SmtpSnapshot {
    pub source: Option<&'static str>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub security: Option<SmtpSecurity>,
    pub from: Option<String>,
    pub username: Option<String>,
    pub password_configured: bool,
    pub configured: bool,
}

struct DeliverySlot {
    signature: Option<String>,
    sink: Option<Arc<dyn MailSink>>,
}

struct DeliveryInner {
    database: Option<Database>,
    env_smtp: Option<SmtpConfig>,
    fixed: Option<Arc<dyn MailSink>>,
    slot: RwLock<DeliverySlot>,
}

#[derive(Clone)]
pub(in crate::http) struct MailDelivery {
    inner: Arc<DeliveryInner>,
}

impl MailDelivery {
    pub(super) fn new(database: Database, env_smtp: Option<SmtpConfig>) -> Self {
        Self {
            inner: Arc::new(DeliveryInner {
                database: Some(database),
                env_smtp,
                fixed: None,
                slot: RwLock::new(DeliverySlot {
                    signature: None,
                    sink: None,
                }),
            }),
        }
    }

    /// Pins a fixed sink (tests); database and environment resolution is
    /// skipped entirely in this mode.
    #[cfg(test)]
    pub(super) fn fixed(sink: Arc<dyn MailSink>) -> Self {
        Self {
            inner: Arc::new(DeliveryInner {
                database: None,
                env_smtp: None,
                fixed: Some(sink),
                slot: RwLock::new(DeliverySlot {
                    signature: None,
                    sink: None,
                }),
            }),
        }
    }

    pub(super) async fn smtp_configured(&self) -> bool {
        self.sink().await.is_some()
    }

    /// Returns the current mailer, rebuilding it when the effective
    /// configuration changed since the last call.
    pub(super) async fn sink(&self) -> Option<Arc<dyn MailSink>> {
        if let Some(fixed) = &self.inner.fixed {
            return Some(fixed.clone());
        }
        let endpoint = self.resolve_endpoint().await;
        let signature = match &endpoint {
            Some(endpoint) => format!("db|{}", endpoint.signature()),
            None => "unconfigured".to_owned(),
        };
        let mut slot = self.inner.slot.write().await;
        if slot.signature.as_deref() == Some(signature.as_str()) {
            return slot.sink.clone();
        }
        let sink = match endpoint {
            Some(endpoint) => match SmtpMailer::new(&endpoint) {
                Ok(mailer) => Some(Arc::new(mailer) as Arc<dyn MailSink>),
                Err(error) => {
                    warn!(%error, "failed to build the SMTP mailer from the current settings");
                    None
                }
            },
            None => None,
        };
        slot.signature = Some(signature);
        slot.sink = sink.clone();
        sink
    }

    pub(super) async fn send_email(&self, recipient: &str, subject: String, body: String) -> Result<()> {
        let sink = self.sink().await.context("SMTP is not configured")?;
        sink.send_raw(recipient, subject, body).await
    }

    /// Snapshot of the effective settings for the settings API. Database
    /// errors propagate; malformed stored values degrade to `configured =
    /// false` instead of failing the whole request.
    pub(super) async fn snapshot(&self) -> Result<SmtpSnapshot> {
        if let Some(record) = self.stored_settings().await? {
            return Ok(Self::snapshot_from_record(record));
        }
        Ok(match &self.inner.env_smtp {
            Some(config) => Self::snapshot_from_env(config),
            None => SmtpSnapshot {
                source: None,
                host: None,
                port: None,
                security: None,
                from: None,
                username: None,
                password_configured: false,
                configured: false,
            },
        })
    }

    async fn stored_settings(&self) -> Result<Option<SmtpSettingsRecord>> {
        let Some(database) = &self.inner.database else {
            return Ok(None);
        };
        if let Some(owner) = database.find_first_owner().await? {
            return Ok(database.get_smtp_settings(&owner.id).await?);
        }
        Ok(None)
    }

    async fn resolve_endpoint(&self) -> Option<SmtpEndpoint> {
        match self.resolve_endpoint_checked().await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                warn!(%error, "failed to resolve the effective SMTP settings");
                None
            }
        }
    }

    async fn resolve_endpoint_checked(&self) -> Result<Option<SmtpEndpoint>> {
        if let Some(record) = self.stored_settings().await? {
            return endpoint_from_record(record);
        }
        if let Some(config) = &self.inner.env_smtp {
            return endpoint_from_env(config);
        }
        Ok(None)
    }

    fn snapshot_from_record(record: SmtpSettingsRecord) -> SmtpSnapshot {
        let security = match record.security.as_str() {
            "tls" => Some(SmtpSecurity::Tls),
            "starttls" => Some(SmtpSecurity::StartTls),
            _ => None,
        };
        let endpoint = endpoint_from_record(record).ok().flatten();
        SmtpSnapshot {
            source: Some("database"),
            host: endpoint.as_ref().map(|endpoint| endpoint.host.clone()),
            port: endpoint.as_ref().map(|endpoint| endpoint.port),
            security,
            from: endpoint.as_ref().map(|endpoint| endpoint.from.clone()),
            username: endpoint.as_ref().and_then(|endpoint| endpoint.username.clone()),
            password_configured: endpoint
                .as_ref()
                .is_some_and(|endpoint| endpoint.password.is_some()),
            configured: endpoint.is_some(),
        }
    }

    fn snapshot_from_env(config: &SmtpConfig) -> SmtpSnapshot {
        let endpoint = endpoint_from_env(config).ok();
        SmtpSnapshot {
            source: Some("environment"),
            host: Some(config.host.clone()),
            port: Some(config.port),
            security: Some(config.security),
            from: Some(config.from.clone()),
            username: config.username.clone(),
            password_configured: config
                .password_env
                .as_ref()
                .and_then(|name| std::env::var(name).ok())
                .is_some_and(|value| !value.is_empty()),
            configured: endpoint.is_some(),
        }
    }
}

fn endpoint_from_record(record: SmtpSettingsRecord) -> Result<Option<SmtpEndpoint>> {
    if record.username.is_some() != record.password.is_some() {
        anyhow::bail!("stored username and password must be configured together");
    }
    let security = match record.security.as_str() {
        "tls" => SmtpSecurity::Tls,
        "starttls" => SmtpSecurity::StartTls,
        other => anyhow::bail!("invalid stored SMTP security: {other}"),
    };
    Ok(Some(SmtpEndpoint {
        host: record.host,
        port: u16::try_from(record.port).context("stored SMTP port is out of range")?,
        security,
        from: record.from_mailbox,
        username: record.username,
        password: record.password,
    }))
}

fn endpoint_from_env(config: &SmtpConfig) -> Result<Option<SmtpEndpoint>> {
    let password = match &config.password_env {
        Some(name) => {
            let password = std::env::var(name).with_context(|| {
                format!("configured SMTP password environment variable {name} is not set")
            })?;
            if password.is_empty() {
                anyhow::bail!("configured SMTP password environment variable {name} is empty");
            }
            Some(password)
        }
        None => None,
    };
    if config.username.is_some() != password.is_some() {
        anyhow::bail!("configured SMTP username and password must be configured together");
    }
    Ok(Some(SmtpEndpoint {
        host: config.host.clone(),
        port: config.port,
        security: config.security,
        from: config.from.clone(),
        username: config.username.clone(),
        password,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soloops_storage::now_ms;

    struct RecordingSink(std::sync::Mutex<Vec<(String, String, String)>>);

    #[async_trait]
    impl MailSink for RecordingSink {
        async fn send_raw(&self, recipient: &str, subject: String, body: String) -> Result<()> {
            self.0.lock().unwrap().push((recipient.to_owned(), subject, body));
            Ok(())
        }
    }

    async fn migrated_database() -> Database {
        let database = Database::connect(":memory:").await.unwrap();
        database.migrate().await.unwrap();
        database
    }

    fn stored_record(owner_id: &str, password: Option<&str>) -> SmtpSettingsRecord {
        SmtpSettingsRecord {
            owner_id: owner_id.to_owned(),
            host: "smtp.example.com".into(),
            port: 465,
            security: "tls".into(),
            from_mailbox: "SoloOps <soloops@example.com>".into(),
            username: Some("soloops".into()),
            password: password.map(str::to_owned),
            updated_at: now_ms(),
        }
    }

    #[test]
    fn env_endpoints_require_paired_credentials() {
        let config = SmtpConfig {
            host: "smtp.example.com".into(),
            port: 465,
            security: SmtpSecurity::Tls,
            from: "soloops@example.com".into(),
            username: Some("soloops".into()),
            password_env: Some("SOLOOPS_SMTP_TEST_PASSWORD".into()),
        };
        // SAFETY: a unique test-only variable name never collides with a
        // parallel test and is removed before the assertion below runs.
        unsafe { std::env::set_var("SOLOOPS_SMTP_TEST_PASSWORD", "") };
        assert!(
            endpoint_from_env(&config).is_err(),
            "an empty configured password must disable the mailer"
        );
        unsafe {
            std::env::set_var("SOLOOPS_SMTP_TEST_PASSWORD", "secret");
        }
        let endpoint = endpoint_from_env(&config).unwrap().unwrap();
        assert_eq!(endpoint.host, "smtp.example.com");
        assert_eq!(endpoint.password.as_deref(), Some("secret"));
        unsafe { std::env::remove_var("SOLOOPS_SMTP_TEST_PASSWORD") };
    }

    #[tokio::test]
    async fn database_settings_take_precedence_and_cache_by_signature() {
        let database = migrated_database().await;
        let owner = database.create_owner("owner", "hash").await.unwrap();
        let delivery = MailDelivery::new(database.clone(), None);

        assert!(!delivery.smtp_configured().await);
        assert!(delivery.snapshot().await.unwrap().source.is_none());

        database
            .upsert_smtp_settings(&stored_record(&owner.id, Some("secret")))
            .await
            .unwrap();
        assert!(delivery.smtp_configured().await);
        let snapshot = delivery.snapshot().await.unwrap();
        assert_eq!(snapshot.source, Some("database"));
        assert_eq!(snapshot.host.as_deref(), Some("smtp.example.com"));
        assert!(snapshot.password_configured);

        // Rotating the stored password changes the signature and rebuilds.
        database
            .upsert_smtp_settings(&stored_record(&owner.id, Some("rotated")))
            .await
            .unwrap();
        assert!(delivery.smtp_configured().await);

        assert!(database.delete_smtp_settings(&owner.id).await.unwrap());
        assert!(!delivery.smtp_configured().await);
        let snapshot = delivery.snapshot().await.unwrap();
        assert_eq!(snapshot.source, None);
    }

    #[tokio::test]
    async fn fixed_sink_is_used_without_resolution() {
        let sink = Arc::new(RecordingSink(std::sync::Mutex::new(Vec::new())));
        let delivery = MailDelivery::fixed(sink.clone());
        delivery
            .send_email("owner@example.com", "subject".into(), "body".into())
            .await
            .unwrap();
        let sent = sink.0.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "owner@example.com");
    }

    #[tokio::test]
    async fn unpaired_stored_credentials_disable_delivery() {
        let database = migrated_database().await;
        let owner = database.create_owner("owner", "hash").await.unwrap();
        let delivery = MailDelivery::new(database.clone(), None);
        database
            .upsert_smtp_settings(&stored_record(&owner.id, None))
            .await
            .unwrap();
        assert!(
            !delivery.smtp_configured().await,
            "stored username without a password must degrade to unconfigured"
        );
        let snapshot = delivery.snapshot().await.unwrap();
        assert_eq!(snapshot.source, Some("database"));
        assert!(!snapshot.configured);
        assert!(!snapshot.password_configured);
    }
}
