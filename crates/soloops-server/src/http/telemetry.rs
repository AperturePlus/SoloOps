use super::*;

pub fn init_telemetry(service: &str, filter: &str) -> Result<()> {
    let filter = EnvFilter::try_new(filter)?;
    if std::env::var("SOLOOPS_LOG_FORMAT").as_deref() == Ok("pretty") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .try_init()
            .map_err(|error| anyhow::anyhow!("failed to initialize telemetry: {error}"))?;
    } else {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_target(true)
            .with_current_span(true)
            .with_span_list(true)
            .try_init()
            .map_err(|error| anyhow::anyhow!("failed to initialize telemetry: {error}"))?;
    }
    info!(service, "telemetry initialized");
    Ok(())
}

#[derive(Default)]
pub(super) struct Metrics {
    pub(super) requests: AtomicU64,
    pub(super) errors: AtomicU64,
    pub(super) auth_failures: AtomicU64,
    pub(super) websocket_clients: AtomicU64,
    pub(super) public_ip_checks: AtomicU64,
    pub(super) public_ip_check_failures: AtomicU64,
    pub(super) ip_notification_emails_sent: AtomicU64,
    pub(super) ip_notification_email_failures: AtomicU64,
}

impl Metrics {
    pub(super) fn render(&self) -> String {
        format!(
            concat!(
                "# TYPE soloops_http_requests_total counter\n",
                "soloops_http_requests_total {}\n",
                "# TYPE soloops_http_errors_total counter\n",
                "soloops_http_errors_total {}\n",
                "# TYPE soloops_auth_failures_total counter\n",
                "soloops_auth_failures_total {}\n",
                "# TYPE soloops_websocket_clients gauge\n",
                "soloops_websocket_clients {}\n",
                "# TYPE soloops_public_ip_checks_total counter\n",
                "soloops_public_ip_checks_total {}\n",
                "# TYPE soloops_public_ip_check_failures_total counter\n",
                "soloops_public_ip_check_failures_total {}\n",
                "# TYPE soloops_ip_notification_emails_sent_total counter\n",
                "soloops_ip_notification_emails_sent_total {}\n",
                "# TYPE soloops_ip_notification_email_failures_total counter\n",
                "soloops_ip_notification_email_failures_total {}\n"
            ),
            self.requests.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
            self.auth_failures.load(Ordering::Relaxed),
            self.websocket_clients.load(Ordering::Relaxed),
            self.public_ip_checks.load(Ordering::Relaxed),
            self.public_ip_check_failures.load(Ordering::Relaxed),
            self.ip_notification_emails_sent.load(Ordering::Relaxed),
            self.ip_notification_email_failures.load(Ordering::Relaxed),
        )
    }
}
