//! Managed deployment recovery and timeout tests.

use super::providers::{FakeHostExecutor, FakeProvider, ManagedRecoveryProvider, SlowManagedExecutor};
use crate::{ModelResponse, ModelToolCall, RuntimeConfig, RuntimeEngine, SecretRef};
use serde_json::json;
use soloops_domain::{
    AgentPlan, BudgetSnapshot, CreateTaskRequest, RunStatus, ToolCallStatus, UsageSnapshot,
};
use soloops_storage::{
    Database, NewManagedDeploymentRevision, NewToolCall, PersistModelResponse, RuntimeSessionConfig,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, atomic::Ordering};

#[tokio::test]
async fn interrupted_managed_deployment_replays_result_and_emits_evidence() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Managed recovery".into(),
                goal: "Recover an interrupted managed deployment".into(),
            },
        )
        .await
        .unwrap();
    database
        .claim_next_run("planning-worker", 30_000)
        .await
        .unwrap()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspaces").join(&task.latest_run_id);
    let artifacts = temp.path().join("artifacts").join(&task.latest_run_id);
    database
        .initialize_runtime(
            &task.latest_run_id,
            &RuntimeSessionConfig {
                provider: "test".into(),
                model: "test".into(),
                prompt_version: "test".into(),
                workspace_path: workspace.to_string_lossy().into_owned(),
                artifact_path: artifacts.to_string_lossy().into_owned(),
                budget: BudgetSnapshot::default(),
            },
        )
        .await
        .unwrap();
    let plan_attempt = database
        .prepare_model_attempt(&task.latest_run_id, "managed-plan-setup", 1)
        .await
        .unwrap();
    database
        .persist_model_response(
            &task.latest_run_id,
            PersistModelResponse {
                attempt_id: &plan_attempt,
                provider_request_id: None,
                items: &[],
                calls: &[NewToolCall {
                    call_id: "managed-plan-proposal".into(),
                    ordinal: 0,
                    name: "managed.deploy.plan".into(),
                    arguments: json!({
                        "projectId": "demo",
                        "composePath": "compose.yaml",
                        "caddyFragmentPath": "site.caddy",
                    }),
                    risk: soloops_domain::ToolRisk::ReadOnly,
                    policy: soloops_domain::PolicyDecision::Allow,
                }],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
    database
        .start_tool_call(&task.latest_run_id, "managed-plan-proposal")
        .await
        .unwrap();
    database
        .finish_tool_call(
            &task.latest_run_id,
            "managed-plan-proposal",
            &json!({"status": "proposed"}),
            "Managed deployment proposal stored",
            None,
            false,
        )
        .await
        .unwrap();
    let proposal_sha256 = "a".repeat(64);
    let compose_sha256 = "b".repeat(64);
    let caddy_sha256 = "c".repeat(64);
    let source = json!({"healthPath": "/health"});
    let preview = json!({"site": "demo.example", "services": ["web"]});
    let revision = database
        .create_managed_deployment_revision(NewManagedDeploymentRevision {
            project_id: "demo",
            run_id: &task.latest_run_id,
            plan_call_id: "managed-plan-proposal",
            proposal_sha256: &proposal_sha256,
            compose_sha256: &compose_sha256,
            caddy_sha256: &caddy_sha256,
            source: &source,
            preview: &preview,
            bundle_path: "/managed/demo/revision",
        })
        .await
        .unwrap();
    let plan: AgentPlan = serde_json::from_value(json!({
        "summary": "Deploy a verified revision",
        "steps": [{"id": "deploy", "title": "Deploy", "status": "in_progress", "required": true}]
    }))
    .unwrap();
    database.save_plan(&task.latest_run_id, &plan).await.unwrap();
    let attempt = database
        .prepare_model_attempt(&task.latest_run_id, "managed-recovery", 1)
        .await
        .unwrap();
    database
        .persist_model_response(
            &task.latest_run_id,
            PersistModelResponse {
                attempt_id: &attempt,
                provider_request_id: None,
                items: &[],
                calls: &[NewToolCall {
                    call_id: "managed-apply-recovery".into(),
                    ordinal: 0,
                    name: "managed.deploy.apply".into(),
                    arguments: json!({"proposalId": revision.id, "proposalSha256": proposal_sha256}),
                    risk: soloops_domain::ToolRisk::Privileged,
                    policy: soloops_domain::PolicyDecision::RequireApproval,
                }],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
    database
        .wait_for_tool_approval(&task.latest_run_id, "managed-apply-recovery")
        .await
        .unwrap();
    database
        .decide_tool_call(
            &task.latest_run_id,
            "managed-apply-recovery",
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
    database
        .start_tool_call(&task.latest_run_id, "managed-apply-recovery")
        .await
        .unwrap();
    database
        .begin_managed_deployment_operation(
            "managed-apply-recovery",
            &task.latest_run_id,
            "demo",
            "apply",
            &revision.id,
            None,
            "application-recovery-lease",
            60_000,
        )
        .await
        .unwrap();
    database
        .finish_managed_deployment_operation(
            "managed-apply-recovery",
            "application-recovery-lease",
            &json!({
                "action": "apply",
                "projectId": "demo",
                "status": "active",
                "revisionId": revision.id,
                "previousRevisionId": null,
                "proposalSha256": proposal_sha256,
                "preview": preview,
            }),
            &json!({"composeHealthy": true, "routeStatus": 200}),
        )
        .await
        .unwrap();
    assert_eq!(database.recover_expired_runs().await.unwrap(), 1);
    assert_eq!(database.recover_safe_runs().await.unwrap(), 1);

    let config = RuntimeConfig {
        model_base_url: "http://unused".into(),
        model_name: "fake".into(),
        model_api_key_ref: SecretRef::Env("UNUSED".into()),
        prompt_cache_key_enabled: false,
        workspace_root: temp.path().join("workspaces"),
        artifact_root: temp.path().join("artifacts"),
        hostd_socket: None,
        sandbox_enabled: false,
        managed_deploy_enabled: true,
        budget: BudgetSnapshot::default(),
        lease_ms: 30_000,
        lease_renew_ms: 10_000,
    };
    let executor = Arc::new(FakeHostExecutor::default());
    let engine = RuntimeEngine::new_with_host_executor(
        database.clone(),
        Arc::new(ManagedRecoveryProvider),
        config,
        Some(executor.clone()),
    )
    .unwrap();
    engine.run_once("recovery-worker").await.unwrap();

    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        snapshot
            .evidence
            .iter()
            .any(|item| item.kind == "managed_deployment")
    );
    assert_eq!(executor.calls.lock().unwrap().len(), 0);
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
async fn first_managed_deployment_timeout_blocks_without_retrying_the_proposal() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Managed timeout".into(),
                goal: "Do not retry an uncertain deployment".into(),
            },
        )
        .await
        .unwrap();
    database
        .claim_next_run("managed-timeout-setup", -1)
        .await
        .unwrap()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspaces").join(&task.latest_run_id);
    let artifacts = temp.path().join("artifacts").join(&task.latest_run_id);
    database
        .initialize_runtime(
            &task.latest_run_id,
            &RuntimeSessionConfig {
                provider: "test".into(),
                model: "test".into(),
                prompt_version: "test".into(),
                workspace_path: workspace.to_string_lossy().into_owned(),
                artifact_path: artifacts.to_string_lossy().into_owned(),
                budget: BudgetSnapshot::default(),
            },
        )
        .await
        .unwrap();
    let plan_attempt = database
        .prepare_model_attempt(&task.latest_run_id, "managed-timeout-plan-setup", 1)
        .await
        .unwrap();
    database
        .persist_model_response(
            &task.latest_run_id,
            PersistModelResponse {
                attempt_id: &plan_attempt,
                provider_request_id: None,
                items: &[],
                calls: &[NewToolCall {
                    call_id: "managed-timeout-plan".into(),
                    ordinal: 0,
                    name: "managed.deploy.plan".into(),
                    arguments: json!({
                        "projectId": "demo",
                        "composePath": "compose.yaml",
                        "caddyFragmentPath": "site.caddy",
                    }),
                    risk: soloops_domain::ToolRisk::ReadOnly,
                    policy: soloops_domain::PolicyDecision::Allow,
                }],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
    database
        .start_tool_call(&task.latest_run_id, "managed-timeout-plan")
        .await
        .unwrap();
    database
        .finish_tool_call(
            &task.latest_run_id,
            "managed-timeout-plan",
            &json!({"status": "proposed"}),
            "Managed deployment proposal stored",
            None,
            false,
        )
        .await
        .unwrap();
    let proposal_sha256 = "a".repeat(64);
    let compose_sha256 = "b".repeat(64);
    let caddy_sha256 = "c".repeat(64);
    let source = json!({"healthPath": "/health"});
    let preview = json!({"site": "demo.example", "services": ["web"]});
    let revision = database
        .create_managed_deployment_revision(NewManagedDeploymentRevision {
            project_id: "demo",
            run_id: &task.latest_run_id,
            plan_call_id: "managed-timeout-plan",
            proposal_sha256: &proposal_sha256,
            compose_sha256: &compose_sha256,
            caddy_sha256: &caddy_sha256,
            source: &source,
            preview: &preview,
            bundle_path: "/managed/demo/timeout-revision",
        })
        .await
        .unwrap();
    let provider = Arc::new(FakeProvider {
        responses: Mutex::new(VecDeque::from([ModelResponse {
            assistant_message: None,
            tool_calls: vec![ModelToolCall {
                call_id: "managed-timeout-apply".into(),
                name: "managed.deploy.apply".into(),
                arguments: json!({
                    "proposalId": revision.id,
                    "proposalSha256": proposal_sha256,
                }),
            }],
            usage: UsageSnapshot::default(),
            stop_reason: Some("tool_calls".into()),
            provider_request_id: None,
        }])),
    });
    let budget = BudgetSnapshot {
        max_tool_duration_ms: 10,
        ..BudgetSnapshot::default()
    };
    let config = RuntimeConfig {
        model_base_url: "http://unused".into(),
        model_name: "fake".into(),
        model_api_key_ref: SecretRef::Env("UNUSED".into()),
        prompt_cache_key_enabled: false,
        workspace_root: temp.path().join("workspaces"),
        artifact_root: temp.path().join("artifacts"),
        hostd_socket: None,
        sandbox_enabled: false,
        managed_deploy_enabled: true,
        budget,
        lease_ms: 30_000,
        lease_renew_ms: 10_000,
    };
    let executor = Arc::new(SlowManagedExecutor::default());
    let engine =
        RuntimeEngine::new_with_host_executor(database.clone(), provider, config, Some(executor.clone()))
            .unwrap();

    assert_eq!(database.recover_expired_runs().await.unwrap(), 1);
    assert_eq!(database.recover_safe_runs().await.unwrap(), 1);

    engine.run_once("managed-timeout-planner").await.unwrap();
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    let call = snapshot
        .tool_calls
        .iter()
        .find(|call| call.call_id == "managed-timeout-apply")
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

    engine.run_once("managed-timeout-worker").await.unwrap();

    let run = database.get_run(&task.latest_run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Blocked);
    assert!(run.status_reason.unwrap().contains("outcome is unknown"));
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    let call = snapshot
        .tool_calls
        .iter()
        .find(|call| call.call_id == "managed-timeout-apply")
        .unwrap();
    assert_eq!(call.status, ToolCallStatus::Failed);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert!(engine.run_once("managed-timeout-idle").await.unwrap().is_none());
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
}
