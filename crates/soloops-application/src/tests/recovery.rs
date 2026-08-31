//! Interrupted workspace create/replace recovery tests.

use super::providers::{ReplaceWorkflowProvider, WorkflowProvider};
use crate::{
    RuntimeConfig, RuntimeEngine, SecretRef,
    tools::prepare_workspace_write,
};
use soloops_domain::{BudgetSnapshot, CreateTaskRequest, RunStatus};
use soloops_storage::Database;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicUsize},
};

struct InterruptedCreateHarness {
    _temp: tempfile::TempDir,
    database: Database,
    engine: RuntimeEngine,
    run_id: String,
    target: PathBuf,
}

async fn interrupted_create_harness(
    published_content: Option<&str>,
    store_recovery: bool,
) -> InterruptedCreateHarness {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Recover create".into(),
                goal: "Create demo.txt across a lease loss".into(),
            },
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspaces").join(&task.latest_run_id);
    let target = workspace.join("demo.txt");
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
    let call = database
        .pending_tool_calls(&task.latest_run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|call| call.summary.name == "workspace.create")
        .unwrap();
    database
        .decide_tool_call(
            &task.latest_run_id,
            &call.summary.call_id,
            &owner.id,
            soloops_domain::ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        database
            .claim_approved_run("crashed-worker", -1)
            .await
            .unwrap()
            .as_deref(),
        Some(task.latest_run_id.as_str())
    );
    if store_recovery {
        let recovery = prepare_workspace_write(&workspace, &call.summary.name, &call.arguments)
            .await
            .unwrap()
            .unwrap();
        let recovery = serde_json::to_value(recovery).unwrap();
        database
            .start_tool_call_with_recovery(&task.latest_run_id, &call.summary.call_id, Some(&recovery))
            .await
            .unwrap();
    } else {
        database
            .start_tool_call(&task.latest_run_id, &call.summary.call_id)
            .await
            .unwrap();
    }
    if let Some(content) = published_content {
        tokio::fs::write(&target, content).await.unwrap();
    }
    assert_eq!(database.recover_expired_runs().await.unwrap(), 1);
    assert_eq!(database.recover_safe_runs().await.unwrap(), 1);

    InterruptedCreateHarness {
        _temp: temp,
        database,
        engine,
        run_id: task.latest_run_id,
        target,
    }
}

#[tokio::test]
async fn interrupted_create_completes_from_the_expected_hash_or_retries_when_absent() {
    for published_content in [Some("hello"), None] {
        let harness = interrupted_create_harness(published_content, true).await;
        harness.engine.run_once("recovery-worker").await.unwrap();
        let run = harness.database.get_run(&harness.run_id).await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Succeeded);
        assert_eq!(tokio::fs::read_to_string(&harness.target).await.unwrap(), "hello");
        let snapshot = harness
            .database
            .runtime_snapshot(&harness.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.workspace_revision, 1);
        assert!(snapshot.evidence.iter().any(|item| item.kind == "file_mutation"));
    }
}

#[tokio::test]
async fn interrupted_create_blocks_without_overwriting_unknown_content() {
    let harness = interrupted_create_harness(Some("external change"), true).await;
    harness.engine.run_once("recovery-worker").await.unwrap();
    let run = harness.database.get_run(&harness.run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Blocked);
    assert_eq!(
        tokio::fs::read_to_string(&harness.target).await.unwrap(),
        "external change"
    );
}

#[tokio::test]
async fn legacy_interrupted_create_removes_only_a_strict_prefix_before_retrying() {
    let harness = interrupted_create_harness(Some("hel"), false).await;
    harness.engine.run_once("recovery-worker").await.unwrap();
    assert_eq!(
        harness
            .database
            .get_run(&harness.run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Succeeded
    );
    assert_eq!(tokio::fs::read_to_string(&harness.target).await.unwrap(), "hello");
}

#[tokio::test]
async fn legacy_interrupted_create_blocks_on_non_prefix_content() {
    let harness = interrupted_create_harness(Some("hex"), false).await;
    harness.engine.run_once("recovery-worker").await.unwrap();
    assert_eq!(
        harness
            .database
            .get_run(&harness.run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Blocked
    );
    assert_eq!(tokio::fs::read_to_string(&harness.target).await.unwrap(), "hex");
}

async fn interrupted_replace_harness(current_content: &str) -> InterruptedCreateHarness {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Recover replace".into(),
                goal: "Replace demo.txt across a lease loss".into(),
            },
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspaces").join(&task.latest_run_id);
    let target = workspace.join("demo.txt");
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::write(&target, "hello").await.unwrap();
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
        Arc::new(ReplaceWorkflowProvider {
            turn: AtomicUsize::new(0),
        }),
        config,
    )
    .unwrap();
    engine.run_once("worker-a").await.unwrap();
    let call = database
        .pending_tool_calls(&task.latest_run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|call| call.summary.name == "workspace.replace")
        .unwrap();
    database
        .decide_tool_call(
            &task.latest_run_id,
            &call.summary.call_id,
            &owner.id,
            soloops_domain::ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap();
    database
        .claim_approved_run("crashed-worker", -1)
        .await
        .unwrap()
        .unwrap();
    let recovery = prepare_workspace_write(&workspace, &call.summary.name, &call.arguments)
        .await
        .unwrap()
        .unwrap();
    let recovery = serde_json::to_value(recovery).unwrap();
    database
        .start_tool_call_with_recovery(&task.latest_run_id, &call.summary.call_id, Some(&recovery))
        .await
        .unwrap();
    tokio::fs::write(&target, current_content).await.unwrap();
    assert_eq!(database.recover_expired_runs().await.unwrap(), 1);
    assert_eq!(database.recover_safe_runs().await.unwrap(), 1);

    InterruptedCreateHarness {
        _temp: temp,
        database,
        engine,
        run_id: task.latest_run_id,
        target,
    }
}

#[tokio::test]
async fn interrupted_replace_completes_at_expected_hash_or_retries_at_previous_hash() {
    for current_content in ["goodbye", "hello"] {
        let harness = interrupted_replace_harness(current_content).await;
        harness.engine.run_once("recovery-worker").await.unwrap();
        assert_eq!(
            harness
                .database
                .get_run(&harness.run_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            RunStatus::Succeeded
        );
        assert_eq!(
            tokio::fs::read_to_string(&harness.target).await.unwrap(),
            "goodbye"
        );
        let snapshot = harness
            .database
            .runtime_snapshot(&harness.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.workspace_revision, 1);
        assert!(snapshot.evidence.iter().any(|item| item.kind == "file_mutation"));
    }
}

#[tokio::test]
async fn interrupted_replace_blocks_when_neither_known_hash_matches() {
    let harness = interrupted_replace_harness("external change").await;
    harness.engine.run_once("recovery-worker").await.unwrap();
    assert_eq!(
        harness
            .database
            .get_run(&harness.run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Blocked
    );
    assert_eq!(
        tokio::fs::read_to_string(&harness.target).await.unwrap(),
        "external change"
    );
}
