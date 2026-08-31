//! Approval + workspace/process/sandbox execution flow tests.

use super::providers::{FakeHostExecutor, ProcessWorkflowProvider, SandboxWorkflowProvider, WorkflowProvider};
use crate::{RuntimeConfig, RuntimeEngine, SecretRef};
use soloops_domain::{BudgetSnapshot, CreateTaskRequest, RunStatus, ToolCallStatus};
use soloops_storage::Database;
use std::sync::{Arc, atomic::AtomicUsize};

#[tokio::test]
async fn approval_workspace_evidence_and_finish_form_a_persisted_success_path() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Runtime".into(),
                goal: "Create demo.txt".into(),
            },
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        model_base_url: "http://unused".into(),
        model_name: "fake".into(),
        model_api_key_ref: SecretRef::Env("UNUSED".into()),
        prompt_cache_key_enabled: false,
        workspace_root: temp.path().join("workspaces"),
        artifact_root: temp.path().join("artifacts"),
        hostd_socket: None,
        sandbox_enabled: false,
        managed_deploy_enabled: false,
        budget: BudgetSnapshot::default(),
        lease_ms: 30_000,
        lease_renew_ms: 10_000,
    };
    let engine = RuntimeEngine::new(
        database.clone(),
        Arc::new(WorkflowProvider {
            turn: AtomicUsize::new(0),
        }),
        config,
    )
    .unwrap();
    engine.run_once("worker-a").await.unwrap();
    let waiting = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    let call = waiting
        .tool_calls
        .iter()
        .find(|call| call.name == "workspace.create")
        .unwrap();
    assert_eq!(call.status, ToolCallStatus::WaitingForApproval);
    assert_eq!(
        database
            .runtime_execution_state(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .consecutive_no_progress,
        0
    );
    database
        .decide_tool_call(
            &task.latest_run_id,
            &call.call_id,
            &owner.id,
            soloops_domain::ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap();
    engine.run_once("worker-b").await.unwrap();
    let run = database.get_run(&task.latest_run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Succeeded);
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(snapshot.report.is_some());
    assert!(snapshot.evidence.iter().any(|item| item.kind == "file_snapshot"));
    assert_eq!(
        tokio::fs::read_to_string(
            temp.path()
                .join("workspaces")
                .join(&task.latest_run_id)
                .join("demo.txt")
        )
        .await
        .unwrap(),
        "hello"
    );
}

#[tokio::test]
async fn approved_process_exec_uses_the_injected_host_executor() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Host executor".into(),
                goal: "Exercise the hostd boundary".into(),
            },
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        model_base_url: "http://unused".into(),
        model_name: "fake".into(),
        model_api_key_ref: SecretRef::Env("UNUSED".into()),
        prompt_cache_key_enabled: false,
        workspace_root: temp.path().join("workspaces"),
        artifact_root: temp.path().join("artifacts"),
        hostd_socket: None,
        sandbox_enabled: false,
        managed_deploy_enabled: false,
        budget: BudgetSnapshot::default(),
        lease_ms: 30_000,
        lease_renew_ms: 10_000,
    };
    let executor = Arc::new(FakeHostExecutor::default());
    let engine = RuntimeEngine::new_with_host_executor(
        database.clone(),
        Arc::new(ProcessWorkflowProvider {
            turn: AtomicUsize::new(0),
        }),
        config,
        Some(executor.clone()),
    )
    .unwrap();
    engine.run_once("worker-host-a").await.unwrap();
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    let call = snapshot
        .tool_calls
        .iter()
        .find(|call| call.name == "process.exec")
        .unwrap();
    assert_eq!(call.status, ToolCallStatus::WaitingForApproval);
    database
        .decide_tool_call(
            &task.latest_run_id,
            &call.call_id,
            &owner.id,
            soloops_domain::ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap();
    engine.run_once("worker-host-b").await.unwrap();
    assert_eq!(executor.calls.lock().unwrap().len(), 1);
    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Succeeded
    );
}

#[tokio::test]
async fn sandbox_exec_is_registered_only_when_enabled_and_uses_exact_approval() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Sandbox executor".into(),
                goal: "Exercise the Docker sandbox boundary".into(),
            },
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        model_base_url: "http://unused".into(),
        model_name: "fake".into(),
        model_api_key_ref: SecretRef::Env("UNUSED".into()),
        prompt_cache_key_enabled: false,
        workspace_root: temp.path().join("workspaces"),
        artifact_root: temp.path().join("artifacts"),
        hostd_socket: None,
        sandbox_enabled: true,
        managed_deploy_enabled: false,
        budget: BudgetSnapshot::default(),
        lease_ms: 30_000,
        lease_renew_ms: 10_000,
    };
    let executor = Arc::new(FakeHostExecutor::default());
    let engine = RuntimeEngine::new_with_host_executor(
        database.clone(),
        Arc::new(SandboxWorkflowProvider {
            turn: AtomicUsize::new(0),
        }),
        config,
        Some(executor.clone()),
    )
    .unwrap();
    engine.run_once("worker-sandbox-a").await.unwrap();
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    let call = snapshot
        .tool_calls
        .iter()
        .find(|call| call.name == "sandbox.exec")
        .unwrap();
    assert_eq!(call.status, ToolCallStatus::WaitingForApproval);
    database
        .decide_tool_call(
            &task.latest_run_id,
            &call.call_id,
            &owner.id,
            soloops_domain::ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap();
    engine.run_once("worker-sandbox-b").await.unwrap();
    assert_eq!(executor.calls.lock().unwrap().len(), 1);
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(snapshot.evidence.iter().any(|item| item.kind == "command_result"));
    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Succeeded
    );
}
