//! Core runtime loop tests: no-progress blocking, cancellation, and retry keys.

use super::providers::{CancellingProvider, FakeProvider, RetryingProvider};
use crate::{ModelResponse, RuntimeConfig, RuntimeEngine, SecretRef};
use soloops_domain::{BudgetSnapshot, CreateTaskRequest, RunStatus, UsageSnapshot};
use soloops_storage::Database;
use std::sync::{Arc, Mutex, atomic::AtomicUsize};

#[tokio::test]
async fn plain_text_without_progress_blocks_after_three_turns() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Runtime".into(),
                goal: "Make progress".into(),
            },
        )
        .await
        .unwrap();
    let provider = Arc::new(FakeProvider {
        responses: Mutex::new(
            (0..3)
                .map(|_| ModelResponse {
                    assistant_message: Some("thinking".into()),
                    tool_calls: vec![],
                    usage: UsageSnapshot::default(),
                    stop_reason: Some("stop".into()),
                    provider_request_id: None,
                })
                .collect(),
        ),
    });
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
    let engine = RuntimeEngine::new(database.clone(), provider, config).unwrap();
    engine.run_once("worker").await.unwrap();
    let run = database.get_run(&task.latest_run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Blocked);
}

#[tokio::test]
async fn cancellation_during_model_call_is_not_rescheduled() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Cancel".into(),
                goal: "Do not retry after cancellation".into(),
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
        Arc::new(CancellingProvider {
            database: database.clone(),
            owner_id: owner.id,
        }),
        config,
    )
    .unwrap();

    engine.run_once("worker").await.unwrap();
    let run = database.get_run(&task.latest_run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
}

#[tokio::test]
async fn retries_reuse_the_logical_turn_key_but_use_unique_attempt_keys() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Retry".into(),
                goal: "Retry without consuming a turn".into(),
            },
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let provider = Arc::new(RetryingProvider {
        calls: Mutex::new(Vec::new()),
        invocation: AtomicUsize::new(0),
    });
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
    let engine = RuntimeEngine::new(database.clone(), provider.clone(), config).unwrap();

    engine.run_once("worker-a").await.unwrap();
    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::RetryScheduled
    );
    engine.run_once("worker-b").await.unwrap();

    {
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].0, calls[1].0);
        assert_ne!(calls[0].1, calls[1].1);
        assert_ne!(calls[1].0, calls[2].0);
    }
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.usage.model_turns, 3);
    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Blocked
    );
}
