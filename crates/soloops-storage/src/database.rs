use std::{
    path::Path,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_domain::{CreateTaskRequest, EventEnvelope, EventType, Owner, RunDetail, RunStatus, TaskSummary};
use sqlx::{
    ConnectOptions, Row, Sqlite, SqliteConnection, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous},
};
use thiserror::Error;
use uuid::Uuid;

const BASELINE_SQL: &str = include_str!("../migrations/0001_phase_zero.sql");
const AGENT_RUNTIME_SQL: &str = include_str!("../migrations/0002_agent_runtime.sql");
const WORKSPACE_WRITE_RECOVERY_SQL: &str = include_str!("../migrations/0003_workspace_write_recovery.sql");
const IP_NOTIFICATIONS_SQL: &str = include_str!("../migrations/0004_ip_notifications.sql");
const MANAGED_DEPLOYMENTS_SQL: &str = include_str!("../migrations/0005_managed_deployments.sql");
const MANAGED_DEPLOYMENT_LEASES_SQL: &str = include_str!("../migrations/0006_managed_deployment_leases.sql");
const REQUIRED_TABLES: &[&str] = &[
    "users",
    "sessions",
    "tasks",
    "runs",
    "events",
    "audit_logs",
    "agent_sessions",
    "agent_items",
    "model_attempts",
    "tool_calls",
    "tool_approvals",
    "evidence",
    "ip_notification_settings",
    "ip_notification_recipients",
    "managed_deployments",
    "managed_deployment_revisions",
    "managed_deployment_operations",
];
const REQUIRED_COLUMNS: &[(&str, &[&str])] = &[
    ("users", &["id", "username", "password_hash", "created_at"]),
    (
        "sessions",
        &[
            "id",
            "token_hash",
            "user_id",
            "expires_at",
            "revoked_at",
            "created_at",
            "last_seen_at",
        ],
    ),
    (
        "tasks",
        &["id", "title", "goal", "created_by", "created_at", "updated_at"],
    ),
    (
        "runs",
        &[
            "id",
            "task_id",
            "status",
            "status_reason",
            "lease_owner",
            "lease_expires_at",
            "created_at",
            "started_at",
            "finished_at",
        ],
    ),
    (
        "events",
        &["sequence", "id", "run_id", "type", "payload", "created_at"],
    ),
    (
        "audit_logs",
        &[
            "sequence",
            "id",
            "actor_type",
            "actor_id",
            "action",
            "object_type",
            "object_id",
            "outcome",
            "context",
            "created_at",
        ],
    ),
    (
        "agent_sessions",
        &[
            "run_id",
            "provider",
            "model",
            "prompt_version",
            "workspace_path",
            "artifact_path",
            "checkpoint",
            "plan_json",
            "budget_json",
            "usage_json",
            "workspace_revision",
            "retry_at",
            "consecutive_no_progress",
            "consecutive_protocol_errors",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "agent_items",
        &[
            "id",
            "run_id",
            "sequence",
            "kind",
            "payload",
            "provider_request_id",
            "created_at",
        ],
    ),
    (
        "model_attempts",
        &[
            "id",
            "run_id",
            "request_key",
            "attempt",
            "status",
            "provider_request_id",
            "error_category",
            "started_at",
            "completed_at",
        ],
    ),
    (
        "tool_calls",
        &[
            "call_id",
            "run_id",
            "item_id",
            "ordinal",
            "name",
            "arguments_json",
            "arguments_sha256",
            "approval_preview_json",
            "risk",
            "policy_decision",
            "status",
            "idempotency_key",
            "result_summary",
            "result_json",
            "recovery_json",
            "error_category",
            "workspace_revision_before",
            "workspace_revision_after",
            "created_at",
            "started_at",
            "completed_at",
        ],
    ),
    (
        "tool_approvals",
        &[
            "call_id",
            "run_id",
            "owner_id",
            "arguments_sha256",
            "decision",
            "reason",
            "created_at",
        ],
    ),
    (
        "evidence",
        &[
            "id",
            "run_id",
            "tool_call_id",
            "kind",
            "summary",
            "artifact_ref",
            "content_sha256",
            "workspace_revision",
            "created_at",
        ],
    ),
    (
        "ip_notification_settings",
        &[
            "owner_id",
            "enabled",
            "current_ipv4",
            "last_checked_at",
            "last_changed_at",
            "updated_at",
        ],
    ),
    (
        "ip_notification_recipients",
        &[
            "owner_id",
            "email",
            "last_notified_ipv4",
            "last_notified_at",
            "last_attempt_at",
            "last_error",
            "created_at",
        ],
    ),
    (
        "managed_deployments",
        &["project_id", "current_revision_id", "updated_at"],
    ),
    (
        "managed_deployment_revisions",
        &[
            "id",
            "project_id",
            "run_id",
            "plan_call_id",
            "previous_revision_id",
            "proposal_sha256",
            "compose_sha256",
            "caddy_sha256",
            "source_json",
            "preview_json",
            "bundle_path",
            "status",
            "health_json",
            "created_at",
            "activated_at",
            "finished_at",
        ],
    ),
    (
        "managed_deployment_operations",
        &[
            "call_id",
            "run_id",
            "project_id",
            "action",
            "revision_id",
            "previous_revision_id",
            "phase",
            "result_json",
            "error_category",
            "lease_token",
            "lease_expires_at",
            "recovery_attempts",
            "last_recovery_error",
            "created_at",
            "updated_at",
            "finished_at",
        ],
    ),
];

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Clone)]
pub struct AuthenticatedOwner {
    pub owner: Owner,
    pub session_id: String,
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub actor_type: &'static str,
    pub actor_id: Option<String>,
    pub action: &'static str,
    pub object_type: Option<&'static str>,
    pub object_id: Option<String>,
    pub outcome: &'static str,
    pub context: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpNotificationRecord {
    pub owner_id: String,
    pub enabled: bool,
    pub current_ipv4: Option<String>,
    pub last_checked_at: Option<i64>,
    pub last_changed_at: Option<i64>,
    pub recipients: Vec<IpNotificationRecipientRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpNotificationRecipientRecord {
    pub email: String,
    pub last_notified_ipv4: Option<String>,
    pub last_notified_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid run status stored in database: {0}")]
    InvalidRunStatus(String),
    #[error("invalid event type stored in database: {0}")]
    InvalidEventType(String),
    #[error("invalid payload for event {event_id} at sequence {sequence}: {source}")]
    InvalidEventPayload {
        event_id: String,
        sequence: i64,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid run transition: {from} -> {to}")]
    InvalidTransition { from: RunStatus, to: RunStatus },
    #[error("run was not found: {0}")]
    RunNotFound(String),
    #[error("tool call state changed before update: {run_id}:{call_id}")]
    ToolCallStateConflict { run_id: String, call_id: String },
    #[error("managed deployment state conflict: {0}")]
    ManagedDeploymentConflict(String),
    #[error("database schema is incomplete; missing table {0}")]
    MissingTable(String),
    #[error("database schema is incomplete; missing column {table}.{column}")]
    MissingColumn { table: String, column: String },
    #[error("owner already exists")]
    OwnerAlreadyExists,
    #[error("owner username is required")]
    InvalidOwnerUsername,
    #[error("invalid runtime JSON in {field}: {source}")]
    InvalidRuntimeJson {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

impl Database {
    pub(crate) async fn begin_write(&self) -> Result<Transaction<'static, Sqlite>, StorageError> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let in_memory = path == Path::new(":memory:");
        if !in_memory
            && let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.map_err(sqlx::Error::Io)?;
        }

        let mut options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .synchronous(SqliteSynchronous::Normal);
        if !in_memory {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }
        options = options.disable_statement_logging();

        let pool = SqlitePoolOptions::new()
            .max_connections(if in_memory { 1 } else { 8 })
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

mod auth;
mod notifications;
mod schema;
mod tasks;

pub(crate) use tasks::{insert_audit_on_connection, insert_event, transition_run_on_connection};

#[cfg(test)]
mod tests;
