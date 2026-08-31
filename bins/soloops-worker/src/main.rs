use anyhow::{Context, Result};
use std::sync::Arc;

use soloops_application::{ChatCompletionsProvider, EnvironmentSecretResolver, RuntimeConfig, RuntimeEngine};
use soloops_server::{AppConfig, init_telemetry};
use soloops_storage::Database;
use tracing::{error, info};
use uuid::Uuid;

const SESSION_CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::load()?;
    let runtime_config = RuntimeConfig::load()?;
    init_telemetry("worker", &config.log_filter)?;
    let database = Database::connect(&config.database_path).await?;
    database
        .verify_schema()
        .await
        .context("database is not migrated; run `cargo run -p soloopsctl -- migrate`")?;
    let worker_id = format!("worker-{}", Uuid::new_v4());
    let provider = Arc::new(
        ChatCompletionsProvider::new(
            &runtime_config.model_base_url,
            runtime_config.model_name.clone(),
            runtime_config.api_key(&EnvironmentSecretResolver)?,
        )
        .map_err(|error| anyhow::anyhow!(error))?
        .with_prompt_cache_key(runtime_config.prompt_cache_key_enabled),
    );
    let engine = RuntimeEngine::new(database.clone(), provider, runtime_config)?;
    let mut next_session_cleanup = tokio::time::Instant::now();
    info!(worker_id, poll_ms = config.worker_poll_ms, "worker started");

    loop {
        match engine.run_once(&worker_id).await {
            Ok(Some(run_id)) => info!(worker_id, run_id, "agent runtime cycle completed"),
            Ok(None) => {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(config.worker_poll_ms)) => {}
                    _ = tokio::time::sleep_until(next_session_cleanup) => {
                        match database.cleanup_expired_sessions().await {
                            Ok(deleted) if deleted > 0 => info!(deleted, "expired or revoked sessions cleaned up"),
                            Ok(_) => {}
                            Err(error) => error!(%error, "session cleanup failed"),
                        }
                        next_session_cleanup = tokio::time::Instant::now() + SESSION_CLEANUP_INTERVAL;
                    }
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
            Err(error) => {
                error!(worker_id, %error, "worker cycle failed");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
    info!(worker_id, "worker stopped");
    database.close().await;
    Ok(())
}
