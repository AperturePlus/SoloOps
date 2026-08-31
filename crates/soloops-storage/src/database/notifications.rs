use super::*;

impl Database {
    pub async fn get_ip_notification_settings(
        &self,
        owner_id: &str,
    ) -> Result<IpNotificationRecord, StorageError> {
        let settings = sqlx::query(
            "SELECT enabled, current_ipv4, last_checked_at, last_changed_at
             FROM ip_notification_settings WHERE owner_id = ?",
        )
        .bind(owner_id)
        .fetch_optional(&self.pool)
        .await?;
        let recipients = sqlx::query(
            "SELECT email, last_notified_ipv4, last_notified_at, last_attempt_at, last_error
             FROM ip_notification_recipients WHERE owner_id = ? ORDER BY email COLLATE NOCASE",
        )
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| IpNotificationRecipientRecord {
            email: row.get("email"),
            last_notified_ipv4: row.get("last_notified_ipv4"),
            last_notified_at: row.get("last_notified_at"),
            last_attempt_at: row.get("last_attempt_at"),
            last_error: row.get("last_error"),
        })
        .collect();
        Ok(match settings {
            Some(row) => IpNotificationRecord {
                owner_id: owner_id.to_owned(),
                enabled: row.get::<i64, _>("enabled") != 0,
                current_ipv4: row.get("current_ipv4"),
                last_checked_at: row.get("last_checked_at"),
                last_changed_at: row.get("last_changed_at"),
                recipients,
            },
            None => IpNotificationRecord {
                owner_id: owner_id.to_owned(),
                enabled: false,
                current_ipv4: None,
                last_checked_at: None,
                last_changed_at: None,
                recipients,
            },
        })
    }

    pub async fn enabled_ip_notification_settings(&self) -> Result<Vec<IpNotificationRecord>, StorageError> {
        let owner_ids: Vec<String> = sqlx::query_scalar(
            "SELECT owner_id FROM ip_notification_settings WHERE enabled = 1 ORDER BY owner_id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut settings = Vec::with_capacity(owner_ids.len());
        for owner_id in owner_ids {
            settings.push(self.get_ip_notification_settings(&owner_id).await?);
        }
        Ok(settings)
    }

    pub async fn update_ip_notification_settings(
        &self,
        owner_id: &str,
        enabled: bool,
        recipients: &[String],
    ) -> Result<IpNotificationRecord, StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO ip_notification_settings (owner_id, enabled, updated_at)
             VALUES (?, ?, ?)
             ON CONFLICT(owner_id) DO UPDATE SET enabled = excluded.enabled, updated_at = excluded.updated_at",
        )
        .bind(owner_id)
        .bind(i64::from(enabled))
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        for email in recipients {
            sqlx::query(
                "INSERT INTO ip_notification_recipients (owner_id, email, created_at)
                 VALUES (?, ?, ?) ON CONFLICT(owner_id, email) DO NOTHING",
            )
            .bind(owner_id)
            .bind(email)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        if recipients.is_empty() {
            sqlx::query("DELETE FROM ip_notification_recipients WHERE owner_id = ?")
                .bind(owner_id)
                .execute(&mut *transaction)
                .await?;
        } else {
            let placeholders = std::iter::repeat_n("?", recipients.len())
                .collect::<Vec<_>>()
                .join(",");
            let query = format!(
                "DELETE FROM ip_notification_recipients WHERE owner_id = ? AND email NOT IN ({placeholders})"
            );
            let mut query = sqlx::query(&query).bind(owner_id);
            for email in recipients {
                query = query.bind(email);
            }
            query.execute(&mut *transaction).await?;
        }
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(owner_id.to_owned()),
                action: "settings.ip_notifications.update",
                object_type: Some("ip_notification_settings"),
                object_id: Some(owner_id.to_owned()),
                outcome: "success",
                context: json!({ "enabled": enabled, "recipientCount": recipients.len() }),
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        self.get_ip_notification_settings(owner_id).await
    }

    pub async fn record_public_ipv4(
        &self,
        owner_id: &str,
        ipv4: &str,
        checked_at: i64,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE ip_notification_settings
             SET current_ipv4 = ?, last_checked_at = ?,
                 last_changed_at = CASE WHEN current_ipv4 IS NULL OR current_ipv4 <> ? THEN ? ELSE last_changed_at END
             WHERE owner_id = ?",
        )
        .bind(ipv4)
        .bind(checked_at)
        .bind(ipv4)
        .bind(checked_at)
        .bind(owner_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_ip_notification_success(
        &self,
        owner_id: &str,
        email: &str,
        ipv4: &str,
        attempted_at: i64,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE ip_notification_recipients
             SET last_notified_ipv4 = ?, last_notified_at = ?, last_attempt_at = ?, last_error = NULL
             WHERE owner_id = ? AND email = ?",
        )
        .bind(ipv4)
        .bind(attempted_at)
        .bind(attempted_at)
        .bind(owner_id)
        .bind(email)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_ip_notification_failure(
        &self,
        owner_id: &str,
        email: &str,
        attempted_at: i64,
        error: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE ip_notification_recipients SET last_attempt_at = ?, last_error = ?
             WHERE owner_id = ? AND email = ?",
        )
        .bind(attempted_at)
        .bind(error)
        .bind(owner_id)
        .bind(email)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
