//! Shared fixtures for the database test suite.

use super::super::*;
use crate::{NewToolCall, PersistModelResponse, RuntimeSessionConfig};
use serde_json::json;
use soloops_domain::{BudgetSnapshot, PolicyDecision, ToolRisk, UsageSnapshot};

pub async fn database() -> Database {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    database
}
pub async fn initialized_runtime(database: &Database) -> (Owner, TaskSummary) {
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Runtime".into(),
                goal: "Exercise runtime persistence".into(),
            },
        )
        .await
        .unwrap();
    database
        .claim_next_run("worker-a", 30_000)
        .await
        .unwrap()
        .unwrap();
    database
        .initialize_runtime(
            &task.latest_run_id,
            &RuntimeSessionConfig {
                provider: "test".into(),
                model: "test".into(),
                prompt_version: "test".into(),
                workspace_path: "workspace".into(),
                artifact_path: "artifacts".into(),
                budget: BudgetSnapshot::default(),
            },
        )
        .await
        .unwrap();
    (owner, task)
}
pub async fn insert_read_only_tool_call(database: &Database, run_id: &str, call_id: &str) {
    let attempt_id = database
        .prepare_model_attempt(run_id, &format!("{run_id}:turn:1"), 1)
        .await
        .unwrap();
    database
        .persist_model_response(
            run_id,
            PersistModelResponse {
                attempt_id: &attempt_id,
                provider_request_id: None,
                items: &[],
                calls: &[NewToolCall {
                    call_id: call_id.to_owned(),
                    ordinal: 0,
                    name: "workspace.search".into(),
                    arguments: json!({"path": ".", "query": "needle"}),
                    risk: ToolRisk::ReadOnly,
                    policy: PolicyDecision::Allow,
                }],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
}
