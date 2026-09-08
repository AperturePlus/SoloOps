use anyhow::{Context, Result};
use std::sync::Arc;

use soloops_application::{
    ChatCompletionsProvider, EffectiveModelSettings, EnvModelConfig, RuntimeConfig, RuntimeEngine,
    SecretValue, resolve_env_model_settings, resolve_model_settings,
};
use soloops_server::{AppConfig, init_telemetry};
use soloops_storage::Database;
use tracing::{error, info, warn};
use uuid::Uuid;

const SESSION_CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const FAILURE_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);

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
    // The effective model endpoint (database settings first, environment
    // fallback) is re-resolved between runs so WebUI edits apply without a
    // restart. The engine is rebuilt only when its settings signature changes.
    let env_model = runtime_config.model_env();
    let mut engine: Option<RuntimeEngine> = None;
    let mut engine_signature: Option<String> = None;
    let mut next_session_cleanup = tokio::time::Instant::now();
    info!(worker_id, poll_ms = config.worker_poll_ms, "worker started");

    loop {
        let effective = resolve_effective(&database, &env_model).await;
        let outcome = match effective {
            Ok(Some(settings)) => {
                run_cycle(
                    &database,
                    &runtime_config,
                    &settings,
                    &mut engine,
                    &mut engine_signature,
                    &worker_id,
                )
                .await
            }
            Ok(None) => {
                warn!(
                    worker_id,
                    "model API settings are unconfigured; configure them in the WebUI \
                     (Settings → Model API) or via SOLOOPS_MODEL_* variables"
                );
                Outcome::Wait
            }
            Err(resolution_error) => {
                error!(worker_id, error = %resolution_error, "resolving effective model settings failed");
                tokio::time::sleep(FAILURE_BACKOFF).await;
                Outcome::Wait
            }
        };
        match outcome {
            Outcome::Immediate => {}
            Outcome::Wait => {
                if idle_wait(
                    &database,
                    &worker_id,
                    config.worker_poll_ms,
                    &mut next_session_cleanup,
                )
                .await
                {
                    break;
                }
            }
        }
    }
    info!(worker_id, "worker stopped");
    database.close().await;
    Ok(())
}

enum Outcome {
    /// A cycle ran, or the failure backoff already elapsed; poll again
    /// immediately without the idle wait.
    Immediate,
    /// Nothing to do; wait for the poll tick (cleanup and Ctrl+C stay live).
    Wait,
}

/// Resolves the effective model settings for the bootstrap owner. The worker
/// has no session context, so it follows the same first-owner rule as mail
/// delivery.
async fn resolve_effective(
    database: &Database,
    env_model: &EnvModelConfig,
) -> Result<Option<EffectiveModelSettings>, soloops_storage::StorageError> {
    match database.find_first_owner().await? {
        Some(owner) => resolve_model_settings(database, &owner.id, env_model).await,
        None => Ok(resolve_env_model_settings(env_model)),
    }
}

async fn run_cycle(
    database: &Database,
    runtime_config: &RuntimeConfig,
    settings: &EffectiveModelSettings,
    engine: &mut Option<RuntimeEngine>,
    engine_signature: &mut Option<String>,
    worker_id: &str,
) -> Outcome {
    let signature = settings.signature();
    if engine_signature.as_deref() != Some(signature.as_str()) {
        let provider = match ChatCompletionsProvider::new(
            &settings.base_url,
            settings.model_name.clone(),
            SecretValue::new(settings.api_key.expose_secret().to_owned()),
        ) {
            Ok(provider) => Arc::new(provider.with_prompt_cache_key(runtime_config.prompt_cache_key_enabled)),
            Err(error) => {
                error!(worker_id, %error, "building the model provider failed");
                tokio::time::sleep(FAILURE_BACKOFF).await;
                return Outcome::Wait;
            }
        };
        let mut config = runtime_config.clone();
        config.model_base_url = settings.base_url.clone();
        config.model_name = settings.model_name.clone();
        let runtime_engine = match RuntimeEngine::new(database.clone(), provider, config) {
            Ok(runtime_engine) => runtime_engine,
            Err(error) => {
                error!(worker_id, %error, "building the runtime engine failed");
                tokio::time::sleep(FAILURE_BACKOFF).await;
                return Outcome::Wait;
            }
        };
        *engine = Some(runtime_engine);
        *engine_signature = Some(signature);
        info!(
            worker_id,
            source = settings.source,
            base_url = settings.base_url,
            model = settings.model_name,
            "model provider rebuilt from the effective settings"
        );
    }
    let engine = engine.as_ref().expect("engine was just built");
    match engine.run_once(worker_id).await {
        Ok(Some(run_id)) => {
            info!(worker_id, run_id, "agent runtime cycle completed");
            Outcome::Immediate
        }
        Ok(None) => Outcome::Wait,
        Err(error) => {
            error!(worker_id, %error, "worker cycle failed");
            tokio::time::sleep(FAILURE_BACKOFF).await;
            Outcome::Immediate
        }
    }
}

/// Waits for the next poll tick while keeping session cleanup and Ctrl+C
/// responsive. Returns `true` when the worker should stop.
async fn idle_wait(
    database: &Database,
    worker_id: &str,
    poll_ms: u64,
    next_session_cleanup: &mut tokio::time::Instant,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(std::time::Duration::from_millis(poll_ms)) => false,
        _ = tokio::time::sleep_until(*next_session_cleanup) => {
            match database.cleanup_expired_sessions().await {
                Ok(deleted) if deleted > 0 => info!(worker_id, deleted, "expired or revoked sessions cleaned up"),
                Ok(_) => {}
                Err(error) => error!(worker_id, %error, "session cleanup failed"),
            }
            *next_session_cleanup = tokio::time::Instant::now() + SESSION_CLEANUP_INTERVAL;
            false
        }
        _ = tokio::signal::ctrl_c() => true,
    }
}
