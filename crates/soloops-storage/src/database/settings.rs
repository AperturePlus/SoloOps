use super::*;

impl Database {
    pub async fn get_smtp_settings(
        &self,
        owner_id: &str,
    ) -> Result<Option<SmtpSettingsRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT host, port, security, from_mailbox, username, password, updated_at
             FROM smtp_settings WHERE owner_id = ?",
        )
        .bind(owner_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| SmtpSettingsRecord {
            owner_id: owner_id.to_owned(),
            host: row.get("host"),
            port: row.get("port"),
            security: row.get("security"),
            from_mailbox: row.get("from_mailbox"),
            username: row.get("username"),
            password: row.get("password"),
            updated_at: row.get("updated_at"),
        }))
    }

    /// Saves the owner-managed SMTP transport. The audit entry deliberately
    /// excludes the password and only records that credentials are set.
    pub async fn upsert_smtp_settings(
        &self,
        record: &SmtpSettingsRecord,
    ) -> Result<SmtpSettingsRecord, StorageError> {
        if record.security != "tls" && record.security != "starttls" {
            return Err(StorageError::InvalidSmtpSecurity(record.security.clone()));
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO smtp_settings
                 (owner_id, host, port, security, from_mailbox, username, password, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(owner_id) DO UPDATE SET
                 host = excluded.host,
                 port = excluded.port,
                 security = excluded.security,
                 from_mailbox = excluded.from_mailbox,
                 username = excluded.username,
                 password = excluded.password,
                 updated_at = excluded.updated_at",
        )
        .bind(&record.owner_id)
        .bind(&record.host)
        .bind(record.port)
        .bind(&record.security)
        .bind(&record.from_mailbox)
        .bind(&record.username)
        .bind(&record.password)
        .bind(record.updated_at)
        .execute(&mut *transaction)
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(record.owner_id.clone()),
                action: "settings.smtp.update",
                object_type: Some("smtp_settings"),
                object_id: Some(record.owner_id.clone()),
                outcome: "success",
                context: json!({
                    "host": record.host,
                    "port": record.port,
                    "security": record.security,
                    "fromMailbox": record.from_mailbox,
                    "username": record.username,
                    "passwordConfigured": record.password.is_some(),
                }),
            },
            record.updated_at,
        )
        .await?;
        transaction.commit().await?;
        self.get_smtp_settings(&record.owner_id)
            .await
            .map(|record| record.expect("smtp settings were just upserted"))
    }

    /// Removes the owner-managed SMTP transport so delivery falls back to the
    /// environment configuration, if any.
    pub async fn delete_smtp_settings(&self, owner_id: &str) -> Result<bool, StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let deleted = sqlx::query("DELETE FROM smtp_settings WHERE owner_id = ?")
            .bind(owner_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            > 0;
        if deleted {
            insert_audit_on_connection(
                &mut transaction,
                AuditEntry {
                    actor_type: "owner",
                    actor_id: Some(owner_id.to_owned()),
                    action: "settings.smtp.delete",
                    object_type: Some("smtp_settings"),
                    object_id: Some(owner_id.to_owned()),
                    outcome: "success",
                    context: json!({}),
                },
                now,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(deleted)
    }
}

impl Database {
    pub async fn get_model_settings(
        &self,
        owner_id: &str,
    ) -> Result<Option<ModelSettingsRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT base_url, model_name, api_key, updated_at
             FROM model_settings WHERE owner_id = ?",
        )
        .bind(owner_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ModelSettingsRecord {
            owner_id: owner_id.to_owned(),
            base_url: row.get("base_url"),
            model_name: row.get("model_name"),
            api_key: row.get("api_key"),
            updated_at: row.get("updated_at"),
        }))
    }

    /// Saves the owner-managed LLM API settings. The audit entry deliberately
    /// excludes the API key and only records that a credential is set.
    pub async fn upsert_model_settings(
        &self,
        record: &ModelSettingsRecord,
    ) -> Result<ModelSettingsRecord, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO model_settings
                 (owner_id, base_url, model_name, api_key, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(owner_id) DO UPDATE SET
                 base_url = excluded.base_url,
                 model_name = excluded.model_name,
                 api_key = excluded.api_key,
                 updated_at = excluded.updated_at",
        )
        .bind(&record.owner_id)
        .bind(&record.base_url)
        .bind(&record.model_name)
        .bind(&record.api_key)
        .bind(record.updated_at)
        .execute(&mut *transaction)
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(record.owner_id.clone()),
                action: "settings.model.update",
                object_type: Some("model_settings"),
                object_id: Some(record.owner_id.clone()),
                outcome: "success",
                context: json!({
                    "baseUrl": record.base_url,
                    "modelName": record.model_name,
                    "apiKeyConfigured": record.api_key.is_some(),
                }),
            },
            record.updated_at,
        )
        .await?;
        transaction.commit().await?;
        self.get_model_settings(&record.owner_id)
            .await
            .map(|record| record.expect("model settings were just upserted"))
    }

    /// Removes the owner-managed LLM API settings so the agent runtime falls
    /// back to the environment configuration, if any.
    pub async fn delete_model_settings(&self, owner_id: &str) -> Result<bool, StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let deleted = sqlx::query("DELETE FROM model_settings WHERE owner_id = ?")
            .bind(owner_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            > 0;
        if deleted {
            insert_audit_on_connection(
                &mut transaction,
                AuditEntry {
                    actor_type: "owner",
                    actor_id: Some(owner_id.to_owned()),
                    action: "settings.model.delete",
                    object_type: Some("model_settings"),
                    object_id: Some(owner_id.to_owned()),
                    outcome: "success",
                    context: json!({}),
                },
                now,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(deleted)
    }
}
