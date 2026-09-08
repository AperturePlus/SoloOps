use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        ConnectInfo, FromRequestParts, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderName, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::random;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_application::{
    ChatCompletionsProvider, EffectiveModelSettings, ModelProvider, ModelRequest, ModelSettingsSnapshot,
    SecretValue, model_settings_snapshot, resolve_model_settings,
};
use soloops_domain::{
    AgentPlan, ApiErrorResponse, ApprovalDecisionRequest, BudgetSnapshot, CreateTaskRequest,
    IpNotificationSettings, LoginRequest, ModelSettings, Owner, RunDetail, RuntimeSnapshot, SessionResponse,
    SmtpSettings, SshAccessReport, TaskSummary, TestIpNotificationResponse, TestModelSettingsResponse,
    TestSmtpDeliveryRequest, TestSmtpDeliveryResponse, ToolCallSummary, UpdateIpNotificationSettingsRequest,
    UpdateModelSettingsRequest, UpdateSmtpSettingsRequest,
};
use soloops_storage::{
    AuditEntry, AuthenticatedOwner, Database, ModelSettingsRecord, SmtpSettingsRecord, StorageError, now_ms,
};
use tokio::sync::Mutex;
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

mod auth;
mod config;
mod error;
mod mail;
mod notifications;
mod password_rotation;
mod ssh_access;
mod telemetry;

pub use auth::SESSION_COOKIE;
pub use config::{AppConfig, PasswordRotationConfig, SmtpConfig, SmtpSecurity};
pub use notifications::NotificationService;
pub use password_rotation::PasswordRotationService;
pub use telemetry::init_telemetry;

use auth::{LoginLimiter, login, logout, require_owner, session};
use error::AppError;
use mail::MailDelivery;
use telemetry::Metrics;

#[derive(Clone)]
struct AppState {
    database: Database,
    config: Arc<AppConfig>,
    metrics: Arc<Metrics>,
    login_limiter: LoginLimiter,
    notifications: NotificationService,
    mail: MailDelivery,
}

pub fn build_router(database: Database, config: AppConfig) -> Router {
    build_router_and_notifications(database, config)
        .expect("notification service construction failed")
        .0
}

pub fn build_router_and_notifications(
    database: Database,
    config: AppConfig,
) -> Result<(Router, NotificationService, PasswordRotationService)> {
    let metrics_state = Arc::new(Metrics::default());
    let mail = MailDelivery::new(database.clone(), config.smtp.clone());
    let notifications =
        NotificationService::new(database.clone(), &config, metrics_state.clone(), mail.clone());
    let password_rotation = PasswordRotationService::new(
        database.clone(),
        &config.password_rotation,
        metrics_state.clone(),
        mail.clone(),
    );
    let state = AppState {
        database,
        config: Arc::new(config),
        metrics: metrics_state,
        login_limiter: LoginLimiter::new(),
        notifications: notifications.clone(),
        mail,
    };
    let mut router = Router::new()
        .route("/healthz", get(health))
        .route("/livez", get(health))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/session", get(session))
        .route("/api/tasks", get(list_tasks).post(create_task))
        .route("/api/tasks/{task_id}", get(get_task))
        .route("/api/runs/{run_id}", get(get_run))
        .route("/api/runs/{run_id}/runtime", get(get_runtime))
        .route("/api/runs/{run_id}/cancel", post(cancel_run))
        .route(
            "/api/runs/{run_id}/tool-calls/{call_id}/decision",
            post(decide_tool_call),
        )
        .route("/api/events", get(events))
        .route(
            "/api/settings/ip-notifications",
            get(get_ip_notification_settings).put(update_ip_notification_settings),
        )
        .route("/api/settings/ip-notifications/test", post(test_ip_notifications))
        .route(
            "/api/settings/smtp",
            get(get_smtp_settings)
                .put(update_smtp_settings)
                .delete(delete_smtp_settings),
        )
        .route("/api/settings/smtp/test", post(test_smtp_delivery))
        .route(
            "/api/settings/model",
            get(get_model_settings)
                .put(update_model_settings)
                .delete(delete_model_settings),
        )
        .route("/api/settings/model/test", post(test_model_settings))
        .route("/api/settings/ssh-access", get(get_ssh_access))
        .layer(middleware::from_fn_with_state(state.clone(), observe_request))
        .layer(middleware::from_fn_with_state(state.clone(), enforce_origin))
        .layer(PropagateRequestIdLayer::new(HeaderName::from_static(
            "x-request-id",
        )))
        .layer(SetRequestIdLayer::new(
            HeaderName::from_static("x-request-id"),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
        .with_state(state.clone());

    if state.config.web_dist.is_dir() {
        let index = state.config.web_dist.join("index.html");
        router = router
            .fallback_service(ServeDir::new(&state.config.web_dist).not_found_service(ServeFile::new(index)));
    }
    Ok((router, notifications, password_rotation))
}

async fn observe_request(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    state.metrics.requests.fetch_add(1, Ordering::Relaxed);
    let response = next.run(request).await;
    if response.status().is_server_error() {
        state.metrics.errors.fetch_add(1, Ordering::Relaxed);
    }
    response
}

async fn enforce_origin(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS)
        && let Some(origin) = request.headers().get(header::ORIGIN)
        && origin.to_str().ok() != Some(state.config.web_origin.as_str())
    {
        return AppError::forbidden("invalid_origin", "Request origin is not allowed").into_response();
    }
    next.run(request).await
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "api" }))
}

async fn ready(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    state.database.ready().await?;
    Ok(Json(json!({ "status": "ready", "service": "api" })))
}

async fn metrics(State(state): State<AppState>) -> Result<Response, AppError> {
    if !state.config.metrics_enabled {
        return Err(AppError::not_found("metrics_disabled", "Metrics are disabled"));
    }
    Ok((
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.render(),
    )
        .into_response())
}

async fn get_ip_notification_settings(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<IpNotificationSettings>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    state
        .notifications
        .settings(&owner.owner.id)
        .await
        .map(Json)
        .map_err(AppError::internal)
}

async fn update_ip_notification_settings(
    State(state): State<AppState>,
    jar: CookieJar,
    payload: Result<Json<UpdateIpNotificationSettingsRequest>, JsonRejection>,
) -> Result<Json<IpNotificationSettings>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    let Json(input) = payload.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let recipients = normalize_recipients(input.recipients)?;
    if input.enabled && recipients.is_empty() {
        return Err(AppError::validation(
            "recipients",
            "at least one recipient is required when notifications are enabled",
        ));
    }
    if input.enabled && !state.notifications.smtp_configured().await {
        return Err(AppError::conflict(
            "notification_transport_unconfigured",
            "SMTP is not configured",
        ));
    }
    state
        .database
        .update_ip_notification_settings(&owner.owner.id, input.enabled, &recipients)
        .await?;
    state.notifications.wake();
    state
        .notifications
        .settings(&owner.owner.id)
        .await
        .map(Json)
        .map_err(AppError::internal)
}

async fn test_ip_notifications(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<(StatusCode, Json<TestIpNotificationResponse>), AppError> {
    let owner = require_owner(&state, &jar).await?;
    if !state.notifications.smtp_configured().await {
        return Err(AppError::conflict(
            "notification_transport_unconfigured",
            "SMTP is not configured",
        ));
    }
    let settings = state
        .database
        .get_ip_notification_settings(&owner.owner.id)
        .await?;
    if settings.recipients.is_empty() {
        return Err(AppError::validation(
            "recipients",
            "save at least one recipient before sending a test email",
        ));
    }
    let response = state
        .notifications
        .test(&owner.owner.id)
        .await
        .map_err(AppError::internal)?;
    let status = if response.failed_recipients.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::BAD_GATEWAY
    };
    Ok((status, Json(response)))
}

async fn get_smtp_settings(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<SmtpSettings>, AppError> {
    require_owner(&state, &jar).await?;
    let snapshot = state.mail.snapshot().await.map_err(AppError::internal)?;
    Ok(Json(smtp_settings_response(snapshot)))
}

async fn update_smtp_settings(
    State(state): State<AppState>,
    jar: CookieJar,
    payload: Result<Json<UpdateSmtpSettingsRequest>, JsonRejection>,
) -> Result<Json<SmtpSettings>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    let Json(input) = payload.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let input = input
        .normalize()
        .map_err(|error| AppError::validation(error.field, error.message))?;
    input
        .from
        .parse::<lettre::message::Mailbox>()
        .map_err(|_| AppError::validation("from", "must be a valid sender mailbox"))?;
    let existing = state.database.get_smtp_settings(&owner.owner.id).await?;
    // Absent keeps the stored credential, an empty string clears it, and a
    // non-empty string replaces it.
    let password = match (&input.password, &existing) {
        (None, Some(row)) => row.password.clone(),
        (None, None) => None,
        (Some(value), _) if value.is_empty() => None,
        (Some(value), _) => Some(value.clone()),
    };
    if input.username.is_some() != password.is_some() {
        return Err(AppError::validation(
            "username",
            "username and password must be configured together",
        ));
    }
    let record = SmtpSettingsRecord {
        owner_id: owner.owner.id.clone(),
        host: input.host,
        port: i64::from(input.port),
        security: input.security,
        from_mailbox: input.from,
        username: input.username,
        password,
        updated_at: now_ms(),
    };
    state.database.upsert_smtp_settings(&record).await?;
    let snapshot = state.mail.snapshot().await.map_err(AppError::internal)?;
    Ok(Json(smtp_settings_response(snapshot)))
}

async fn delete_smtp_settings(State(state): State<AppState>, jar: CookieJar) -> Result<StatusCode, AppError> {
    let owner = require_owner(&state, &jar).await?;
    state.database.delete_smtp_settings(&owner.owner.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_smtp_delivery(
    State(state): State<AppState>,
    jar: CookieJar,
    payload: Result<Json<TestSmtpDeliveryRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<TestSmtpDeliveryResponse>), AppError> {
    let owner = require_owner(&state, &jar).await?;
    let Json(input) = payload.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let input = input
        .normalize()
        .map_err(|error| AppError::validation(error.field, error.message))?;
    if !state.mail.smtp_configured().await {
        return Err(AppError::conflict(
            "notification_transport_unconfigured",
            "SMTP is not configured; save the settings before sending a test email",
        ));
    }
    let result = state
        .mail
        .send_email(
            &input.recipient,
            "[SoloOps] SMTP 测试邮件".into(),
            "这是一封 SoloOps SMTP 配置测试邮件。收到本邮件说明当前 SMTP 配置可以正常投递。\n".into(),
        )
        .await;
    let response = match result {
        Ok(()) => TestSmtpDeliveryResponse {
            delivered: true,
            error: None,
        },
        Err(error) => TestSmtpDeliveryResponse {
            delivered: false,
            error: Some(error.to_string()),
        },
    };
    state
        .database
        .write_audit(AuditEntry {
            actor_type: "owner",
            actor_id: Some(owner.owner.id.clone()),
            action: "settings.smtp.test",
            object_type: Some("smtp_settings"),
            object_id: Some(owner.owner.id.clone()),
            outcome: if response.delivered { "success" } else { "failure" },
            context: json!({ "recipient": input.recipient }),
        })
        .await?;
    let status = if response.delivered {
        StatusCode::OK
    } else {
        StatusCode::BAD_GATEWAY
    };
    Ok((status, Json(response)))
}

fn smtp_settings_response(snapshot: mail::SmtpSnapshot) -> SmtpSettings {
    SmtpSettings {
        configured: snapshot.configured,
        source: snapshot.source.map(str::to_owned),
        host: snapshot.host,
        port: snapshot.port,
        security: snapshot.security.map(|security| {
            match security {
                SmtpSecurity::Tls => "tls",
                SmtpSecurity::StartTls => "starttls",
            }
            .to_owned()
        }),
        from: snapshot.from,
        username: snapshot.username,
        password_configured: snapshot.password_configured,
    }
}

async fn get_model_settings(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<ModelSettings>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    let snapshot = model_settings_snapshot(&state.database, &owner.owner.id, &state.config.model_env)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(model_settings_response(snapshot)))
}

async fn update_model_settings(
    State(state): State<AppState>,
    jar: CookieJar,
    payload: Result<Json<UpdateModelSettingsRequest>, JsonRejection>,
) -> Result<Json<ModelSettings>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    let Json(input) = payload.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let input = input
        .normalize()
        .map_err(|error| AppError::validation(error.field, error.message))?;
    let existing = state.database.get_model_settings(&owner.owner.id).await?;
    // Absent keeps the stored credential, an empty string clears it (for
    // endpoints that authenticate nothing), and a non-empty string replaces it.
    let api_key = match (&input.api_key, &existing) {
        (None, Some(row)) => row.api_key.clone(),
        (None, None) => None,
        (Some(value), _) if value.is_empty() => None,
        (Some(value), _) => Some(value.clone()),
    };
    let record = ModelSettingsRecord {
        owner_id: owner.owner.id.clone(),
        base_url: input.base_url,
        model_name: input.model_name,
        api_key,
        updated_at: now_ms(),
    };
    state.database.upsert_model_settings(&record).await?;
    let snapshot = model_settings_snapshot(&state.database, &owner.owner.id, &state.config.model_env)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(model_settings_response(snapshot)))
}

async fn delete_model_settings(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<StatusCode, AppError> {
    let owner = require_owner(&state, &jar).await?;
    state.database.delete_model_settings(&owner.owner.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_model_settings(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<(StatusCode, Json<TestModelSettingsResponse>), AppError> {
    let owner = require_owner(&state, &jar).await?;
    let resolved = resolve_model_settings(&state.database, &owner.owner.id, &state.config.model_env)
        .await
        .map_err(AppError::internal)?;
    let Some(settings) = resolved else {
        return Err(AppError::conflict(
            "model_settings_unconfigured",
            "Model API is not configured; save the settings before sending a test request",
        ));
    };
    let response = probe_model_endpoint(&settings).await;
    state
        .database
        .write_audit(AuditEntry {
            actor_type: "owner",
            actor_id: Some(owner.owner.id.clone()),
            action: "settings.model.test",
            object_type: Some("model_settings"),
            object_id: Some(owner.owner.id.clone()),
            outcome: if response.responded { "success" } else { "failure" },
            // Never include the API key; the endpoint coordinates suffice.
            context: json!({
                "baseUrl": settings.base_url,
                "modelName": settings.model_name,
            }),
        })
        .await?;
    let status = if response.responded {
        StatusCode::OK
    } else {
        StatusCode::BAD_GATEWAY
    };
    Ok((status, Json(response)))
}

/// Sends a minimal chat-completion request through the effective settings so
/// the Owner can verify the endpoint, model and credential end to end.
async fn probe_model_endpoint(settings: &EffectiveModelSettings) -> TestModelSettingsResponse {
    let provider = match ChatCompletionsProvider::new(
        &settings.base_url,
        settings.model_name.clone(),
        SecretValue::new(settings.api_key.expose_secret().to_owned()),
    ) {
        Ok(provider) => provider,
        Err(error) => {
            return TestModelSettingsResponse {
                responded: false,
                error: Some(error.to_string()),
            };
        }
    };
    let request = ModelRequest {
        logical_request_key: format!("settings-model-test:{}", now_ms()),
        attempt_key: uuid::Uuid::new_v4().to_string(),
        goal: "Connection test. Reply with the single word: pong.".to_owned(),
        plan: AgentPlan::default(),
        journal: Vec::new(),
        evidence: Vec::new(),
        tools: Vec::new(),
        remaining_budget: BudgetSnapshot {
            max_model_turns: 1,
            max_tool_calls: 0,
            max_input_tokens: 2048,
            max_output_tokens: 32,
            max_duration_ms: 30_000,
            max_tool_duration_ms: 1_000,
            max_tool_output_bytes: 2048,
            max_workspace_bytes: 2048,
        },
    };
    match provider.complete(request).await {
        Ok(_) => TestModelSettingsResponse {
            responded: true,
            error: None,
        },
        Err(error) => TestModelSettingsResponse {
            responded: false,
            error: Some(error.to_string()),
        },
    }
}

fn model_settings_response(snapshot: ModelSettingsSnapshot) -> ModelSettings {
    ModelSettings {
        configured: snapshot.configured,
        source: snapshot.source.map(str::to_owned),
        base_url: snapshot.base_url,
        model_name: snapshot.model_name,
        api_key_configured: snapshot.api_key_configured,
    }
}

async fn get_ssh_access(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<SshAccessReport>, AppError> {
    require_owner(&state, &jar).await?;
    // Filesystem scans stay off the async runtime.
    let report = tokio::task::spawn_blocking(ssh_access::scan_ssh_access)
        .await
        .map_err(|error| AppError::internal(format!("ssh access scan failed: {error}")))?;
    Ok(Json(report))
}

fn normalize_recipients(recipients: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for recipient in recipients {
        let recipient = recipient.trim();
        if recipient.is_empty() || recipient.len() > 254 || recipient.parse::<lettre::Address>().is_err() {
            return Err(AppError::validation(
                "recipients",
                "each recipient must be a valid email address",
            ));
        }
        let recipient = recipient.to_ascii_lowercase();
        if seen.insert(recipient.clone()) {
            normalized.push(recipient);
        }
    }
    if normalized.len() > 20 {
        return Err(AppError::validation(
            "recipients",
            "at most 20 recipients are allowed",
        ));
    }
    normalized.sort();
    Ok(normalized)
}

async fn create_task(
    State(state): State<AppState>,
    jar: CookieJar,
    payload: Result<Json<CreateTaskRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<TaskSummary>), AppError> {
    let owner = require_owner(&state, &jar).await?;
    let Json(input) = payload.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let input = input
        .normalize()
        .map_err(|error| AppError::validation(error.field, error.message))?;
    let task = state
        .database
        .create_task_with_run(&owner.owner.id, &input)
        .await?;
    Ok((StatusCode::CREATED, Json(task)))
}

#[derive(Serialize)]
struct Items<T> {
    items: Vec<T>,
}

async fn list_tasks(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Items<TaskSummary>>, AppError> {
    require_owner(&state, &jar).await?;
    Ok(Json(Items {
        items: state.database.list_tasks().await?,
    }))
}

async fn get_task(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(task_id): Path<String>,
) -> Result<Json<TaskSummary>, AppError> {
    require_owner(&state, &jar).await?;
    state
        .database
        .get_task(&task_id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("not_found", "Task was not found"))
}

async fn get_run(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(run_id): Path<String>,
) -> Result<Json<RunDetail>, AppError> {
    require_owner(&state, &jar).await?;
    state
        .database
        .get_run(&run_id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("not_found", "Run was not found"))
}

async fn get_runtime(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(run_id): Path<String>,
) -> Result<Json<RuntimeSnapshot>, AppError> {
    require_owner(&state, &jar).await?;
    state
        .database
        .runtime_snapshot(&run_id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("runtime_not_found", "Runtime has not started for this Run"))
}

async fn cancel_run(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(run_id): Path<String>,
) -> Result<Json<RunDetail>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    Ok(Json(state.database.cancel_run(&run_id, &owner.owner.id).await?))
}

async fn decide_tool_call(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((run_id, call_id)): Path<(String, String)>,
    payload: Result<Json<ApprovalDecisionRequest>, JsonRejection>,
) -> Result<Json<ToolCallSummary>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    let Json(input) = payload.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let input = input
        .normalize()
        .map_err(|error| AppError::validation(error.field, error.message))?;
    Ok(Json(
        state
            .database
            .decide_tool_call(
                &run_id,
                &call_id,
                &owner.owner.id,
                input.decision,
                input.reason.as_deref(),
            )
            .await?,
    ))
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EventQuery {
    #[serde(default)]
    after: i64,
    run_id: Option<String>,
}

async fn events(
    State(state): State<AppState>,
    jar: CookieJar,
    query: Result<Query<EventQuery>, QueryRejection>,
    request: Request<Body>,
) -> Result<Response, AppError> {
    let Query(query) = query.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let owner = require_owner(&state, &jar).await?;
    let after = query.after.max(0);
    if request
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
    {
        let (mut parts, _) = request.into_parts();
        let websocket = WebSocketUpgrade::from_request_parts(&mut parts, &state)
            .await
            .map_err(|_| {
                AppError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_upgrade",
                    "Invalid WebSocket upgrade",
                )
            })?;
        let run_id = query.run_id.clone();
        return Ok(websocket
            .on_upgrade(move |socket| stream_events(socket, state, after, run_id, owner.owner.id))
            .into_response());
    }
    let items = state
        .database
        .list_events(after, query.run_id.as_deref(), 200)
        .await?;
    Ok(Json(Items { items }).into_response())
}

async fn stream_events(
    mut socket: WebSocket,
    state: AppState,
    mut after: i64,
    run_id: Option<String>,
    owner_id: String,
) {
    state.metrics.websocket_clients.fetch_add(1, Ordering::Relaxed);
    info!(owner_id, ?run_id, after, "websocket event stream opened");
    let mut interval = tokio::time::interval(Duration::from_millis(state.config.event_poll_ms));
    loop {
        match state.database.list_events(after, run_id.as_deref(), 200).await {
            Ok(events) => {
                for event in events {
                    let payload = match serde_json::to_string(&event) {
                        Ok(payload) => payload,
                        Err(error) => {
                            error!(%error, "failed to serialize event");
                            break;
                        }
                    };
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        state.metrics.websocket_clients.fetch_sub(1, Ordering::Relaxed);
                        return;
                    }
                    after = event.sequence;
                }
            }
            Err(error) => {
                error!(%error, "failed to read websocket events");
                if matches!(error, StorageError::InvalidEventPayload { .. }) {
                    let _ = socket
                        .send(Message::Close(Some(CloseFrame {
                            code: 4002,
                            reason: "event_stream_corrupt".into(),
                        })))
                        .await;
                }
                break;
            }
        }
        tokio::select! {
            _ = interval.tick() => {}
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
    state.metrics.websocket_clients.fetch_sub(1, Ordering::Relaxed);
    info!(owner_id, ?run_id, after, "websocket event stream closed");
}

#[cfg(test)]
mod tests;
