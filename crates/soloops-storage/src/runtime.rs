use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_domain::{
    AgentPlan, ApprovalDecision, BudgetSnapshot, EventType, EvidenceSummary, FinalReport, PolicyDecision,
    RunStatus, RuntimeCheckpoint, RuntimeSnapshot, ToolCallSummary, ToolRisk, UsageSnapshot,
};
use sqlx::Row;
use uuid::Uuid;

use super::{
    AuditEntry, Database, StorageError, insert_audit_on_connection, insert_event, now_ms,
    transition_run_on_connection,
};

#[derive(Debug, Clone)]
pub struct RuntimeSessionConfig {
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub workspace_path: String,
    pub artifact_path: String,
    pub budget: BudgetSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAgentItem {
    pub sequence: i64,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct NewAgentItem {
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct NewToolCall {
    pub call_id: String,
    pub ordinal: i64,
    pub name: String,
    pub arguments: Value,
    pub risk: ToolRisk,
    pub policy: PolicyDecision,
}

pub struct PersistModelResponse<'a> {
    pub attempt_id: &'a str,
    pub provider_request_id: Option<&'a str>,
    pub items: &'a [NewAgentItem],
    pub calls: &'a [NewToolCall],
    pub usage: &'a UsageSnapshot,
    pub made_progress: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeToolCall {
    pub summary: ToolCallSummary,
    pub arguments: Value,
    pub arguments_sha256: String,
    pub result: Option<Value>,
    pub recovery: Option<Value>,
    pub workspace_revision_before: i64,
    pub approval: Option<ApprovalDecision>,
    pub approval_arguments_sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeExecutionState {
    pub snapshot: RuntimeSnapshot,
    pub workspace_path: String,
    pub artifact_path: String,
    pub consecutive_no_progress: i64,
    pub consecutive_protocol_errors: i64,
}

#[derive(Debug, Clone)]
pub struct NewEvidence {
    pub kind: String,
    pub summary: String,
    pub artifact_ref: Option<String>,
    pub content_sha256: Option<String>,
}

fn encode<T: Serialize>(value: &T) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|source| StorageError::InvalidRuntimeJson {
        field: "serialize",
        source,
    })
}

fn decode<T: DeserializeOwned>(field: &'static str, value: &str) -> Result<T, StorageError> {
    serde_json::from_str(value).map_err(|source| StorageError::InvalidRuntimeJson { field, source })
}

fn enum_text<T: Serialize>(value: T) -> Result<String, StorageError> {
    let encoded = serde_json::to_value(value).map_err(|source| StorageError::InvalidRuntimeJson {
        field: "enum",
        source,
    })?;
    Ok(encoded.as_str().unwrap_or_default().to_owned())
}

fn parse_enum<T: DeserializeOwned>(field: &'static str, value: &str) -> Result<T, StorageError> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|source| StorageError::InvalidRuntimeJson { field, source })
}

impl Database {
    pub async fn authorize_host_tool_call(
        &self,
        run_id: &str,
        call_id: &str,
        tool_name: &str,
        arguments_sha256: &str,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1
               FROM tool_calls tc
               JOIN runs r ON r.id = tc.run_id
               JOIN tasks t ON t.id = r.task_id
               WHERE tc.run_id = ? AND tc.call_id = ? AND tc.name = ?
                 AND tc.status = 'running' AND r.status = 'running'
                 AND tc.arguments_sha256 = ?
                 AND (
                   tc.policy_decision = 'allow' OR EXISTS (
                     SELECT 1 FROM tool_approvals ta
                     WHERE ta.call_id = tc.call_id AND ta.run_id = tc.run_id
                       AND ta.arguments_sha256 = tc.arguments_sha256
                       AND ta.decision = 'approve' AND ta.owner_id = t.created_by
                   )
                 )
             )",
        )
        .bind(run_id)
        .bind(call_id)
        .bind(tool_name)
        .bind(arguments_sha256)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_run_goal(&self, run_id: &str) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT tasks.goal FROM runs JOIN tasks ON tasks.id = runs.task_id WHERE runs.id = ?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn initialize_runtime(
        &self,
        run_id: &str,
        config: &RuntimeSessionConfig,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let result = async {
            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO agent_sessions
                 (run_id, provider, model, prompt_version, workspace_path, artifact_path,
                  checkpoint, plan_json, budget_json, usage_json, workspace_revision,
                  retry_at, consecutive_no_progress, consecutive_protocol_errors, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'preparing', ?, ?, ?, 0, NULL, 0, 0, ?, ?)",
            )
            .bind(run_id)
            .bind(&config.provider)
            .bind(&config.model)
            .bind(&config.prompt_version)
            .bind(&config.workspace_path)
            .bind(&config.artifact_path)
            .bind(encode(&AgentPlan::default())?)
            .bind(encode(&config.budget)?)
            .bind(encode(&UsageSnapshot::default())?)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;

            let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
            if status == "leased" {
                transition_run_on_connection(&mut transaction, run_id, RunStatus::Planning, None, now)
                    .await?;
                if inserted.rows_affected() == 0 {
                    transition_run_on_connection(&mut transaction, run_id, RunStatus::Running, None, now)
                        .await?;
                }
            }
            if inserted.rows_affected() == 1 {
                insert_audit_on_connection(
                    &mut transaction,
                    AuditEntry {
                        actor_type: "worker",
                        actor_id: None,
                        action: "agent.start",
                        object_type: Some("run"),
                        object_id: Some(run_id.to_owned()),
                        outcome: "success",
                        context: json!({"provider": config.provider, "model": config.model}),
                    },
                    now,
                )
                .await?;
            }
            Ok(())
        }
        .await;
        match result {
            Ok(()) => transaction.commit().await.map_err(Into::into),
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    pub async fn set_runtime_checkpoint(
        &self,
        run_id: &str,
        checkpoint: RuntimeCheckpoint,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE agent_sessions SET checkpoint = ?, updated_at = ? WHERE run_id = ?")
            .bind(enum_text(checkpoint)?)
            .bind(now_ms())
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prepare_model_attempt(
        &self,
        run_id: &str,
        request_key: &str,
        attempt: i64,
    ) -> Result<String, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "UPDATE agent_sessions SET checkpoint = 'calling_model', updated_at = ? WHERE run_id = ?",
        )
        .bind(now)
        .bind(run_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO model_attempts
             (id, run_id, request_key, attempt, status, provider_request_id, error_category, started_at, completed_at)
             VALUES (?, ?, ?, ?, 'running', NULL, NULL, ?, NULL)",
        )
        .bind(&id)
        .bind(run_id)
        .bind(request_key)
        .bind(attempt)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(id)
    }

    pub async fn next_model_attempt(&self, run_id: &str, request_key: &str) -> Result<i64, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(MAX(attempt), 0) + 1 FROM model_attempts WHERE run_id = ? AND request_key = ?",
        )
        .bind(run_id)
        .bind(request_key)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn record_model_error(
        &self,
        run_id: &str,
        attempt_id: &str,
        category: &str,
        message: &str,
    ) -> Result<i64, StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "UPDATE model_attempts SET status = 'failed', error_category = ?, completed_at = ? WHERE id = ?",
        )
        .bind(category)
        .bind(now)
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        let sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_items WHERE run_id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        sqlx::query(
            "INSERT INTO agent_items (id, run_id, sequence, kind, payload, provider_request_id, created_at)
             VALUES (?, ?, ?, 'system_feedback', ?, NULL, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(run_id)
        .bind(sequence)
        .bind(json!({"error": {"category": category, "message": message.chars().take(500).collect::<String>()}}).to_string())
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let errors: i64 = sqlx::query_scalar(
            "UPDATE agent_sessions SET consecutive_protocol_errors = consecutive_protocol_errors + 1,
             updated_at = ? WHERE run_id = ? RETURNING consecutive_protocol_errors",
        )
        .bind(now)
        .bind(run_id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(errors)
    }

    pub async fn schedule_model_retry(
        &self,
        run_id: &str,
        attempt_id: &str,
        category: &str,
        retry_at: i64,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "UPDATE model_attempts SET status = 'failed', error_category = ?, completed_at = ? WHERE id = ?",
        )
        .bind(category)
        .bind(now)
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE agent_sessions SET checkpoint = 'retry_scheduled', retry_at = ?, updated_at = ? WHERE run_id = ?")
            .bind(retry_at).bind(now).bind(run_id).execute(&mut *transaction).await?;
        transition_run_on_connection(
            &mut transaction,
            run_id,
            RunStatus::RetryScheduled,
            Some(category),
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn promote_due_retries(&self) -> Result<u64, StorageError> {
        let now = now_ms();
        let run_ids = sqlx::query_scalar::<_, String>(
            "SELECT runs.id FROM runs JOIN agent_sessions ON agent_sessions.run_id = runs.id
             WHERE runs.status = 'retry_scheduled' AND agent_sessions.retry_at <= ?",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let mut count = 0;
        for run_id in run_ids {
            let mut transaction = self.pool.begin().await?;
            let due: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM runs JOIN agent_sessions ON agent_sessions.run_id = runs.id
                 WHERE runs.id = ? AND runs.status = 'retry_scheduled' AND agent_sessions.retry_at <= ?)",
            )
            .bind(&run_id)
            .bind(now)
            .fetch_one(&mut *transaction)
            .await?;
            if due {
                transition_run_on_connection(&mut transaction, &run_id, RunStatus::Queued, None, now).await?;
                sqlx::query("UPDATE agent_sessions SET checkpoint = 'preparing', retry_at = NULL, updated_at = ? WHERE run_id = ?")
                    .bind(now).bind(&run_id).execute(&mut *transaction).await?;
                count += 1;
            }
            transaction.commit().await?;
        }
        Ok(count)
    }

    pub async fn persist_model_response(
        &self,
        run_id: &str,
        response: PersistModelResponse<'_>,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let status_text: String = sqlx::query_scalar("SELECT status FROM runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&mut *transaction)
            .await?;
        let status: RunStatus = parse_enum("runs.status", &status_text)?;
        if !matches!(status, RunStatus::Planning | RunStatus::Running) {
            return Err(StorageError::InvalidTransition {
                from: status,
                to: RunStatus::Running,
            });
        }
        let mut sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) FROM agent_items WHERE run_id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        for item in response.items {
            sequence += 1;
            sqlx::query(
                "INSERT INTO agent_items (id, run_id, sequence, kind, payload, provider_request_id, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(run_id)
            .bind(sequence)
            .bind(&item.kind)
            .bind(item.payload.to_string())
            .bind(response.provider_request_id)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            if item.kind == "assistant_message" {
                let preview = item
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                insert_event(
                    &mut transaction,
                    run_id,
                    EventType::AgentMessage,
                    json!({"preview": preview.chars().take(500).collect::<String>()}),
                    now,
                )
                .await?;
            }
        }
        for call in response.calls {
            let arguments_json = call.arguments.to_string();
            let digest = format!("{:x}", Sha256::digest(arguments_json.as_bytes()));
            sqlx::query(
                "INSERT OR IGNORE INTO tool_calls
                 (call_id, run_id, item_id, ordinal, name, arguments_json, arguments_sha256,
                  risk, policy_decision, status, idempotency_key, result_summary, result_json,
                  error_category, workspace_revision_before, workspace_revision_after,
                  created_at, started_at, completed_at)
                 VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, 'pending', ?, NULL, NULL, NULL,
                         (SELECT workspace_revision FROM agent_sessions WHERE run_id = ?), NULL, ?, NULL, NULL)",
            )
            .bind(&call.call_id)
            .bind(run_id)
            .bind(call.ordinal)
            .bind(&call.name)
            .bind(arguments_json)
            .bind(digest)
            .bind(enum_text(call.risk)?)
            .bind(enum_text(call.policy)?)
            .bind(format!("{run_id}:{}", call.call_id))
            .bind(run_id)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "UPDATE model_attempts SET status = 'completed', provider_request_id = ?, completed_at = ? WHERE id = ?",
        )
        .bind(response.provider_request_id)
        .bind(now)
        .bind(response.attempt_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE agent_sessions SET usage_json = ?, checkpoint = 'executing_tools',
             consecutive_no_progress = CASE WHEN ? THEN 0 ELSE consecutive_no_progress + 1 END,
             consecutive_protocol_errors = 0, updated_at = ? WHERE run_id = ?",
        )
        .bind(encode(response.usage)?)
        .bind(response.made_progress)
        .bind(now)
        .bind(run_id)
        .execute(&mut *transaction)
        .await?;
        if status == RunStatus::Planning {
            transition_run_on_connection(&mut transaction, run_id, RunStatus::Running, None, now).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn load_agent_items(
        &self,
        run_id: &str,
        limit: i64,
    ) -> Result<Vec<StoredAgentItem>, StorageError> {
        let rows = sqlx::query(
            "SELECT sequence, kind, payload FROM agent_items WHERE run_id = ? ORDER BY sequence DESC LIMIT ?",
        )
        .bind(run_id)
        .bind(limit.clamp(1, 200))
        .fetch_all(&self.pool)
        .await?;
        let mut items = rows
            .iter()
            .map(|row| {
                let payload: String = row.get("payload");
                Ok(StoredAgentItem {
                    sequence: row.get("sequence"),
                    kind: row.get("kind"),
                    payload: decode("agent_items.payload", &payload)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        items.reverse();
        Ok(items)
    }

    pub async fn save_plan(&self, run_id: &str, plan: &AgentPlan) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_items WHERE run_id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        sqlx::query("UPDATE agent_sessions SET plan_json = ?, updated_at = ? WHERE run_id = ?")
            .bind(encode(plan)?)
            .bind(now)
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO agent_items (id, run_id, sequence, kind, payload, provider_request_id, created_at)
             VALUES (?, ?, ?, 'plan_update', ?, NULL, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(run_id)
        .bind(sequence)
        .bind(encode(plan)?)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        insert_event(
            &mut transaction,
            run_id,
            EventType::AgentPlanUpdated,
            json!({"summary": plan.summary, "steps": plan.steps.len()}),
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn pending_tool_calls(&self, run_id: &str) -> Result<Vec<RuntimeToolCall>, StorageError> {
        let rows = sqlx::query(
            "SELECT tc.*, ta.decision AS approval_decision, ta.arguments_sha256 AS approval_arguments_sha256
             FROM tool_calls tc LEFT JOIN tool_approvals ta ON ta.call_id = tc.call_id
             WHERE tc.run_id = ? AND tc.status IN ('pending', 'waiting_for_approval', 'running', 'unknown')
             ORDER BY tc.ordinal ASC, tc.created_at ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(tool_call_from_row).collect()
    }

    pub async fn wait_for_tool_approval(&self, run_id: &str, call_id: &str) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE tool_calls SET policy_decision = 'require_approval', status = 'waiting_for_approval'
             WHERE run_id = ? AND call_id = ? AND status = 'pending'",
        )
        .bind(run_id)
        .bind(call_id)
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::ToolCallStateConflict {
                run_id: run_id.to_owned(),
                call_id: call_id.to_owned(),
            });
        }
        sqlx::query(
            "UPDATE agent_sessions SET checkpoint = 'waiting_for_approval', updated_at = ? WHERE run_id = ?",
        )
        .bind(now)
        .bind(run_id)
        .execute(&mut *transaction)
        .await?;
        transition_run_on_connection(
            &mut transaction,
            run_id,
            RunStatus::WaitingForApproval,
            Some("tool_approval_required"),
            now,
        )
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "worker",
                actor_id: None,
                action: "tool.policy",
                object_type: Some("tool_call"),
                object_id: Some(call_id.to_owned()),
                outcome: "require_approval",
                context: json!({"runId": run_id}),
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn decide_tool_call(
        &self,
        run_id: &str,
        call_id: &str,
        owner_id: &str,
        decision: ApprovalDecision,
        reason: Option<&str>,
    ) -> Result<ToolCallSummary, StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT tc.*, NULL AS approval_decision, NULL AS approval_arguments_sha256 FROM tool_calls tc
             JOIN runs ON runs.id = tc.run_id
             WHERE tc.run_id = ? AND tc.call_id = ? AND tc.status = 'waiting_for_approval'
               AND runs.status = 'waiting_for_approval'",
        )
        .bind(run_id)
        .bind(call_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| StorageError::RunNotFound(format!("{run_id}:{call_id}")))?;
        let call = tool_call_from_row(&row)?;
        sqlx::query(
            "INSERT INTO tool_approvals (call_id, run_id, owner_id, arguments_sha256, decision, reason, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(call_id)
        .bind(run_id)
        .bind(owner_id)
        .bind(&call.arguments_sha256)
        .bind(enum_text(decision)?)
        .bind(reason)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(owner_id.to_owned()),
                action: "tool.approval",
                object_type: Some("tool_call"),
                object_id: Some(call_id.to_owned()),
                outcome: if decision == ApprovalDecision::Approve {
                    "approved"
                } else {
                    "denied"
                },
                context: json!({"runId": run_id, "reason": reason}),
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(call.summary)
    }

    pub async fn claim_approved_run(
        &self,
        worker_id: &str,
        lease_ms: i64,
    ) -> Result<Option<String>, StorageError> {
        // Both approval decisions are claimable: denied calls must resume so the worker can
        // persist the terminal Tool Call result instead of leaving the Run waiting forever.
        let candidate_id: Option<String> = sqlx::query_scalar(
            "SELECT runs.id FROM runs
             JOIN tool_calls ON tool_calls.run_id = runs.id AND tool_calls.status = 'waiting_for_approval'
             JOIN tool_approvals ON tool_approvals.call_id = tool_calls.call_id
             WHERE runs.status = 'waiting_for_approval'
             ORDER BY tool_approvals.created_at ASC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(candidate_id) = candidate_id else {
            return Ok(None);
        };

        let mut connection = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *connection).await?;
        let result = async {
            let run_id: Option<String> = sqlx::query_scalar(
                "SELECT runs.id FROM runs
                 JOIN tool_calls ON tool_calls.run_id = runs.id AND tool_calls.status = 'waiting_for_approval'
                 JOIN tool_approvals ON tool_approvals.call_id = tool_calls.call_id
                 WHERE runs.id = ? AND runs.status = 'waiting_for_approval'",
            )
            .bind(&candidate_id)
            .fetch_optional(&mut *connection)
            .await?;
            let Some(run_id) = run_id else { return Ok(None) };
            let now = now_ms();
            transition_run_on_connection(&mut connection, &run_id, RunStatus::Running, None, now).await?;
            sqlx::query("UPDATE runs SET lease_owner = ?, lease_expires_at = ? WHERE id = ?")
                .bind(worker_id)
                .bind(now + lease_ms)
                .bind(&run_id)
                .execute(&mut *connection)
                .await?;
            sqlx::query(
                "UPDATE agent_sessions SET checkpoint = 'executing_tools', updated_at = ? WHERE run_id = ?",
            )
            .bind(now)
            .bind(&run_id)
            .execute(&mut *connection)
            .await?;
            Ok(Some(run_id))
        }
        .await;
        match result {
            Ok(value) => {
                sqlx::query("COMMIT").execute(&mut *connection).await?;
                Ok(value)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                Err(error)
            }
        }
    }

    pub async fn start_tool_call(&self, run_id: &str, call_id: &str) -> Result<(), StorageError> {
        self.start_tool_call_with_recovery(run_id, call_id, None).await
    }

    pub async fn start_tool_call_with_recovery(
        &self,
        run_id: &str,
        call_id: &str,
        recovery: Option<&Value>,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE tool_calls SET status = 'running', recovery_json = ?, started_at = ?
             WHERE run_id = ? AND call_id = ?
               AND EXISTS (SELECT 1 FROM runs WHERE runs.id = tool_calls.run_id AND runs.status = 'running')
               AND (
               status = 'pending' OR (
                 status = 'waiting_for_approval' AND EXISTS (
                   SELECT 1 FROM tool_approvals
                   WHERE tool_approvals.call_id = tool_calls.call_id
                     AND tool_approvals.decision = 'approve'
                     AND tool_approvals.arguments_sha256 = tool_calls.arguments_sha256
                 )
               )
             )",
        )
        .bind(recovery.map(Value::to_string))
        .bind(now)
        .bind(run_id)
        .bind(call_id)
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::ToolCallStateConflict {
                run_id: run_id.to_owned(),
                call_id: call_id.to_owned(),
            });
        }
        insert_event(
            &mut transaction,
            run_id,
            EventType::ToolCallStarted,
            json!({"callId": call_id}),
            now,
        )
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "worker",
                actor_id: None,
                action: "tool.policy",
                object_type: Some("tool_call"),
                object_id: Some(call_id.to_owned()),
                outcome: "allow",
                context: json!({"runId": run_id}),
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn finish_tool_call(
        &self,
        run_id: &str,
        call_id: &str,
        result: &Value,
        summary: &str,
        evidence: Option<&NewEvidence>,
        increments_workspace: bool,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        if increments_workspace {
            sqlx::query("UPDATE agent_sessions SET workspace_revision = workspace_revision + 1, updated_at = ? WHERE run_id = ?")
                .bind(now)
                .bind(run_id)
                .execute(&mut *transaction)
                .await?;
        }
        let revision: i64 =
            sqlx::query_scalar("SELECT workspace_revision FROM agent_sessions WHERE run_id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        let changed = sqlx::query(
            "UPDATE tool_calls SET status = 'completed', result_summary = ?, result_json = ?,
             workspace_revision_after = ?, completed_at = ?
             WHERE run_id = ? AND call_id = ? AND status = 'running'
               AND EXISTS (
                 SELECT 1 FROM runs WHERE runs.id = tool_calls.run_id
                   AND runs.status IN ('running', 'verifying')
               )",
        )
        .bind(summary)
        .bind(result.to_string())
        .bind(revision)
        .bind(now)
        .bind(run_id)
        .bind(call_id)
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::ToolCallStateConflict {
                run_id: run_id.to_owned(),
                call_id: call_id.to_owned(),
            });
        }
        // Derive the evidence row and its id from the same source of truth so the
        // journal payload and the evidence insert can never disagree.
        let evidence_row = evidence.map(|evidence| (Uuid::new_v4().to_string(), evidence));
        let sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_items WHERE run_id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        sqlx::query(
            "INSERT INTO agent_items (id, run_id, sequence, kind, payload, provider_request_id, created_at)
             VALUES (?, ?, ?, 'tool_result', ?, NULL, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(run_id)
        .bind(sequence)
        .bind(
            json!({"callId": call_id, "result": result, "evidenceId": evidence_row.as_ref().map(|(id, _)| id)})
                .to_string(),
        )
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        if let Some((evidence_id, evidence)) = evidence_row {
            sqlx::query(
                "INSERT INTO evidence
                 (id, run_id, tool_call_id, kind, summary, artifact_ref, content_sha256, workspace_revision, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(evidence_id)
            .bind(run_id)
            .bind(call_id)
            .bind(&evidence.kind)
            .bind(&evidence.summary)
            .bind(&evidence.artifact_ref)
            .bind(&evidence.content_sha256)
            .bind(revision)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        insert_event(
            &mut transaction,
            run_id,
            EventType::ToolCallCompleted,
            json!({"callId": call_id, "summary": summary, "workspaceRevision": revision}),
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn fail_tool_call(
        &self,
        run_id: &str,
        call_id: &str,
        category: &str,
        summary: &str,
        denied: bool,
    ) -> Result<(), StorageError> {
        self.fail_tool_call_internal(run_id, call_id, category, summary, denied, false)
            .await
    }

    pub async fn fail_tool_call_before_start(
        &self,
        run_id: &str,
        call_id: &str,
        category: &str,
        summary: &str,
    ) -> Result<(), StorageError> {
        self.fail_tool_call_internal(run_id, call_id, category, summary, false, true)
            .await
    }

    async fn fail_tool_call_internal(
        &self,
        run_id: &str,
        call_id: &str,
        category: &str,
        summary: &str,
        denied: bool,
        before_start: bool,
    ) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let status = if denied { "denied" } else { "failed" };
        let denied = i64::from(denied);
        let allowed_status = if denied == 1 || before_start {
            "status IN ('pending', 'waiting_for_approval')"
        } else {
            "status = 'running'"
        };
        let changed = sqlx::query(&format!(
            "UPDATE tool_calls SET status = ?, result_summary = ?, error_category = ?, completed_at = ?
             WHERE run_id = ? AND call_id = ?
               AND EXISTS (
                 SELECT 1 FROM runs WHERE runs.id = tool_calls.run_id
                   AND runs.status IN ('running', 'verifying')
               )
               AND {allowed_status}"
        ))
        .bind(status)
        .bind(summary)
        .bind(category)
        .bind(now)
        .bind(run_id)
        .bind(call_id)
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::ToolCallStateConflict {
                run_id: run_id.to_owned(),
                call_id: call_id.to_owned(),
            });
        }
        let sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_items WHERE run_id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        sqlx::query(
            "INSERT INTO agent_items (id, run_id, sequence, kind, payload, provider_request_id, created_at)
             VALUES (?, ?, ?, 'tool_result', ?, NULL, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(run_id)
        .bind(sequence)
        .bind(json!({"callId": call_id, "error": {"category": category, "message": summary}}).to_string())
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        insert_event(
            &mut transaction,
            run_id,
            EventType::ToolCallFailed,
            json!({"callId": call_id, "category": category, "summary": summary}),
            now,
        )
        .await?;
        if denied == 1 {
            insert_audit_on_connection(
                &mut transaction,
                AuditEntry {
                    actor_type: "worker",
                    actor_id: None,
                    action: "tool.policy",
                    object_type: Some("tool_call"),
                    object_id: Some(call_id.to_owned()),
                    outcome: "deny",
                    context: json!({"runId": run_id, "category": category}),
                },
                now,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn save_final_report(&self, run_id: &str, report: &FinalReport) -> Result<(), StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_items WHERE run_id = ?")
                .bind(run_id)
                .fetch_one(&mut *transaction)
                .await?;
        sqlx::query("UPDATE agent_sessions SET checkpoint = 'reporting', updated_at = ? WHERE run_id = ?")
            .bind(now)
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        transition_run_on_connection(&mut transaction, run_id, RunStatus::Reporting, None, now).await?;
        sqlx::query(
            "INSERT INTO agent_items (id, run_id, sequence, kind, payload, provider_request_id, created_at)
             VALUES (?, ?, ?, 'report', ?, NULL, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(run_id)
        .bind(sequence)
        .bind(encode(report)?)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        insert_event(
            &mut transaction,
            run_id,
            EventType::RunReported,
            json!({"outcome": report.outcome, "summary": report.summary}),
            now,
        )
        .await?;
        transition_run_on_connection(&mut transaction, run_id, RunStatus::Succeeded, None, now).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn runtime_snapshot(&self, run_id: &str) -> Result<Option<RuntimeSnapshot>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let snapshot = runtime_snapshot_on_connection(&mut transaction, run_id).await?;
        transaction.commit().await?;
        Ok(snapshot)
    }

    pub async fn runtime_execution_state(
        &self,
        run_id: &str,
    ) -> Result<Option<RuntimeExecutionState>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let Some(mut snapshot) = runtime_snapshot_on_connection(&mut transaction, run_id).await? else {
            transaction.commit().await?;
            return Ok(None);
        };
        let row = sqlx::query(
            "SELECT workspace_path, artifact_path, consecutive_no_progress, consecutive_protocol_errors
             FROM agent_sessions WHERE run_id = ?",
        )
        .bind(run_id)
        .fetch_one(&mut *transaction)
        .await?;
        // Budget enforcement must observe active execution time only: time spent
        // waiting for Owner approval or in model retry backoff is not execution and
        // must not push a run over max_duration_ms.
        snapshot.usage.elapsed_ms = active_execution_ms_on_connection(&mut transaction, run_id).await?;
        let state = RuntimeExecutionState {
            snapshot,
            workspace_path: row.get("workspace_path"),
            artifact_path: row.get("artifact_path"),
            consecutive_no_progress: row.get("consecutive_no_progress"),
            consecutive_protocol_errors: row.get("consecutive_protocol_errors"),
        };
        transaction.commit().await?;
        Ok(Some(state))
    }

    pub async fn reset_tool_call_pending(&self, run_id: &str, call_id: &str) -> Result<(), StorageError> {
        let changed = sqlx::query(
            "UPDATE tool_calls SET status = 'pending', started_at = NULL
             WHERE run_id = ? AND call_id = ? AND status IN ('running', 'unknown')",
        )
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

    pub async fn renew_lease(
        &self,
        run_id: &str,
        worker_id: &str,
        lease_ms: i64,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "UPDATE runs SET lease_expires_at = ? WHERE id = ? AND lease_owner = ?
             AND status IN ('leased','planning','running','verifying','reporting')",
        )
        .bind(now_ms() + lease_ms)
        .bind(run_id)
        .bind(worker_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn cancel_run(
        &self,
        run_id: &str,
        owner_id: &str,
    ) -> Result<soloops_domain::RunDetail, StorageError> {
        let now = now_ms();
        let mut transaction = self.begin_write().await?;
        let run = transition_run_on_connection(
            &mut transaction,
            run_id,
            RunStatus::Cancelled,
            Some("Cancelled by Owner"),
            now,
        )
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(owner_id.to_owned()),
                action: "run.cancel",
                object_type: Some("run"),
                object_id: Some(run_id.to_owned()),
                outcome: "success",
                context: json!({}),
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(run)
    }

    pub async fn recover_expired_runs(&self) -> Result<u64, StorageError> {
        let now = now_ms();
        let rows = sqlx::query(
            "SELECT id, status FROM runs WHERE lease_expires_at IS NOT NULL AND lease_expires_at < ?
             AND status IN ('leased','planning','running','verifying','reporting')",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let mut recovered = 0;
        for row in rows {
            let run_id: String = row.get("id");
            let from: String = row.get("status");
            let mut transaction = self.pool.begin().await?;
            let still_expired: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM runs
                 WHERE id = ? AND status = ? AND lease_expires_at < ?)",
            )
            .bind(&run_id)
            .bind(&from)
            .bind(now)
            .fetch_one(&mut *transaction)
            .await?;
            if still_expired {
                transition_run_on_connection(
                    &mut transaction,
                    &run_id,
                    RunStatus::NeedsRecovery,
                    Some("lease_expired"),
                    now,
                )
                .await?;
                sqlx::query(
                    "UPDATE agent_sessions SET checkpoint = 'recovering', updated_at = ? WHERE run_id = ?",
                )
                .bind(now)
                .bind(&run_id)
                .execute(&mut *transaction)
                .await?;
                recovered += 1;
            }
            transaction.commit().await?;
        }
        Ok(recovered)
    }

    pub async fn recover_safe_runs(&self) -> Result<u64, StorageError> {
        let rows = sqlx::query(
            "SELECT runs.id,
                    EXISTS(SELECT 1 FROM tool_calls WHERE tool_calls.run_id = runs.id AND status IN ('running','unknown') AND risk = 'process') AS unsafe_process
             FROM runs WHERE status = 'needs_recovery'",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut count = 0;
        for row in rows {
            let run_id: String = row.get("id");
            let unsafe_process: bool = row.get("unsafe_process");
            let now = now_ms();
            let mut transaction = self.pool.begin().await?;
            if unsafe_process {
                transition_run_on_connection(
                    &mut transaction,
                    &run_id,
                    RunStatus::Blocked,
                    Some("Interrupted process has unknown side effects"),
                    now,
                )
                .await?;
            } else {
                sqlx::query("UPDATE tool_calls SET status = 'pending', started_at = NULL WHERE run_id = ? AND status IN ('running','unknown') AND risk = 'read_only'")
                    .bind(&run_id)
                    .execute(&mut *transaction)
                    .await?;
                transition_run_on_connection(
                    &mut transaction,
                    &run_id,
                    RunStatus::Queued,
                    Some("Recovered from expired lease"),
                    now,
                )
                .await?;
            }
            transaction.commit().await?;
            count += 1;
        }
        Ok(count)
    }
}

/// Accumulated wall-clock time the run has spent actively executing: the union of
/// model-attempt windows and tool-call windows. Queued time, Owner approval waits and
/// model retry backoff contribute nothing, because those rows either never started
/// (`started_at IS NULL`) or carry no in-flight window.
async fn active_execution_ms_on_connection(
    connection: &mut sqlx::SqliteConnection,
    run_id: &str,
) -> Result<u64, StorageError> {
    let now = now_ms();
    let tool_ms: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(MIN(COALESCE(completed_at, ?), ?) - started_at), 0)
         FROM tool_calls WHERE run_id = ? AND started_at IS NOT NULL",
    )
    .bind(now)
    .bind(now)
    .bind(run_id)
    .fetch_one(&mut *connection)
    .await?;
    let model_ms: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(MIN(COALESCE(completed_at, ?), ?) - started_at), 0)
         FROM model_attempts WHERE run_id = ? AND started_at IS NOT NULL",
    )
    .bind(now)
    .bind(now)
    .bind(run_id)
    .fetch_one(&mut *connection)
    .await?;
    Ok(tool_ms.saturating_add(model_ms).max(0) as u64)
}

async fn runtime_snapshot_on_connection(
    connection: &mut sqlx::SqliteConnection,
    run_id: &str,
) -> Result<Option<RuntimeSnapshot>, StorageError> {
    let row = sqlx::query(
        "SELECT checkpoint, plan_json, budget_json, usage_json, workspace_revision
             FROM agent_sessions WHERE run_id = ?",
    )
    .bind(run_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let tool_rows = sqlx::query(
            "SELECT tc.*, ta.decision AS approval_decision, ta.arguments_sha256 AS approval_arguments_sha256 FROM tool_calls tc
             LEFT JOIN tool_approvals ta ON ta.call_id = tc.call_id
             WHERE tc.run_id = ? ORDER BY tc.created_at ASC, tc.ordinal ASC",
        )
        .bind(run_id)
        .fetch_all(&mut *connection)
        .await?;
    let evidence_rows = sqlx::query(
        "SELECT id, tool_call_id, kind, summary, artifact_ref, content_sha256, workspace_revision, created_at
             FROM evidence WHERE run_id = ? ORDER BY created_at ASC",
    )
    .bind(run_id)
    .fetch_all(&mut *connection)
    .await?;
    let report_payload: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM agent_items WHERE run_id = ? AND kind = 'report' ORDER BY sequence DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(Some(RuntimeSnapshot {
        run_id: run_id.to_owned(),
        checkpoint: parse_enum("agent_sessions.checkpoint", &row.get::<String, _>("checkpoint"))?,
        plan: decode("agent_sessions.plan_json", &row.get::<String, _>("plan_json"))?,
        budget: decode("agent_sessions.budget_json", &row.get::<String, _>("budget_json"))?,
        usage: decode("agent_sessions.usage_json", &row.get::<String, _>("usage_json"))?,
        workspace_revision: row.get("workspace_revision"),
        tool_calls: tool_rows
            .iter()
            .map(tool_call_from_row)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|call| call.summary)
            .collect(),
        evidence: evidence_rows
            .iter()
            .map(|row| EvidenceSummary {
                id: row.get("id"),
                tool_call_id: row.get("tool_call_id"),
                kind: row.get("kind"),
                summary: row.get("summary"),
                artifact_ref: row.get("artifact_ref"),
                content_sha256: row.get("content_sha256"),
                workspace_revision: row.get("workspace_revision"),
                created_at: row.get("created_at"),
            })
            .collect(),
        report: report_payload
            .as_deref()
            .map(|value| decode("agent_items.report", value))
            .transpose()?,
    }))
}

fn tool_call_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<RuntimeToolCall, StorageError> {
    let policy: String = row.get("policy_decision");
    let status: String = row.get("status");
    let risk: String = row.get("risk");
    let arguments: String = row.get("arguments_json");
    let result: Option<String> = row.get("result_json");
    let recovery: Option<String> = row.get("recovery_json");
    let approval_text: Option<String> = row.try_get("approval_decision").ok().flatten();
    let approval_arguments_sha256: Option<String> = row.try_get("approval_arguments_sha256").ok().flatten();
    Ok(RuntimeToolCall {
        summary: ToolCallSummary {
            call_id: row.get("call_id"),
            name: row.get("name"),
            arguments_sha256: row.get("arguments_sha256"),
            approval_preview: row
                .get::<Option<String>, _>("approval_preview_json")
                .as_deref()
                .map(|value| decode("tool_calls.approval_preview_json", value))
                .transpose()?,
            risk: parse_enum("tool_calls.risk", &risk)?,
            policy: parse_enum("tool_calls.policy_decision", &policy)?,
            status: parse_enum("tool_calls.status", &status)?,
            result_summary: row.get("result_summary"),
            error_category: row.get("error_category"),
            started_at: row.get("started_at"),
            completed_at: row.get("completed_at"),
        },
        arguments: decode("tool_calls.arguments_json", &arguments)?,
        arguments_sha256: row.get("arguments_sha256"),
        result: result
            .as_deref()
            .map(|value| decode("tool_calls.result_json", value))
            .transpose()?,
        recovery: recovery
            .as_deref()
            .map(|value| decode("tool_calls.recovery_json", value))
            .transpose()?,
        workspace_revision_before: row.get("workspace_revision_before"),
        approval: approval_text
            .as_deref()
            .map(|value| parse_enum("tool_approvals.decision", value))
            .transpose()?,
        approval_arguments_sha256,
    })
}
