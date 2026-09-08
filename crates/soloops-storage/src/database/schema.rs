use super::*;

impl Database {
    pub async fn migrate(&self) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS soloops_schema_migrations (
                version INTEGER PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                checksum TEXT NOT NULL,
                applied_at INTEGER NOT NULL,
                adopted INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        let applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 1")
                .fetch_optional(&self.pool)
                .await?;
        if applied.is_none() {
            let adopted = self.table_exists("users").await?;
            if adopted {
                self.verify_phase_zero_schema().await?;
            } else {
                sqlx::raw_sql(BASELINE_SQL).execute(&self.pool).await?;
            }
            let checksum = format!("{:x}", Sha256::digest(BASELINE_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (1, 'phase_zero', ?, ?, ?)",
            )
            .bind(checksum)
            .bind(now_ms())
            .bind(i64::from(adopted))
            .execute(&self.pool)
            .await?;
        }
        let runtime_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 2")
                .fetch_optional(&self.pool)
                .await?;
        if runtime_applied.is_none() {
            sqlx::raw_sql(AGENT_RUNTIME_SQL).execute(&self.pool).await?;
            let checksum = format!("{:x}", Sha256::digest(AGENT_RUNTIME_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (2, 'agent_runtime', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&self.pool)
            .await?;
        }
        let recovery_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 3")
                .fetch_optional(&self.pool)
                .await?;
        if recovery_applied.is_none() {
            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(WORKSPACE_WRITE_RECOVERY_SQL)
                .execute(&mut *transaction)
                .await?;
            let checksum = format!("{:x}", Sha256::digest(WORKSPACE_WRITE_RECOVERY_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (3, 'workspace_write_recovery', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        let notifications_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 4")
                .fetch_optional(&self.pool)
                .await?;
        if notifications_applied.is_none() {
            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(IP_NOTIFICATIONS_SQL)
                .execute(&mut *transaction)
                .await?;
            let checksum = format!("{:x}", Sha256::digest(IP_NOTIFICATIONS_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (4, 'ip_notifications', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        let managed_deployments_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 5")
                .fetch_optional(&self.pool)
                .await?;
        if managed_deployments_applied.is_none() {
            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(MANAGED_DEPLOYMENTS_SQL)
                .execute(&mut *transaction)
                .await?;
            let checksum = format!("{:x}", Sha256::digest(MANAGED_DEPLOYMENTS_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (5, 'managed_deployments', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        let managed_deployment_leases_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 6")
                .fetch_optional(&self.pool)
                .await?;
        if managed_deployment_leases_applied.is_none() {
            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(MANAGED_DEPLOYMENT_LEASES_SQL)
                .execute(&mut *transaction)
                .await?;
            let checksum = format!("{:x}", Sha256::digest(MANAGED_DEPLOYMENT_LEASES_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (6, 'managed_deployment_leases', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        let password_rotation_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 7")
                .fetch_optional(&self.pool)
                .await?;
        if password_rotation_applied.is_none() {
            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(PASSWORD_ROTATION_SQL)
                .execute(&mut *transaction)
                .await?;
            let checksum = format!("{:x}", Sha256::digest(PASSWORD_ROTATION_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (7, 'password_rotation', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        let smtp_settings_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 8")
                .fetch_optional(&self.pool)
                .await?;
        if smtp_settings_applied.is_none() {
            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(SMTP_SETTINGS_SQL)
                .execute(&mut *transaction)
                .await?;
            let checksum = format!("{:x}", Sha256::digest(SMTP_SETTINGS_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (8, 'smtp_settings', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        let model_settings_applied: Option<i64> =
            sqlx::query_scalar("SELECT version FROM soloops_schema_migrations WHERE version = 9")
                .fetch_optional(&self.pool)
                .await?;
        if model_settings_applied.is_none() {
            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(MODEL_SETTINGS_SQL)
                .execute(&mut *transaction)
                .await?;
            let checksum = format!("{:x}", Sha256::digest(MODEL_SETTINGS_SQL.as_bytes()));
            sqlx::query(
                "INSERT INTO soloops_schema_migrations
                 (version, name, checksum, applied_at, adopted)
                 VALUES (9, 'model_settings', ?, ?, 0)",
            )
            .bind(checksum)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        self.verify_schema().await
    }

    pub async fn verify_schema(&self) -> Result<(), StorageError> {
        for table in REQUIRED_TABLES {
            if !self.table_exists(table).await? {
                return Err(StorageError::MissingTable((*table).to_owned()));
            }
        }
        for (table, required) in REQUIRED_COLUMNS {
            let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
                .fetch_all(&self.pool)
                .await?;
            for column in *required {
                if !rows.iter().any(|row| row.get::<String, _>("name") == *column) {
                    return Err(StorageError::MissingColumn {
                        table: (*table).to_owned(),
                        column: (*column).to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    async fn verify_phase_zero_schema(&self) -> Result<(), StorageError> {
        for table in &REQUIRED_TABLES[..6] {
            if !self.table_exists(table).await? {
                return Err(StorageError::MissingTable((*table).to_owned()));
            }
        }
        for (table, required) in &REQUIRED_COLUMNS[..6] {
            let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
                .fetch_all(&self.pool)
                .await?;
            for column in *required {
                if !rows.iter().any(|row| row.get::<String, _>("name") == *column) {
                    return Err(StorageError::MissingColumn {
                        table: (*table).to_owned(),
                        column: (*column).to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    pub async fn ready(&self) -> Result<(), StorageError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        self.verify_schema().await
    }

    pub(super) async fn table_exists(&self, table: &str) -> Result<bool, StorageError> {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ? LIMIT 1")
                .bind(table)
                .fetch_optional(&self.pool)
                .await?;
        Ok(found.is_some())
    }
}
