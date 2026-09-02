//! Managed deployment revision, lease, and operation tests.

use super::super::*;
use super::common::*;
use crate::{NewManagedDeploymentRevision, NewToolCall, PersistModelResponse};
use sha2::Digest;
use soloops_domain::{PolicyDecision, ToolRisk, UsageSnapshot};

#[tokio::test]
async fn managed_deployment_revision_is_immutable_and_read_only_host_action_is_authorized() {
    let database = database().await;
    let (_owner, task) = initialized_runtime(&database).await;
    let arguments = json!({
        "projectId": "demo",
        "composePath": "compose.yaml",
        "caddyFragmentPath": "site.caddy"
    });
    let attempt_id = database
        .prepare_model_attempt(&task.latest_run_id, "managed-plan", 1)
        .await
        .unwrap();
    database
        .persist_model_response(
            &task.latest_run_id,
            PersistModelResponse {
                attempt_id: &attempt_id,
                provider_request_id: None,
                items: &[],
                calls: &[NewToolCall {
                    call_id: "managed-plan-call".into(),
                    ordinal: 0,
                    name: "managed.deploy.plan".into(),
                    arguments: arguments.clone(),
                    risk: ToolRisk::ReadOnly,
                    policy: PolicyDecision::Allow,
                }],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
    database
        .start_tool_call(&task.latest_run_id, "managed-plan-call")
        .await
        .unwrap();
    let digest = format!("{:x}", sha2::Sha256::digest(arguments.to_string().as_bytes()));
    assert!(
        database
            .authorize_host_tool_call(
                &task.latest_run_id,
                "managed-plan-call",
                "managed.deploy.plan",
                &digest,
            )
            .await
            .unwrap()
    );
    let preview = json!({"services": ["web"], "images": ["demo@sha256:fixed"]});
    let source = json!({"healthPath": "/health"});
    let revision = database
        .create_managed_deployment_revision(NewManagedDeploymentRevision {
            project_id: "demo",
            run_id: &task.latest_run_id,
            plan_call_id: "managed-plan-call",
            proposal_sha256: &"a".repeat(64),
            compose_sha256: &"b".repeat(64),
            caddy_sha256: &"c".repeat(64),
            source: &source,
            preview: &preview,
            bundle_path: "/managed/demo/revision",
        })
        .await
        .unwrap();
    let duplicate = database
        .create_managed_deployment_revision(NewManagedDeploymentRevision {
            project_id: "demo",
            run_id: &task.latest_run_id,
            plan_call_id: "managed-plan-call",
            proposal_sha256: &"a".repeat(64),
            compose_sha256: &"b".repeat(64),
            caddy_sha256: &"c".repeat(64),
            source: &source,
            preview: &preview,
            bundle_path: "/managed/demo/revision",
        })
        .await
        .unwrap();
    assert_eq!(duplicate.id, revision.id);
    assert_eq!(duplicate.preview, preview);
    assert_eq!(duplicate.status, "proposed");

    let conflict = database
        .create_managed_deployment_revision(NewManagedDeploymentRevision {
            project_id: "demo",
            run_id: &task.latest_run_id,
            plan_call_id: "managed-plan-call",
            proposal_sha256: &"d".repeat(64),
            compose_sha256: &"b".repeat(64),
            caddy_sha256: &"c".repeat(64),
            source: &source,
            preview: &preview,
            bundle_path: "/managed/demo/revision",
        })
        .await
        .unwrap_err();
    assert!(matches!(conflict, StorageError::ManagedDeploymentConflict(_)));

    let operation_attempt = database
        .prepare_model_attempt(&task.latest_run_id, "managed-apply", 1)
        .await
        .unwrap();
    database
        .persist_model_response(
            &task.latest_run_id,
            PersistModelResponse {
                attempt_id: &operation_attempt,
                provider_request_id: None,
                items: &[],
                calls: &[
                    NewToolCall {
                        call_id: "managed-apply-call".into(),
                        ordinal: 0,
                        name: "managed.deploy.apply".into(),
                        arguments: json!({"proposalId": revision.id, "proposalSha256": revision.proposal_sha256}),
                        risk: ToolRisk::Privileged,
                        policy: PolicyDecision::RequireApproval,
                    },
                    NewToolCall {
                        call_id: "managed-apply-concurrent".into(),
                        ordinal: 1,
                        name: "managed.deploy.apply".into(),
                        arguments: json!({"proposalId": revision.id, "proposalSha256": revision.proposal_sha256}),
                        risk: ToolRisk::Privileged,
                        policy: PolicyDecision::RequireApproval,
                    },
                ],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
    let operation = database
        .begin_managed_deployment_operation(
            "managed-apply-call",
            &task.latest_run_id,
            "demo",
            "apply",
            &revision.id,
            None,
            "lease-a",
            60_000,
        )
        .await
        .unwrap();
    let retry = database
        .begin_managed_deployment_operation(
            "managed-apply-call",
            &task.latest_run_id,
            "demo",
            "apply",
            &revision.id,
            None,
            "lease-a",
            60_000,
        )
        .await
        .unwrap();
    assert_eq!(retry.call_id, operation.call_id);
    assert_eq!(retry.lease_token.as_deref(), Some("lease-a"));
    assert!(matches!(
        database
            .begin_managed_deployment_operation(
                "managed-apply-call",
                &task.latest_run_id,
                "demo",
                "rollback",
                &revision.id,
                None,
                "lease-mismatch",
                60_000,
            )
            .await
            .unwrap_err(),
        StorageError::ManagedDeploymentConflict(_)
    ));
    assert!(
        database
            .begin_managed_deployment_operation(
                "managed-apply-concurrent",
                &task.latest_run_id,
                "demo",
                "apply",
                &revision.id,
                None,
                "lease-b",
                60_000,
            )
            .await
            .is_err()
    );
    assert!(
        database
            .update_managed_deployment_phase("managed-apply-call", "lease-a", "prepared", "applying_compose",)
            .await
            .unwrap()
    );
    assert!(
        !database
            .update_managed_deployment_phase("managed-apply-call", "lease-a", "prepared", "applying_caddy",)
            .await
            .unwrap()
    );
    database
        .fail_managed_deployment_operation(
            "managed-apply-call",
            "lease-a",
            "test_failure",
            &json!({"status": "failed"}),
        )
        .await
        .unwrap();
    assert_eq!(
        database
            .begin_managed_deployment_operation(
                "managed-apply-concurrent",
                &task.latest_run_id,
                "demo",
                "apply",
                &revision.id,
                None,
                "lease-b",
                60_000,
            )
            .await
            .unwrap()
            .phase,
        "prepared"
    );
}
#[tokio::test]
async fn managed_deployment_lease_claim_is_exclusive_and_old_owner_cannot_commit() {
    let database = database().await;
    let (_owner, task) = initialized_runtime(&database).await;
    let attempt = database
        .prepare_model_attempt(&task.latest_run_id, "lease-operation", 1)
        .await
        .unwrap();
    database
        .persist_model_response(
            &task.latest_run_id,
            PersistModelResponse {
                attempt_id: &attempt,
                provider_request_id: None,
                items: &[],
                calls: &[
                    NewToolCall {
                        call_id: "lease-plan".into(),
                        ordinal: 0,
                        name: "managed.deploy.plan".into(),
                        arguments: json!({}),
                        risk: ToolRisk::ReadOnly,
                        policy: PolicyDecision::Allow,
                    },
                    NewToolCall {
                        call_id: "lease-apply".into(),
                        ordinal: 1,
                        name: "managed.deploy.apply".into(),
                        arguments: json!({}),
                        risk: ToolRisk::Privileged,
                        policy: PolicyDecision::Allow,
                    },
                ],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
    let revision = database
        .create_managed_deployment_revision(NewManagedDeploymentRevision {
            project_id: "lease-project",
            run_id: &task.latest_run_id,
            plan_call_id: "lease-plan",
            proposal_sha256: &"a".repeat(64),
            compose_sha256: &"b".repeat(64),
            caddy_sha256: &"c".repeat(64),
            source: &json!({"healthPath": "/"}),
            preview: &json!({}),
            bundle_path: "/managed/lease-project/revision",
        })
        .await
        .unwrap();
    database
        .begin_managed_deployment_operation(
            "lease-apply",
            &task.latest_run_id,
            "lease-project",
            "apply",
            &revision.id,
            None,
            "old-owner",
            -1,
        )
        .await
        .unwrap();

    let claimed = database
        .claim_expired_managed_deployment_operation("reconciler", 60_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.call_id, "lease-apply");
    assert_eq!(claimed.lease_token.as_deref(), Some("reconciler"));
    assert_eq!(claimed.recovery_attempts, 1);
    assert!(
        database
            .claim_expired_managed_deployment_operation("other", 60_000)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !database
            .renew_managed_deployment_lease("lease-apply", "old-owner", 60_000)
            .await
            .unwrap()
    );
    assert!(
        !database
            .update_managed_deployment_phase("lease-apply", "old-owner", "prepared", "applying_compose",)
            .await
            .unwrap()
    );
    assert!(matches!(
        database
            .fail_managed_deployment_operation(
                "lease-apply",
                "old-owner",
                "stale",
                &json!({"status": "failed"}),
            )
            .await
            .unwrap_err(),
        StorageError::ManagedDeploymentConflict(_)
    ));
}
