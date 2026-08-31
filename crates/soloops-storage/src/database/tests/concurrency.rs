//! Concurrency and lock-behavior tests for idle claims and approvals.

use super::common::*;
use super::super::*;
use crate::{NewToolCall, PersistModelResponse};
use soloops_domain::{PolicyDecision, ToolRisk, UsageSnapshot};

#[tokio::test]
async fn idle_claims_do_not_wait_for_an_unrelated_writer() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("idle-claim.db");
    let database = Database::connect(&path).await.unwrap();
    database.migrate().await.unwrap();

    let mut writer = database.pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *writer)
        .await
        .unwrap();

    let queued = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        database.claim_next_run("idle-worker", 30_000),
    )
    .await
    .expect("an idle queued claim must stay read-only")
    .unwrap();
    assert!(queued.is_none());

    let approved = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        database.claim_approved_run("idle-worker", 30_000),
    )
    .await
    .expect("an idle approval claim must stay read-only")
    .unwrap();
    assert!(approved.is_none());

    sqlx::query("ROLLBACK").execute(&mut *writer).await.unwrap();
}
#[tokio::test]
async fn owner_approval_completes_during_frequent_idle_queue_polling() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("approval-polling.db");
    let database = Database::connect(&path).await.unwrap();
    database.migrate().await.unwrap();
    let (owner, task) = initialized_runtime(&database).await;
    let attempt_id = database
        .prepare_model_attempt(&task.latest_run_id, "approval-polling", 1)
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
                    call_id: "approval-polling-call".into(),
                    ordinal: 0,
                    name: "process.exec".into(),
                    arguments: json!({"program": "echo", "args": ["ok"]}),
                    risk: ToolRisk::Process,
                    policy: PolicyDecision::RequireApproval,
                }],
                usage: &UsageSnapshot::default(),
                made_progress: true,
            },
        )
        .await
        .unwrap();
    database
        .wait_for_tool_approval(&task.latest_run_id, "approval-polling-call")
        .await
        .unwrap();

    let poll_database = Database::connect(&path).await.unwrap();
    let poller = tokio::spawn(async move {
        for _ in 0..200 {
            assert!(
                poll_database
                    .claim_next_run("idle-poller", 30_000)
                    .await
                    .unwrap()
                    .is_none()
            );
            tokio::task::yield_now().await;
        }
    });

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        database.decide_tool_call(
            &task.latest_run_id,
            "approval-polling-call",
            &owner.id,
            soloops_domain::ApprovalDecision::Approve,
            None,
        ),
    )
    .await
    .expect("Owner approval must not be blocked by idle workers")
    .unwrap();
    poller.await.unwrap();

    assert_eq!(
        database
            .claim_approved_run("approved-worker", 30_000)
            .await
            .unwrap(),
        Some(task.latest_run_id)
    );
}
