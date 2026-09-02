//! Run lease recovery tests.

use super::super::*;
use super::common::*;
use crate::{PersistModelResponse, RuntimeSessionConfig};
use soloops_domain::{BudgetSnapshot, UsageSnapshot};

#[tokio::test]
async fn expired_leases_enter_recovery_before_becoming_queueable() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Recover".into(),
                goal: "Recover an expired lease".into(),
            },
        )
        .await
        .unwrap();
    database
        .claim_next_run("worker-a", 30_000)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE runs SET lease_expires_at = 0 WHERE id = ?")
        .bind(&task.latest_run_id)
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(database.recover_expired_runs().await.unwrap(), 1);
    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::NeedsRecovery
    );
    assert_eq!(database.recover_safe_runs().await.unwrap(), 1);
    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Queued
    );
}
#[tokio::test]
async fn recovery_preserves_consecutive_no_progress_and_resumes_existing_runtime_as_running() {
    let database = database().await;
    let (_, task) = initialized_runtime(&database).await;
    for turn in 1..=2 {
        let request_key = format!("{}:turn:{turn}", task.latest_run_id);
        let attempt_id = database
            .prepare_model_attempt(&task.latest_run_id, &request_key, 1)
            .await
            .unwrap();
        database
            .persist_model_response(
                &task.latest_run_id,
                PersistModelResponse {
                    attempt_id: &attempt_id,
                    provider_request_id: None,
                    items: &[],
                    calls: &[],
                    usage: &UsageSnapshot {
                        model_turns: turn,
                        ..UsageSnapshot::default()
                    },
                    made_progress: false,
                },
            )
            .await
            .unwrap();
    }
    assert_eq!(
        database
            .runtime_execution_state(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .consecutive_no_progress,
        2
    );

    sqlx::query("UPDATE runs SET lease_expires_at = 0 WHERE id = ?")
        .bind(&task.latest_run_id)
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(database.recover_expired_runs().await.unwrap(), 1);
    assert_eq!(database.recover_safe_runs().await.unwrap(), 1);
    database
        .claim_next_run("worker-recovery", 30_000)
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

    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Running
    );
    assert_eq!(
        database
            .runtime_execution_state(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .consecutive_no_progress,
        2
    );
}
#[tokio::test]
async fn cancelled_run_cannot_be_rescheduled_or_reported() {
    let database = database().await;
    let (owner, task) = initialized_runtime(&database).await;
    database.cancel_run(&task.latest_run_id, &owner.id).await.unwrap();

    let retry_error = database
        .schedule_model_retry(&task.latest_run_id, "missing-attempt", "timeout", now_ms() + 1000)
        .await
        .unwrap_err();
    assert!(matches!(
        retry_error,
        StorageError::InvalidTransition {
            from: RunStatus::Cancelled,
            to: RunStatus::RetryScheduled
        }
    ));
    let report = soloops_domain::FinalReport {
        outcome: "succeeded".into(),
        summary: "must not persist".into(),
        completed: Vec::new(),
        incomplete: Vec::new(),
        risks: Vec::new(),
        evidence_ids: Vec::new(),
        rollback: None,
        usage: UsageSnapshot::default(),
        markdown: "must not persist".into(),
    };
    assert!(
        database
            .save_final_report(&task.latest_run_id, &report)
            .await
            .is_err()
    );
    let run = database.get_run(&task.latest_run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
    assert!(
        database
            .runtime_snapshot(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .report
            .is_none()
    );
}
#[tokio::test]
async fn safe_recovery_rolls_back_tool_reset_when_run_transition_fails() {
    let database = database().await;
    let (_, task) = initialized_runtime(&database).await;
    insert_read_only_tool_call(&database, &task.latest_run_id, "call-recover").await;
    database
        .start_tool_call(&task.latest_run_id, "call-recover")
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET lease_expires_at = 0 WHERE id = ?")
        .bind(&task.latest_run_id)
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(database.recover_expired_runs().await.unwrap(), 1);
    sqlx::raw_sql(
        "CREATE TRIGGER fail_recovery_transition
         BEFORE UPDATE OF status ON runs
         WHEN NEW.status = 'queued'
         BEGIN
           SELECT RAISE(FAIL, 'forced recovery failure');
         END;",
    )
    .execute(&database.pool)
    .await
    .unwrap();

    assert!(database.recover_safe_runs().await.is_err());
    let status: String =
        sqlx::query_scalar("SELECT status FROM tool_calls WHERE run_id = ? AND call_id = 'call-recover'")
            .bind(&task.latest_run_id)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(status, "running");
    assert_eq!(
        database
            .get_run(&task.latest_run_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::NeedsRecovery
    );
}
