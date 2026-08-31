//! Owner/session/audit and host tool authorization tests.

use super::common::*;
use super::super::*;
use crate::{NewToolCall, PersistModelResponse};
use sha2::Digest;
use soloops_domain::{PolicyDecision, ToolRisk, UsageSnapshot};

#[tokio::test]
async fn rejects_creating_a_second_owner() {
    let database = database().await;
    let first = database.create_owner("owner", "hash").await.unwrap();
    assert_eq!(first.username, "owner");

    let error = database
        .create_owner("another-owner", "another-hash")
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::OwnerAlreadyExists));
    assert_eq!(database.count_users().await.unwrap(), 1);
}
#[tokio::test]
async fn host_tool_authorization_requires_matching_name_owner_approval_and_arguments() {
    let database = database().await;
    let (owner, task) = initialized_runtime(&database).await;
    let arguments = json!({"program": "echo", "args": ["hello"]});
    let attempt_id = database
        .prepare_model_attempt(&task.latest_run_id, "hostd-auth", 1)
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
                    call_id: "call-hostd".into(),
                    ordinal: 0,
                    name: "process.exec".into(),
                    arguments: arguments.clone(),
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
        .wait_for_tool_approval(&task.latest_run_id, "call-hostd")
        .await
        .unwrap();
    let digest = format!("{:x}", sha2::Sha256::digest(arguments.to_string().as_bytes()));
    assert!(
        !database
            .authorize_host_tool_call(&task.latest_run_id, "call-hostd", "process.exec", &digest)
            .await
            .unwrap()
    );
    database
        .decide_tool_call(
            &task.latest_run_id,
            "call-hostd",
            &owner.id,
            soloops_domain::ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap();
    database.claim_approved_run("worker-hostd", 30_000).await.unwrap();
    database
        .start_tool_call(&task.latest_run_id, "call-hostd")
        .await
        .unwrap();
    assert!(
        database
            .authorize_host_tool_call(&task.latest_run_id, "call-hostd", "process.exec", &digest)
            .await
            .unwrap()
    );
    assert!(
        !database
            .authorize_host_tool_call(&task.latest_run_id, "call-hostd", "process.exec", "tampered")
            .await
            .unwrap()
    );
    assert!(
        !database
            .authorize_host_tool_call(&task.latest_run_id, "call-hostd", "sandbox.exec", &digest)
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn rolls_back_task_creation_when_audit_insert_fails() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    sqlx::raw_sql(
        "CREATE TRIGGER fail_task_create_audit
         BEFORE INSERT ON audit_logs
         WHEN NEW.action = 'task.create'
         BEGIN
           SELECT RAISE(FAIL, 'forced audit failure');
         END;",
    )
    .execute(&database.pool)
    .await
    .unwrap();

    let result = database
        .create_task_with_run(
            &owner.id,
            &CreateTaskRequest {
                title: "Must roll back".into(),
                goal: "Keep business data and audit data atomic".into(),
            },
        )
        .await;

    assert!(result.is_err());
    for table in ["tasks", "runs", "events", "audit_logs"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&database.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table} should have been rolled back");
    }
}
#[tokio::test]
async fn rolls_back_session_creation_when_audit_insert_fails() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    sqlx::raw_sql(
        "CREATE TRIGGER fail_login_audit
         BEFORE INSERT ON audit_logs
         WHEN NEW.action = 'auth.login'
         BEGIN
           SELECT RAISE(FAIL, 'forced audit failure');
         END;",
    )
    .execute(&database.pool)
    .await
    .unwrap();

    let result = database
        .create_session_with_audit(
            &owner.id,
            "session-token-hash",
            now_ms() + 60_000,
            json!({ "remoteAddress": "127.0.0.1" }),
        )
        .await;

    assert!(result.is_err());
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(session_count, 0);
    assert_eq!(database.audit_count().await.unwrap(), 0);
}
#[tokio::test]
async fn rolls_back_session_revocation_when_audit_insert_fails() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    let token_hash = "session-token-hash";
    let session_id = database
        .create_session_with_audit(
            &owner.id,
            token_hash,
            now_ms() + 60_000,
            json!({ "remoteAddress": "127.0.0.1" }),
        )
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TRIGGER fail_logout_audit
         BEFORE INSERT ON audit_logs
         WHEN NEW.action = 'auth.logout'
         BEGIN
           SELECT RAISE(FAIL, 'forced audit failure');
         END;",
    )
    .execute(&database.pool)
    .await
    .unwrap();

    let result = database.revoke_session_with_audit(&session_id, &owner.id).await;

    assert!(result.is_err());
    assert!(database.find_session_owner(token_hash).await.unwrap().is_some());
    assert_eq!(database.audit_count().await.unwrap(), 1);
}
#[tokio::test]
async fn rejects_and_cleans_expired_or_revoked_sessions() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    database
        .create_session_with_audit(&owner.id, "expired", now_ms() - 1, json!({}))
        .await
        .unwrap();
    let revoked_id = database
        .create_session_with_audit(&owner.id, "revoked", now_ms() + 60_000, json!({}))
        .await
        .unwrap();
    database
        .create_session_with_audit(&owner.id, "valid", now_ms() + 60_000, json!({}))
        .await
        .unwrap();
    database
        .revoke_session_with_audit(&revoked_id, &owner.id)
        .await
        .unwrap();

    assert!(database.find_session_owner("expired").await.unwrap().is_none());
    assert!(database.find_session_owner("revoked").await.unwrap().is_none());
    assert!(database.find_session_owner("valid").await.unwrap().is_some());
    assert_eq!(database.cleanup_expired_sessions().await.unwrap(), 2);
    assert_eq!(database.cleanup_expired_sessions().await.unwrap(), 0);
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(session_count, 1);
}
