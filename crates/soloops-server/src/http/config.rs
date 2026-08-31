use super::*;

pub struct AppConfig {
    pub environment: String,
    pub host: String,
    pub api_port: u16,
    pub web_origin: String,
    pub database_path: PathBuf,
    pub web_dist: PathBuf,
    pub session_ttl_ms: i64,
    pub event_poll_ms: u64,
    pub worker_poll_ms: u64,
    pub log_filter: String,
    pub secure_cookies: bool,
    pub metrics_enabled: bool,
    pub public_ip_endpoint: String,
    pub public_ip_poll_seconds: u64,
    pub smtp: Option<SmtpConfig>,
}

#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub from: String,
    pub username: Option<String>,
    pub password_env: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpSecurity {
    Tls,
    StartTls,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let _ = dotenvy::dotenv();
        let environment = env("SOLOOPS_ENV", "development");
        let api_port = env("SOLOOPS_API_PORT", "3001")
            .parse()
            .context("SOLOOPS_API_PORT must be a valid TCP port")?;
        let session_ttl_hours: i64 = env("SOLOOPS_SESSION_TTL_HOURS", "24")
            .parse()
            .context("SOLOOPS_SESSION_TTL_HOURS must be an integer")?;
        let event_poll_ms = env("SOLOOPS_EVENT_POLL_MS", "500")
            .parse()
            .context("SOLOOPS_EVENT_POLL_MS must be an integer")?;
        let worker_poll_ms = env("SOLOOPS_WORKER_POLL_MS", "1000")
            .parse()
            .context("SOLOOPS_WORKER_POLL_MS must be an integer")?;
        let public_ip_poll_seconds = env("SOLOOPS_PUBLIC_IP_POLL_SECONDS", "300")
            .parse()
            .context("SOLOOPS_PUBLIC_IP_POLL_SECONDS must be an integer")?;
        if !(1..=24 * 30).contains(&session_ttl_hours) {
            anyhow::bail!("SOLOOPS_SESSION_TTL_HOURS must be between 1 and 720");
        }
        if !(100..=10_000).contains(&event_poll_ms) {
            anyhow::bail!("SOLOOPS_EVENT_POLL_MS must be between 100 and 10000");
        }
        if !(100..=60_000).contains(&worker_poll_ms) {
            anyhow::bail!("SOLOOPS_WORKER_POLL_MS must be between 100 and 60000");
        }
        if !(60..=86_400).contains(&public_ip_poll_seconds) {
            anyhow::bail!("SOLOOPS_PUBLIC_IP_POLL_SECONDS must be between 60 and 86400");
        }
        let smtp = smtp_config_from_values(
            std::env::var("SOLOOPS_SMTP_HOST").ok(),
            std::env::var("SOLOOPS_SMTP_PORT").ok(),
            std::env::var("SOLOOPS_SMTP_SECURITY").ok(),
            std::env::var("SOLOOPS_SMTP_FROM").ok(),
            std::env::var("SOLOOPS_SMTP_USERNAME").ok(),
            std::env::var("SOLOOPS_SMTP_PASSWORD_REF").ok(),
        )?;
        Ok(Self {
            secure_cookies: environment == "production",
            environment,
            host: env("SOLOOPS_HOST", "127.0.0.1"),
            api_port,
            web_origin: env("SOLOOPS_WEB_ORIGIN", "http://127.0.0.1:5173"),
            database_path: absolute_path(env("SOLOOPS_DATABASE_PATH", "var/db/soloops.db"))?,
            web_dist: absolute_path(env("SOLOOPS_WEB_DIST", "apps/web/build"))?,
            session_ttl_ms: session_ttl_hours * 60 * 60 * 1000,
            event_poll_ms,
            worker_poll_ms,
            log_filter: env("SOLOOPS_LOG", "soloops=info,tower_http=info"),
            metrics_enabled: parse_bool(&env("SOLOOPS_METRICS_ENABLED", "true"))?,
            public_ip_endpoint: env("SOLOOPS_PUBLIC_IP_ENDPOINT", "https://api.ipify.org"),
            public_ip_poll_seconds,
            smtp,
        })
    }

    pub fn test(database_path: PathBuf) -> Self {
        Self {
            environment: "test".into(),
            host: "127.0.0.1".into(),
            api_port: 0,
            web_origin: "http://127.0.0.1:5173".into(),
            database_path,
            web_dist: PathBuf::from("missing-test-web-dist"),
            session_ttl_ms: 24 * 60 * 60 * 1000,
            event_poll_ms: 100,
            worker_poll_ms: 100,
            log_filter: "warn".into(),
            secure_cookies: false,
            metrics_enabled: true,
            public_ip_endpoint: "https://api.ipify.org".into(),
            public_ip_poll_seconds: 300,
            smtp: None,
        }
    }
}

fn smtp_config_from_values(
    host: Option<String>,
    port: Option<String>,
    security: Option<String>,
    from: Option<String>,
    username: Option<String>,
    password_ref: Option<String>,
) -> Result<Option<SmtpConfig>> {
    let values_present = [&host, &port, &security, &from, &username, &password_ref]
        .into_iter()
        .any(|value| value.as_deref().is_some_and(|value| !value.trim().is_empty()));
    if !values_present {
        return Ok(None);
    }
    let host = required_value("SOLOOPS_SMTP_HOST", host)?;
    let from = required_value("SOLOOPS_SMTP_FROM", from)?;
    from.parse::<lettre::message::Mailbox>()
        .context("SOLOOPS_SMTP_FROM must be a valid mailbox")?;
    let security = match security
        .as_deref()
        .unwrap_or("tls")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "tls" => SmtpSecurity::Tls,
        "starttls" => SmtpSecurity::StartTls,
        _ => anyhow::bail!("SOLOOPS_SMTP_SECURITY must be tls or starttls"),
    };
    let port = match port.filter(|value| !value.trim().is_empty()) {
        Some(value) => value
            .parse()
            .context("SOLOOPS_SMTP_PORT must be a valid TCP port")?,
        None if security == SmtpSecurity::Tls => 465,
        None => 587,
    };
    let username = username.filter(|value| !value.trim().is_empty());
    let password_env = password_ref
        .filter(|value| !value.trim().is_empty())
        .map(|value| parse_password_ref(&value))
        .transpose()?;
    if username.is_some() != password_env.is_some() {
        anyhow::bail!("SOLOOPS_SMTP_USERNAME and SOLOOPS_SMTP_PASSWORD_REF must be configured together");
    }
    Ok(Some(SmtpConfig {
        host,
        port,
        security,
        from,
        username,
        password_env,
    }))
}

fn required_value(name: &str, value: Option<String>) -> Result<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} is required when SMTP is configured"))
}

fn parse_password_ref(value: &str) -> Result<String> {
    let Some(name) = value.strip_prefix("env:") else {
        anyhow::bail!("SOLOOPS_SMTP_PASSWORD_REF must use env:<name>");
    };
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !valid_start || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric()) {
        anyhow::bail!("SOLOOPS_SMTP_PASSWORD_REF contains an invalid environment name");
    }
    Ok(name.to_owned())
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn absolute_path(path: String) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()?.join(path))
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("invalid boolean value: {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_defaults_to_implicit_tls_and_port_465() {
        let config = smtp_config_from_values(
            Some("smtp.example.com".into()),
            None,
            None,
            Some("SoloOps <ops@example.com>".into()),
            None,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(config.security, SmtpSecurity::Tls);
        assert_eq!(config.port, 465);
    }

    #[test]
    fn smtp_credentials_are_paired_and_secret_refs_are_typed() {
        assert!(
            smtp_config_from_values(
                Some("smtp.example.com".into()),
                None,
                None,
                Some("ops@example.com".into()),
                Some("user".into()),
                None,
            )
            .is_err()
        );
        assert!(parse_password_ref("secret").is_err());
        assert_eq!(parse_password_ref("env:SMTP_PASSWORD").unwrap(), "SMTP_PASSWORD");
    }
}
