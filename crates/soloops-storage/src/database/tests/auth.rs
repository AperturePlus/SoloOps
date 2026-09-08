//! Owner/session/audit and host tool authorization tests.

use super::super::*;
use super::common::*;
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
#[tokio::test]
async fn replacing_owner_password_updates_hash_revokes_sessions_and_audits() {
    let database = database().await;
    let owner = database.create_owner("owner", "old-hash").await.unwrap();
    database
        .create_session_with_audit(&owner.id, "token-a", now_ms() + 60_000, json!({}))
        .await
        .unwrap();
    database
        .create_session_with_audit(&owner.id, "token-b", now_ms() + 60_000, json!({}))
        .await
        .unwrap();
    let audit_before = database.audit_count().await.unwrap();

    database
        .replace_owner_password(&owner.id, "new-hash", "system", "auth.password_rotated")
        .await
        .unwrap();

    let user = database.find_first_owner().await.unwrap().unwrap();
    assert_eq!(user.id, owner.id);
    assert_eq!(user.password_hash, "new-hash");
    assert!(database.find_session_owner("token-a").await.unwrap().is_none());
    assert!(database.find_session_owner("token-b").await.unwrap().is_none());
    assert_eq!(database.audit_count().await.unwrap(), audit_before + 1);
    let audit_row =
        sqlx::query("SELECT action, actor_type, outcome FROM audit_logs ORDER BY sequence DESC LIMIT 1")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(audit_row.get::<String, _>("action"), "auth.password_rotated");
    assert_eq!(audit_row.get::<String, _>("actor_type"), "system");
    assert_eq!(audit_row.get::<String, _>("outcome"), "success");

    let error = database
        .replace_owner_password("missing-user", "new-hash", "system", "auth.password_rotated")
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::UserNotFound(user) if user == "missing-user"));
}
#[tokio::test]
async fn password_rotation_state_upserts_success_and_failure() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();

    let initial = database.get_password_rotation_state(&owner.id).await.unwrap();
    assert_eq!(
        initial,
        PasswordRotationRecord {
            owner_id: owner.id.clone(),
            last_rotated_at: None,
            last_email_at: None,
            last_email_error: None,
        }
    );

    database
        .record_password_rotation_failure(&owner.id, 1_000, "smtp_send_failed")
        .await
        .unwrap();
    let failed = database.get_password_rotation_state(&owner.id).await.unwrap();
    assert_eq!(failed.last_rotated_at, None);
    assert_eq!(failed.last_email_at, Some(1_000));
    assert_eq!(failed.last_email_error.as_deref(), Some("smtp_send_failed"));

    database
        .record_password_rotation_success(&owner.id, 2_000)
        .await
        .unwrap();
    let succeeded = database.get_password_rotation_state(&owner.id).await.unwrap();
    assert_eq!(succeeded.last_rotated_at, Some(2_000));
    assert_eq!(succeeded.last_email_at, Some(2_000));
    assert_eq!(succeeded.last_email_error, None);

    database
        .record_password_rotation_failure(&owner.id, 3_000, "smtp_send_failed")
        .await
        .unwrap();
    let failed_again = database.get_password_rotation_state(&owner.id).await.unwrap();
    assert_eq!(failed_again.last_rotated_at, Some(2_000));
    assert_eq!(failed_again.last_email_at, Some(3_000));
    assert_eq!(failed_again.last_email_error.as_deref(), Some("smtp_send_failed"));
}
#[tokio::test]
async fn smtp_settings_roundtrip_delete_and_audit_exclude_the_password() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    assert!(database.get_smtp_settings(&owner.id).await.unwrap().is_none());

    let record = SmtpSettingsRecord {
        owner_id: owner.id.clone(),
        host: "smtp.example.com".into(),
        port: 465,
        security: "tls".into(),
        from_mailbox: "SoloOps <soloops@example.com>".into(),
        username: Some("soloops".into()),
        password: Some("secret".into()),
        updated_at: now_ms(),
    };
    let saved = database.upsert_smtp_settings(&record).await.unwrap();
    assert_eq!(saved.host, "smtp.example.com");
    assert_eq!(saved.port, 465);
    assert_eq!(saved.security, "tls");
    assert_eq!(saved.username.as_deref(), Some("soloops"));
    assert_eq!(saved.password.as_deref(), Some("secret"));

    let context_row = sqlx::query(
        "SELECT context FROM audit_logs WHERE action = 'settings.smtp.update' ORDER BY sequence DESC LIMIT 1",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    let context: String = context_row.get("context");
    assert!(context.contains("passwordConfigured"));
    assert!(
        !context.contains("secret"),
        "the password must never reach the audit log"
    );

    let updated = SmtpSettingsRecord {
        security: "starttls".into(),
        username: None,
        password: None,
        ..record
    };
    let updated = database.upsert_smtp_settings(&updated).await.unwrap();
    assert_eq!(updated.security, "starttls");
    assert_eq!(updated.password, None);
    assert_eq!(updated.username, None);

    let invalid = SmtpSettingsRecord {
        owner_id: owner.id.clone(),
        host: "smtp.example.com".into(),
        port: 465,
        security: "plain".into(),
        from_mailbox: "SoloOps <soloops@example.com>".into(),
        username: Some("soloops".into()),
        password: Some("secret".into()),
        updated_at: now_ms(),
    };
    let error = database.upsert_smtp_settings(&invalid).await.unwrap_err();
    assert!(matches!(error, StorageError::InvalidSmtpSecurity(security) if security == "plain"));

    assert!(database.delete_smtp_settings(&owner.id).await.unwrap());
    assert!(!database.delete_smtp_settings(&owner.id).await.unwrap());
    assert!(database.get_smtp_settings(&owner.id).await.unwrap().is_none());
}

#[tokio::test]
async fn model_settings_roundtrip_delete_and_audit_exclude_the_api_key() {
    let database = database().await;
    let owner = database.create_owner("owner", "hash").await.unwrap();
    assert!(database.get_model_settings(&owner.id).await.unwrap().is_none());

    let record = ModelSettingsRecord {
        owner_id: owner.id.clone(),
        base_url: "https://models.example.com/v1".into(),
        model_name: "example-model".into(),
        api_key: Some("secret-key".into()),
        updated_at: now_ms(),
    };
    let saved = database.upsert_model_settings(&record).await.unwrap();
    assert_eq!(saved.base_url, "https://models.example.com/v1");
    assert_eq!(saved.model_name, "example-model");
    assert_eq!(saved.api_key.as_deref(), Some("secret-key"));

    let context_row = sqlx::query(
        "SELECT context FROM audit_logs WHERE action = 'settings.model.update' ORDER BY sequence DESC LIMIT 1",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    let context: String = context_row.get("context");
    assert!(context.contains("apiKeyConfigured"));
    assert!(
        !context.contains("secret-key"),
        "the API key must never reach the audit log"
    );

    // Keyless rows (local endpoints that authenticate nothing) stay usable.
    let updated = ModelSettingsRecord {
        api_key: None,
        ..record
    };
    let updated = database.upsert_model_settings(&updated).await.unwrap();
    assert_eq!(updated.api_key, None);

    assert!(database.delete_model_settings(&owner.id).await.unwrap());
    assert!(!database.delete_model_settings(&owner.id).await.unwrap());
    assert!(database.get_model_settings(&owner.id).await.unwrap().is_none());
}
