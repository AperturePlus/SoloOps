//! Schema adoption and migration tests.

use super::common::*;
use super::super::*;

#[tokio::test]
async fn adopts_the_typescript_baseline_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.db");
    let database = Database::connect(&path).await.unwrap();
    sqlx::raw_sql(BASELINE_SQL).execute(&database.pool).await.unwrap();
    database.migrate().await.unwrap();
    assert_eq!(database.count_users().await.unwrap(), 0);
    let adopted: i64 = sqlx::query_scalar("SELECT adopted FROM soloops_schema_migrations WHERE version = 1")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(adopted, 1);
    let runtime_version: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM soloops_schema_migrations WHERE version = 2")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(runtime_version, 1);
    let recovery_version: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM soloops_schema_migrations WHERE version = 3")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(recovery_version, 1);
    assert!(database.table_exists("agent_sessions").await.unwrap());
}
#[tokio::test]
async fn migrates_v2_tool_calls_to_v3_without_losing_data() {
    let database = Database::connect(":memory:").await.unwrap();
    sqlx::raw_sql(BASELINE_SQL).execute(&database.pool).await.unwrap();
    sqlx::query(
        "CREATE TABLE soloops_schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            adopted INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&database.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO soloops_schema_migrations
         (version, name, checksum, applied_at, adopted)
         VALUES (1, 'phase_zero', 'test', 0, 0), (2, 'agent_runtime', 'test', 0, 0)",
    )
    .execute(&database.pool)
    .await
    .unwrap();
    sqlx::raw_sql(AGENT_RUNTIME_SQL)
        .execute(&database.pool)
        .await
        .unwrap();

    let (_owner, task) = initialized_runtime(&database).await;
    insert_read_only_tool_call(&database, &task.latest_run_id, "call-from-v2").await;
    database.migrate().await.unwrap();

    let call = database
        .pending_tool_calls(&task.latest_run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|call| call.summary.call_id == "call-from-v2")
        .unwrap();
    assert_eq!(call.summary.name, "workspace.search");
    assert!(call.recovery.is_none());
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM soloops_schema_migrations WHERE version = 3")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(migration_count, 1);
    let managed_migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM soloops_schema_migrations WHERE version = 5")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(managed_migration_count, 1);
    let approval_preview_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('tool_calls') WHERE name = 'approval_preview_json'",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(approval_preview_column, 1);
}
#[tokio::test]
async fn migrates_and_preserves_ip_notification_recipient_state() {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    database.migrate().await.unwrap();
    let owner = database.create_owner("owner", "hash").await.unwrap();
    database
        .update_ip_notification_settings(
            &owner.id,
            true,
            &["first@example.com".into(), "second@example.com".into()],
        )
        .await
        .unwrap();
    database
        .record_public_ipv4(&owner.id, "203.0.113.10", 100)
        .await
        .unwrap();
    database
        .record_ip_notification_success(&owner.id, "first@example.com", "203.0.113.10", 101)
        .await
        .unwrap();
    database
        .update_ip_notification_settings(
            &owner.id,
            true,
            &["first@example.com".into(), "new@example.com".into()],
        )
        .await
        .unwrap();

    let settings = database.get_ip_notification_settings(&owner.id).await.unwrap();
    assert!(settings.enabled);
    assert_eq!(settings.current_ipv4.as_deref(), Some("203.0.113.10"));
    assert_eq!(settings.recipients.len(), 2);
    let first = settings
        .recipients
        .iter()
        .find(|item| item.email == "first@example.com")
        .unwrap();
    assert_eq!(first.last_notified_ipv4.as_deref(), Some("203.0.113.10"));
    let new = settings
        .recipients
        .iter()
        .find(|item| item.email == "new@example.com")
        .unwrap();
    assert!(new.last_notified_ipv4.is_none());
    assert!(
        !settings
            .recipients
            .iter()
            .any(|item| item.email == "second@example.com")
    );
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM soloops_schema_migrations WHERE version = 4")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(migration_count, 1);
}
