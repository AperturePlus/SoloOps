use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::{Database, StorageError, now_ms};

#[derive(Debug, Clone, PartialEq)]
pub struct ManagedDeploymentRevision {
    pub id: String,
    pub project_id: String,
    pub run_id: String,
    pub plan_call_id: String,
    pub previous_revision_id: Option<String>,
    pub proposal_sha256: String,
    pub compose_sha256: String,
    pub caddy_sha256: String,
    pub source: Value,
    pub preview: Value,
    pub bundle_path: String,
    pub status: String,
    pub health: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManagedDeploymentOperation {
    pub call_id: String,
    pub run_id: String,
    pub project_id: String,
    pub action: String,
    pub revision_id: String,
    pub previous_revision_id: Option<String>,
    pub phase: String,
    pub result: Option<Value>,
    pub error_category: Option<String>,
    pub lease_token: Option<String>,
    pub lease_expires_at: Option<i64>,
    pub recovery_attempts: i64,
    pub last_recovery_error: Option<String>,
    pub finished_at: Option<i64>,
}

pub struct NewManagedDeploymentRevision<'a> {
    pub project_id: &'a str,
    pub run_id: &'a str,
    pub plan_call_id: &'a str,
    pub proposal_sha256: &'a str,
    pub compose_sha256: &'a str,
    pub caddy_sha256: &'a str,
    pub source: &'a Value,
    pub preview: &'a Value,
    pub bundle_path: &'a str,
}

impl Database {
    pub async fn create_managed_deployment_revision(
        &self,
        input: NewManagedDeploymentRevision<'_>,
    ) -> Result<ManagedDeploymentRevision, StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO managed_deployments (project_id, current_revision_id, updated_at)
             VALUES (?, NULL, ?) ON CONFLICT(project_id) DO NOTHING",
        )
        .bind(input.project_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let previous_revision_id: Option<String> =
            sqlx::query_scalar("SELECT current_revision_id FROM managed_deployments WHERE project_id = ?")
                .bind(input.project_id)
                .fetch_one(&mut *transaction)
                .await?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO managed_deployment_revisions
             (id, project_id, run_id, plan_call_id, previous_revision_id, proposal_sha256,
              compose_sha256, caddy_sha256, source_json, preview_json, bundle_path, status,
              health_json, created_at, activated_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'proposed', NULL, ?, NULL, NULL)
             ON CONFLICT(plan_call_id) DO NOTHING",
        )
        .bind(&id)
        .bind(input.project_id)
        .bind(input.run_id)
        .bind(input.plan_call_id)
        .bind(previous_revision_id)
        .bind(input.proposal_sha256)
        .bind(input.compose_sha256)
        .bind(input.caddy_sha256)
        .bind(input.source.to_string())
        .bind(input.preview.to_string())
        .bind(input.bundle_path)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let revision = self
            .managed_deployment_revision_by_plan_call(input.plan_call_id)
            .await?
            .ok_or_else(|| StorageError::RunNotFound(input.plan_call_id.to_owned()))?;
        if revision.project_id != input.project_id
            || revision.run_id != input.run_id
            || revision.proposal_sha256 != input.proposal_sha256
            || revision.compose_sha256 != input.compose_sha256
            || revision.caddy_sha256 != input.caddy_sha256
            || revision.source != *input.source
            || revision.preview != *input.preview
            || revision.bundle_path != input.bundle_path
        {
            return Err(StorageError::ManagedDeploymentConflict(format!(
                "plan call {} was retried with different proposal content",
                input.plan_call_id
            )));
        }
        Ok(revision)
    }

    pub async fn managed_deployment_revision(
        &self,
        revision_id: &str,
    ) -> Result<Option<ManagedDeploymentRevision>, StorageError> {
        let row = sqlx::query("SELECT * FROM managed_deployment_revisions WHERE id = ?")
            .bind(revision_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(revision_from_row).transpose()
    }

    pub async fn managed_deployment_revision_by_sha256(
        &self,
        proposal_sha256: &str,
    ) -> Result<Option<ManagedDeploymentRevision>, StorageError> {
        let row = sqlx::query("SELECT * FROM managed_deployment_revisions WHERE proposal_sha256 = ?")
            .bind(proposal_sha256)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(revision_from_row).transpose()
    }

    pub async fn managed_deployment_revision_by_plan_call(
        &self,
        call_id: &str,
    ) -> Result<Option<ManagedDeploymentRevision>, StorageError> {
        let row = sqlx::query("SELECT * FROM managed_deployment_revisions WHERE plan_call_id = ?")
            .bind(call_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(revision_from_row).transpose()
    }

    pub async fn current_managed_deployment_revision(
        &self,
        project_id: &str,
    ) -> Result<Option<ManagedDeploymentRevision>, StorageError> {
        let row = sqlx::query(
            "SELECT revisions.* FROM managed_deployments deployments
             JOIN managed_deployment_revisions revisions ON revisions.id = deployments.current_revision_id
             WHERE deployments.project_id = ?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(revision_from_row).transpose()
    }

    pub async fn set_tool_call_approval_preview(
        &self,
        run_id: &str,
        call_id: &str,
        preview: &Value,
    ) -> Result<(), StorageError> {
        let changed = sqlx::query(
            "UPDATE tool_calls SET approval_preview_json = ?
             WHERE run_id = ? AND call_id = ? AND status IN ('pending', 'waiting_for_approval')",
        )
        .bind(preview.to_string())
        .bind(run_id)
        .bind(call_id)
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::ToolCallStateConflict {
                run_id: run_id.to_owned(),
                call_id: call_id.to_owned(),
            });
        }
        Ok(())
    }

    pub async fn begin_managed_deployment_operation(
        &self,
        call_id: &str,
        run_id: &str,
        project_id: &str,
        action: &str,
        revision_id: &str,
        previous_revision_id: Option<&str>,
        lease_token: &str,
        lease_ms: i64,
    ) -> Result<ManagedDeploymentOperation, StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        if let Some(row) = sqlx::query("SELECT * FROM managed_deployment_operations WHERE call_id = ?")
            .bind(call_id)
            .fetch_optional(&mut *transaction)
            .await?
        {
            let mut existing = operation_from_row(&row)?;
            validate_operation_retry(
                existing.clone(),
                run_id,
                project_id,
                action,
                revision_id,
                previous_revision_id,
            )?;
            let now = now_ms();
            if existing.finished_at.is_none()
                && existing.phase == "prepared"
                && (existing.lease_token.as_deref() == Some(lease_token)
                    || existing.lease_expires_at.is_none_or(|expires| expires < now))
            {
                sqlx::query(
                    "UPDATE managed_deployment_operations
                     SET lease_token = ?, lease_expires_at = ?, updated_at = ?
                     WHERE call_id = ? AND phase = 'prepared' AND finished_at IS NULL",
                )
                .bind(lease_token)
                .bind(now.saturating_add(lease_ms))
                .bind(now)
                .bind(call_id)
                .execute(&mut *transaction)
                .await?;
                existing.lease_token = Some(lease_token.to_owned());
                existing.lease_expires_at = Some(now.saturating_add(lease_ms));
            } else if existing.finished_at.is_none() && existing.lease_token.as_deref() != Some(lease_token) {
                return Err(StorageError::ManagedDeploymentConflict(format!(
                    "operation call {call_id} is leased by another executor"
                )));
            }
            transaction.commit().await?;
            return Ok(existing);
        }
        let active_call: Option<String> = sqlx::query_scalar(
            "SELECT call_id FROM managed_deployment_operations
             WHERE project_id = ? AND finished_at IS NULL LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(active_call) = active_call {
            return Err(StorageError::ManagedDeploymentConflict(format!(
                "project {project_id} already has unfinished operation {active_call}"
            )));
        }
        sqlx::query(
            "INSERT INTO managed_deployment_operations
             (call_id, run_id, project_id, action, revision_id, previous_revision_id, phase,
              result_json, error_category, lease_token, lease_expires_at, recovery_attempts,
              last_recovery_error, created_at, updated_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, 'prepared', NULL, NULL, ?, ?, 0, NULL, ?, ?, NULL)",
        )
        .bind(call_id)
        .bind(run_id)
        .bind(project_id)
        .bind(action)
        .bind(revision_id)
        .bind(previous_revision_id)
        .bind(lease_token)
        .bind(now.saturating_add(lease_ms))
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT * FROM managed_deployment_operations WHERE call_id = ?")
            .bind(call_id)
            .fetch_one(&mut *transaction)
            .await?;
        let operation = operation_from_row(&row)?;
        transaction.commit().await?;
        validate_operation_retry(
            operation,
            run_id,
            project_id,
            action,
            revision_id,
            previous_revision_id,
        )
    }

    pub async fn managed_deployment_operation(
        &self,
        call_id: &str,
    ) -> Result<Option<ManagedDeploymentOperation>, StorageError> {
        let row = sqlx::query("SELECT * FROM managed_deployment_operations WHERE call_id = ?")
            .bind(call_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(operation_from_row).transpose()
    }

    pub async fn unfinished_managed_deployment_operations(
        &self,
    ) -> Result<Vec<ManagedDeploymentOperation>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM managed_deployment_operations WHERE finished_at IS NULL ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(operation_from_row).collect()
    }

    pub async fn renew_managed_deployment_lease(
        &self,
        call_id: &str,
        lease_token: &str,
        lease_ms: i64,
    ) -> Result<bool, StorageError> {
        let now = now_ms();
        Ok(sqlx::query(
            "UPDATE managed_deployment_operations
             SET lease_expires_at = ?, updated_at = ?
             WHERE call_id = ? AND lease_token = ? AND lease_expires_at >= ? AND finished_at IS NULL",
        )
        .bind(now.saturating_add(lease_ms))
        .bind(now)
        .bind(call_id)
        .bind(lease_token)
        .bind(now)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn claim_expired_managed_deployment_operation(
        &self,
        lease_token: &str,
        lease_ms: i64,
    ) -> Result<Option<ManagedDeploymentOperation>, StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let call_id: Option<String> = sqlx::query_scalar(
            "SELECT call_id FROM managed_deployment_operations
             WHERE finished_at IS NULL AND (lease_expires_at IS NULL OR lease_expires_at < ?)
             ORDER BY created_at ASC LIMIT 1",
        )
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(call_id) = call_id else {
            transaction.commit().await?;
            return Ok(None);
        };
        let changed = sqlx::query(
            "UPDATE managed_deployment_operations
             SET lease_token = ?, lease_expires_at = ?, recovery_attempts = recovery_attempts + 1,
                 last_recovery_error = NULL, updated_at = ?
             WHERE call_id = ? AND finished_at IS NULL
               AND (lease_expires_at IS NULL OR lease_expires_at < ?)",
        )
        .bind(lease_token)
        .bind(now.saturating_add(lease_ms))
        .bind(now)
        .bind(&call_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() != 1 {
            transaction.commit().await?;
            return Ok(None);
        }
        let row = sqlx::query("SELECT * FROM managed_deployment_operations WHERE call_id = ?")
            .bind(&call_id)
            .fetch_one(&mut *transaction)
            .await?;
        let operation = operation_from_row(&row)?;
        transaction.commit().await?;
        Ok(Some(operation))
    }

    pub async fn record_managed_deployment_recovery_failure(
        &self,
        call_id: &str,
        lease_token: &str,
        message: &str,
        retry_after_ms: i64,
    ) -> Result<bool, StorageError> {
        let now = now_ms();
        Ok(sqlx::query(
            "UPDATE managed_deployment_operations
             SET last_recovery_error = ?, lease_expires_at = ?, updated_at = ?
             WHERE call_id = ? AND lease_token = ? AND finished_at IS NULL",
        )
        .bind(message)
        .bind(now.saturating_add(retry_after_ms))
        .bind(now)
        .bind(call_id)
        .bind(lease_token)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn update_managed_deployment_phase(
        &self,
        call_id: &str,
        lease_token: &str,
        expected: &str,
        next: &str,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query(
            "UPDATE managed_deployment_operations SET phase = ?, updated_at = ?
             WHERE call_id = ? AND lease_token = ? AND lease_expires_at >= ?
               AND phase = ? AND finished_at IS NULL",
        )
        .bind(next)
        .bind(now_ms())
        .bind(call_id)
        .bind(lease_token)
        .bind(now_ms())
        .bind(expected)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn finish_managed_deployment_operation(
        &self,
        call_id: &str,
        lease_token: &str,
        result: &Value,
        health: &Value,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT project_id, action, revision_id, previous_revision_id
             FROM managed_deployment_operations
             WHERE call_id = ? AND lease_token = ? AND lease_expires_at >= ? AND finished_at IS NULL",
        )
        .bind(call_id)
        .bind(lease_token)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?;
        let row = row.ok_or_else(|| {
            StorageError::ManagedDeploymentConflict(format!(
                "operation {call_id} lease was lost before completion"
            ))
        })?;
        let project_id: String = row.get("project_id");
        let action: String = row.get("action");
        let revision_id: String = row.get("revision_id");
        let previous_revision_id: Option<String> = row.get("previous_revision_id");
        if action == "apply" {
            if let Some(previous) = previous_revision_id {
                sqlx::query(
                    "UPDATE managed_deployment_revisions SET status = 'superseded', finished_at = ?
                     WHERE id = ? AND status = 'active'",
                )
                .bind(now)
                .bind(previous)
                .execute(&mut *transaction)
                .await?;
            }
        } else {
            let current: Option<String> = sqlx::query_scalar(
                "SELECT current_revision_id FROM managed_deployments WHERE project_id = ?",
            )
            .bind(&project_id)
            .fetch_one(&mut *transaction)
            .await?;
            if let Some(current) = current {
                sqlx::query(
                    "UPDATE managed_deployment_revisions SET status = 'rolled_back', finished_at = ? WHERE id = ?",
                )
                .bind(now)
                .bind(current)
                .execute(&mut *transaction)
                .await?;
            }
        }
        sqlx::query(
            "UPDATE managed_deployment_revisions
             SET status = 'active', health_json = ?, activated_at = ?, finished_at = NULL WHERE id = ?",
        )
        .bind(health.to_string())
        .bind(now)
        .bind(&revision_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE managed_deployments SET current_revision_id = ?, updated_at = ? WHERE project_id = ?",
        )
        .bind(&revision_id)
        .bind(now)
        .bind(&project_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE managed_deployment_operations
             SET phase = 'completed', result_json = ?, lease_token = NULL, lease_expires_at = NULL,
                 updated_at = ?, finished_at = ? WHERE call_id = ? AND lease_token = ?",
        )
        .bind(result.to_string())
        .bind(now)
        .bind(now)
        .bind(call_id)
        .bind(lease_token)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn fail_managed_deployment_operation(
        &self,
        call_id: &str,
        lease_token: &str,
        category: &str,
        result: &Value,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let revision_id: Option<String> = sqlx::query_scalar(
            "SELECT revision_id FROM managed_deployment_operations
                 WHERE call_id = ? AND lease_token = ? AND lease_expires_at >= ? AND finished_at IS NULL",
        )
        .bind(call_id)
        .bind(lease_token)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?;
        let revision_id = revision_id.ok_or_else(|| {
            StorageError::ManagedDeploymentConflict(format!(
                "operation {call_id} lease was lost before failure commit"
            ))
        })?;
        sqlx::query(
            "UPDATE managed_deployment_revisions SET status = 'failed', finished_at = ?
             WHERE id = ? AND status = 'proposed'",
        )
        .bind(now)
        .bind(revision_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE managed_deployment_operations
             SET phase = 'failed', result_json = ?, error_category = ?, lease_token = NULL,
                 lease_expires_at = NULL, updated_at = ?, finished_at = ?
             WHERE call_id = ? AND lease_token = ? AND finished_at IS NULL",
        )
        .bind(result.to_string())
        .bind(category)
        .bind(now)
        .bind(now)
        .bind(call_id)
        .bind(lease_token)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn validate_operation_retry(
    operation: ManagedDeploymentOperation,
    run_id: &str,
    project_id: &str,
    action: &str,
    revision_id: &str,
    previous_revision_id: Option<&str>,
) -> Result<ManagedDeploymentOperation, StorageError> {
    if operation.run_id != run_id
        || operation.project_id != project_id
        || operation.action != action
        || operation.revision_id != revision_id
        || operation.previous_revision_id.as_deref() != previous_revision_id
    {
        return Err(StorageError::ManagedDeploymentConflict(format!(
            "operation call {} was retried with different arguments",
            operation.call_id
        )));
    }
    Ok(operation)
}

fn revision_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ManagedDeploymentRevision, StorageError> {
    Ok(ManagedDeploymentRevision {
        id: row.get("id"),
        project_id: row.get("project_id"),
        run_id: row.get("run_id"),
        plan_call_id: row.get("plan_call_id"),
        previous_revision_id: row.get("previous_revision_id"),
        proposal_sha256: row.get("proposal_sha256"),
        compose_sha256: row.get("compose_sha256"),
        caddy_sha256: row.get("caddy_sha256"),
        source: decode_value("managed_deployment_revisions.source_json", row.get("source_json"))?,
        preview: decode_value(
            "managed_deployment_revisions.preview_json",
            row.get("preview_json"),
        )?,
        bundle_path: row.get("bundle_path"),
        status: row.get("status"),
        health: row
            .get::<Option<String>, _>("health_json")
            .map(|value| decode_value("managed_deployment_revisions.health_json", value))
            .transpose()?,
    })
}

fn operation_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ManagedDeploymentOperation, StorageError> {
    Ok(ManagedDeploymentOperation {
        call_id: row.get("call_id"),
        run_id: row.get("run_id"),
        project_id: row.get("project_id"),
        action: row.get("action"),
        revision_id: row.get("revision_id"),
        previous_revision_id: row.get("previous_revision_id"),
        phase: row.get("phase"),
        result: row
            .get::<Option<String>, _>("result_json")
            .map(|value| decode_value("managed_deployment_operations.result_json", value))
            .transpose()?,
        error_category: row.get("error_category"),
        lease_token: row.get("lease_token"),
        lease_expires_at: row.get("lease_expires_at"),
        recovery_attempts: row.get("recovery_attempts"),
        last_recovery_error: row.get("last_recovery_error"),
        finished_at: row.get("finished_at"),
    })
}

fn decode_value(field: &'static str, encoded: String) -> Result<Value, StorageError> {
    serde_json::from_str(&encoded).map_err(|source| StorageError::InvalidRuntimeJson { field, source })
}
