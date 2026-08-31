//! Task/run/event/retry/tool-call persistence tests.

use super::common::*;
use super::super::*;
use crate::NewEvidence;

#[tokio::test]
async fn creates_a_task_and_ordered_event_stream() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Demo".into(),
                goal: "Exercise the worker".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(task.status, RunStatus::Queued);
    let leased = database
        .claim_next_run("worker-a", 30_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(leased.id, task.latest_run_id);
    database
        .transition_run(&task.latest_run_id, RunStatus::Planning, None)
        .await
        .unwrap();
    database
        .transition_run(
            &task.latest_run_id,
            RunStatus::Blocked,
            Some("Executor is not configured"),
        )
        .await
        .unwrap();
    let events = database
        .list_events(0, Some(&task.latest_run_id), 200)
        .await
        .unwrap();
    assert_eq!(events.len(), 4);
    assert!(events.windows(2).all(|pair| pair[0].sequence < pair[1].sequence));
}
#[tokio::test]
async fn logical_model_request_retries_get_incrementing_attempts_and_unique_ids() {
    let database = database().await;
    let (_owner, task) = initialized_runtime(&database).await;
    let request_key = format!("{}:turn:1", task.latest_run_id);

    let first_attempt = database
        .next_model_attempt(&task.latest_run_id, &request_key)
        .await
        .unwrap();
    let first_id = database
        .prepare_model_attempt(&task.latest_run_id, &request_key, first_attempt)
        .await
        .unwrap();
    let second_attempt = database
        .next_model_attempt(&task.latest_run_id, &request_key)
        .await
        .unwrap();
    let second_id = database
        .prepare_model_attempt(&task.latest_run_id, &request_key, second_attempt)
        .await
        .unwrap();

    assert_eq!((first_attempt, second_attempt), (1, 2));
    assert_ne!(first_id, second_id);
    let rows =
        sqlx::query("SELECT request_key, attempt FROM model_attempts WHERE run_id = ? ORDER BY attempt")
            .bind(&task.latest_run_id)
            .fetch_all(&database.pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.get::<String, _>("request_key") == request_key)
    );
    assert_eq!(rows[0].get::<i64, _>("attempt"), 1);
    assert_eq!(rows[1].get::<i64, _>("attempt"), 2);
}
#[tokio::test]
async fn leases_a_queued_run_only_once() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Only once".into(),
                goal: "Prevent duplicate execution".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        database
            .claim_next_run("worker-a", 30_000)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        database
            .claim_next_run("worker-b", 30_000)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn reports_invalid_event_payload_without_returning_a_fallback() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Corrupt event".into(),
                goal: "Surface invalid persisted JSON".into(),
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE events SET payload = 'not-json' WHERE run_id = ?")
        .bind(&task.latest_run_id)
        .execute(&database.pool)
        .await
        .unwrap();

    let error = database
        .list_events(0, Some(&task.latest_run_id), 200)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StorageError::InvalidEventPayload { sequence: 1, .. }
    ));
}
#[tokio::test]
async fn non_terminal_transition_preserves_an_existing_finished_at() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let task = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Preserve timestamp".into(),
                goal: "Avoid silently clearing persisted data".into(),
            },
        )
        .await
        .unwrap();
    database
        .claim_next_run("worker-a", 30_000)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE runs SET finished_at = 123 WHERE id = ?")
        .bind(&task.latest_run_id)
        .execute(&database.pool)
        .await
        .unwrap();

    let run = database
        .transition_run(&task.latest_run_id, RunStatus::Planning, None)
        .await
        .unwrap();
    assert_eq!(run.finished_at, Some(123));
}
#[tokio::test]
async fn tool_call_completion_is_compare_and_set() {
    let database = database().await;
    let (_, task) = initialized_runtime(&database).await;
    insert_read_only_tool_call(&database, &task.latest_run_id, "call-1").await;
    database
        .start_tool_call(&task.latest_run_id, "call-1")
        .await
        .unwrap();
    database
        .finish_tool_call(
            &task.latest_run_id,
            "call-1",
            &json!({"ok": true}),
            "completed once",
            Some(&NewEvidence {
                kind: "search_result".into(),
                summary: "bounded result".into(),
                artifact_ref: None,
                content_sha256: None,
            }),
            false,
        )
        .await
        .unwrap();

    let error = database
        .fail_tool_call(&task.latest_run_id, "call-1", "late_failure", "too late", false)
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::ToolCallStateConflict { .. }));
    let snapshot = database
        .runtime_snapshot(&task.latest_run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot.tool_calls[0].status,
        soloops_domain::ToolCallStatus::Completed
    );
    assert_eq!(snapshot.evidence.len(), 1);
    let completion_events = database
        .list_events(0, Some(&task.latest_run_id), 200)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == EventType::ToolCallCompleted)
        .count();
    assert_eq!(completion_events, 1);
}
