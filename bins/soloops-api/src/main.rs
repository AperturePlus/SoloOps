use std::net::SocketAddr;

use anyhow::{Context, Result};
use soloops_server::{AppConfig, build_router_and_notifications, init_telemetry};
use soloops_storage::Database;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::load()?;
    init_telemetry("api", &config.log_filter)?;
    let database = Database::connect(&config.database_path).await?;
    database
        .verify_schema()
        .await
        .context("database is not migrated; run `cargo run -p soloopsctl -- migrate`")?;
    let address: SocketAddr = format!("{}:{}", config.host, config.api_port)
        .parse()
        .context("invalid SOLOOPS_HOST or SOLOOPS_API_PORT")?;
    let (router, notifications, password_rotation) =
        build_router_and_notifications(database.clone(), config)?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    info!(%address, "SoloOps API listening");
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let notification_task = tokio::spawn(notifications.run(shutdown_rx.clone()));
    let rotation_task = tokio::spawn(password_rotation.run(shutdown_rx));
    let shutdown_for_server = shutdown_tx.clone();
    let server_result = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        let _ = shutdown_for_server.send(true);
    })
    .await;
    let _ = shutdown_tx.send(true);
    notification_task
        .await
        .context("notification monitor task failed")?;
    rotation_task.await.context("password rotation task failed")?;
    server_result?;
    database.close().await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
