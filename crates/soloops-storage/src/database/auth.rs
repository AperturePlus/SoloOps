use super::*;

impl Database {
    pub async fn count_users(&self) -> Result<i64, StorageError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn create_owner(&self, username: &str, password_hash: &str) -> Result<Owner, StorageError> {
        let username = username.trim().to_lowercase();
        if username.is_empty() {
            return Err(StorageError::InvalidOwnerUsername);
        }
        let mut transaction = self.pool.begin().await?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&mut *transaction)
            .await?;
        if count > 0 {
            return Err(StorageError::OwnerAlreadyExists);
        }
        let owner = Owner {
            id: Uuid::new_v4().to_string(),
            username,
        };
        sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)")
            .bind(&owner.id)
            .bind(&owner.username)
            .bind(password_hash)
            .bind(now_ms())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(owner)
    }

    pub async fn find_user_by_username(&self, username: &str) -> Result<Option<UserRecord>, StorageError> {
        let row = sqlx::query("SELECT id, username, password_hash FROM users WHERE username = ?")
            .bind(username.trim().to_lowercase())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| UserRecord {
            id: row.get("id"),
            username: row.get("username"),
            password_hash: row.get("password_hash"),
        }))
    }

    pub async fn create_session_with_audit(
        &self,
        user_id: &str,
        token_hash: &str,
        expires_at: i64,
        context: Value,
    ) -> Result<String, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO sessions
             (id, token_hash, user_id, expires_at, revoked_at, created_at, last_seen_at)
             VALUES (?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(&id)
        .bind(token_hash)
        .bind(user_id)
        .bind(expires_at)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(user_id.to_owned()),
                action: "auth.login",
                object_type: Some("session"),
                object_id: Some(id.clone()),
                outcome: "success",
                context,
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(id)
    }

    pub async fn find_session_owner(
        &self,
        token_hash: &str,
    ) -> Result<Option<AuthenticatedOwner>, StorageError> {
        let now = now_ms();
        let row = sqlx::query(
            "SELECT users.id, users.username, sessions.id AS session_id
             FROM sessions
             JOIN users ON users.id = sessions.user_id
             WHERE sessions.token_hash = ?
               AND sessions.revoked_at IS NULL
               AND sessions.expires_at > ?",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let session_id: String = row.get("session_id");
        sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id = ?")
            .bind(now)
            .bind(&session_id)
            .execute(&self.pool)
            .await?;
        Ok(Some(AuthenticatedOwner {
            owner: Owner {
                id: row.get("id"),
                username: row.get("username"),
            },
            session_id,
        }))
    }

    pub async fn revoke_session_with_audit(
        &self,
        session_id: &str,
        actor_id: &str,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ?")
            .bind(now)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(actor_id.to_owned()),
                action: "auth.logout",
                object_type: Some("session"),
                object_id: Some(session_id.to_owned()),
                outcome: "success",
                context: json!({}),
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn cleanup_expired_sessions(&self) -> Result<u64, StorageError> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ? OR revoked_at IS NOT NULL")
            .bind(now_ms())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
