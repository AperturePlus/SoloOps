use super::*;

impl Database {
    pub async fn create_task_with_run(
        &self,
        owner_id: &str,
        input: &CreateTaskRequest,
    ) -> Result<TaskSummary, StorageError> {
        let task_id = Uuid::new_v4().to_string();
        let run_id = Uuid::new_v4().to_string();
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO tasks (id, title, goal, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&task_id)
        .bind(&input.title)
        .bind(&input.goal)
        .bind(owner_id)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO runs
             (id, task_id, status, status_reason, lease_owner, lease_expires_at,
              created_at, started_at, finished_at)
             VALUES (?, ?, 'queued', NULL, NULL, NULL, ?, NULL, NULL)",
        )
        .bind(&run_id)
        .bind(&task_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        insert_event(
            &mut transaction,
            &run_id,
            EventType::RunCreated,
            json!({ "taskId": task_id, "status": "queued" }),
            now,
        )
        .await?;
        insert_audit_on_connection(
            &mut transaction,
            AuditEntry {
                actor_type: "owner",
                actor_id: Some(owner_id.to_owned()),
                action: "task.create",
                object_type: Some("task"),
                object_id: Some(task_id.clone()),
                outcome: "success",
                context: json!({ "runId": run_id }),
            },
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(TaskSummary {
            id: task_id,
            title: input.title.clone(),
            goal: input.goal.clone(),
            status: RunStatus::Queued,
            latest_run_id: run_id,
            created_at: now,
        })
    }

    pub async fn list_tasks(&self) -> Result<Vec<TaskSummary>, StorageError> {
        let rows = sqlx::query(
            "SELECT tasks.id, tasks.title, tasks.goal, runs.status,
                    runs.id AS latest_run_id, tasks.created_at
             FROM tasks
             JOIN runs ON runs.id = (
               SELECT r.id FROM runs r
               WHERE r.task_id = tasks.id
               ORDER BY r.created_at DESC, r.id DESC LIMIT 1
             )
             ORDER BY tasks.created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(task_from_row).collect()
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Option<TaskSummary>, StorageError> {
        let row = sqlx::query(
            "SELECT tasks.id, tasks.title, tasks.goal, runs.status,
                    runs.id AS latest_run_id, tasks.created_at
             FROM tasks
             JOIN runs ON runs.id = (
               SELECT r.id FROM runs r
               WHERE r.task_id = tasks.id
               ORDER BY r.created_at DESC, r.id DESC LIMIT 1
             )
             WHERE tasks.id = ?",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(task_from_row).transpose()
    }

    pub async fn get_run(&self, run_id: &str) -> Result<Option<RunDetail>, StorageError> {
        let row = sqlx::query(
            "SELECT id, task_id, status, status_reason, created_at, started_at, finished_at
             FROM runs WHERE id = ?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(run_from_row).transpose()
    }

    pub async fn claim_next_run(
        &self,
        worker_id: &str,
        lease_ms: i64,
    ) -> Result<Option<RunDetail>, StorageError> {
        let candidate_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM runs WHERE status = 'queued' ORDER BY created_at ASC LIMIT 1")
                .fetch_optional(&self.pool)
                .await?;
        let Some(candidate_id) = candidate_id else {
            return Ok(None);
        };

        let mut connection = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *connection).await?;
        let result = async {
            let row = sqlx::query(
                "SELECT id, task_id, status, status_reason, created_at, started_at, finished_at
                 FROM runs WHERE id = ? AND status = 'queued'",
            )
            .bind(&candidate_id)
            .fetch_optional(&mut *connection)
            .await?;
            let Some(row) = row else {
                return Ok(None);
            };
            let run = run_from_row(&row)?;
            let now = now_ms();
            let updated = sqlx::query(
                "UPDATE runs
                 SET status = 'leased', lease_owner = ?, lease_expires_at = ?,
                     started_at = COALESCE(started_at, ?)
                 WHERE id = ? AND status = 'queued'",
            )
            .bind(worker_id)
            .bind(now + lease_ms)
            .bind(now)
            .bind(&run.id)
            .execute(&mut *connection)
            .await?;
            if updated.rows_affected() != 1 {
                return Ok(None);
            }
            insert_event_on_connection(
                &mut connection,
                &run.id,
                EventType::RunStatusChanged,
                json!({ "from": "queued", "to": "leased", "workerId": worker_id }),
                now,
            )
            .await?;
            let row = sqlx::query(
                "SELECT id, task_id, status, status_reason, created_at, started_at, finished_at
                 FROM runs WHERE id = ?",
            )
            .bind(&run.id)
            .fetch_one(&mut *connection)
            .await?;
            Ok(Some(run_from_row(&row)?))
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

    pub async fn transition_run(
        &self,
        run_id: &str,
        to: RunStatus,
        reason: Option<&str>,
    ) -> Result<RunDetail, StorageError> {
        let mut connection = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *connection).await?;
        let result = transition_run_on_connection(&mut connection, run_id, to, reason, now_ms()).await;
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

    pub async fn list_events(
        &self,
        after: i64,
        run_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        let after = after.max(0);
        let limit = limit.clamp(1, 500);
        let rows = if let Some(run_id) = run_id {
            sqlx::query(
                "SELECT sequence, id, run_id, type, payload, created_at
                 FROM events
                 WHERE sequence > ? AND run_id = ?
                 ORDER BY sequence ASC LIMIT ?",
            )
            .bind(after)
            .bind(run_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT sequence, id, run_id, type, payload, created_at
                 FROM events
                 WHERE sequence > ?
                 ORDER BY sequence ASC LIMIT ?",
            )
            .bind(after)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(event_from_row).collect()
    }

    pub async fn write_audit(&self, entry: AuditEntry) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO audit_logs
             (id, actor_type, actor_id, action, object_type, object_id, outcome, context, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(entry.actor_type)
        .bind(entry.actor_id)
        .bind(entry.action)
        .bind(entry.object_type)
        .bind(entry.object_id)
        .bind(entry.outcome)
        .bind(entry.context.to_string())
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn audit_count(&self) -> Result<i64, StorageError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
            .fetch_one(&self.pool)
            .await?)
    }
}

pub(crate) async fn transition_run_on_connection(
    connection: &mut SqliteConnection,
    run_id: &str,
    to: RunStatus,
    reason: Option<&str>,
    now: i64,
) -> Result<RunDetail, StorageError> {
    let row = sqlx::query(
        "SELECT id, task_id, status, status_reason, created_at, started_at, finished_at
         FROM runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| StorageError::RunNotFound(run_id.to_owned()))?;
    let current = run_from_row(&row)?;
    if !current.status.can_transition_to(to) {
        return Err(StorageError::InvalidTransition {
            from: current.status,
            to,
        });
    }
    let finished_at = to.is_terminal().then_some(now);
    let release_lease = i64::from(
        to.is_terminal()
            || matches!(
                to,
                RunStatus::WaitingForApproval | RunStatus::RetryScheduled | RunStatus::NeedsRecovery
            ),
    );
    let changed = sqlx::query(
        "UPDATE runs
         SET status = ?, status_reason = ?,
             lease_owner = CASE WHEN ? = 1 THEN NULL ELSE lease_owner END,
             lease_expires_at = CASE WHEN ? = 1 THEN NULL ELSE lease_expires_at END,
             finished_at = COALESCE(?, finished_at)
         WHERE id = ? AND status = ?",
    )
    .bind(to.as_str())
    .bind(reason)
    .bind(release_lease)
    .bind(release_lease)
    .bind(finished_at)
    .bind(run_id)
    .bind(current.status.as_str())
    .execute(&mut *connection)
    .await?;
    if changed.rows_affected() != 1 {
        let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&mut *connection)
            .await?;
        let from = RunStatus::from_str(&status).map_err(|_| StorageError::InvalidRunStatus(status))?;
        return Err(StorageError::InvalidTransition { from, to });
    }
    insert_event_on_connection(
        connection,
        run_id,
        EventType::RunStatusChanged,
        json!({ "from": current.status, "to": to, "reason": reason }),
        now,
    )
    .await?;
    let row = sqlx::query(
        "SELECT id, task_id, status, status_reason, created_at, started_at, finished_at
         FROM runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_one(&mut *connection)
    .await?;
    run_from_row(&row)
}

fn task_from_row(row: &SqliteRow) -> Result<TaskSummary, StorageError> {
    let status_text: String = row.get("status");
    let status =
        RunStatus::from_str(&status_text).map_err(|_| StorageError::InvalidRunStatus(status_text.clone()))?;
    Ok(TaskSummary {
        id: row.get("id"),
        title: row.get("title"),
        goal: row.get("goal"),
        status,
        latest_run_id: row.get("latest_run_id"),
        created_at: row.get("created_at"),
    })
}

fn run_from_row(row: &SqliteRow) -> Result<RunDetail, StorageError> {
    let status_text: String = row.get("status");
    let status =
        RunStatus::from_str(&status_text).map_err(|_| StorageError::InvalidRunStatus(status_text.clone()))?;
    Ok(RunDetail {
        id: row.get("id"),
        task_id: row.get("task_id"),
        status,
        status_reason: row.get("status_reason"),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    })
}

fn event_from_row(row: &SqliteRow) -> Result<EventEnvelope, StorageError> {
    let event_text: String = row.get("type");
    let event_type =
        EventType::from_str(&event_text).map_err(|_| StorageError::InvalidEventType(event_text.clone()))?;
    let sequence: i64 = row.get("sequence");
    let event_id: String = row.get("id");
    let payload: String = row.get("payload");
    let payload = serde_json::from_str(&payload).map_err(|source| StorageError::InvalidEventPayload {
        event_id: event_id.clone(),
        sequence,
        source,
    })?;
    Ok(EventEnvelope {
        sequence,
        id: event_id,
        run_id: row.get("run_id"),
        event_type,
        payload,
        created_at: row.get("created_at"),
    })
}

pub(crate) async fn insert_event(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    run_id: &str,
    event_type: EventType,
    payload: Value,
    created_at: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO events (id, run_id, type, payload, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(run_id)
    .bind(event_type.as_str())
    .bind(payload.to_string())
    .bind(created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn insert_event_on_connection(
    connection: &mut SqliteConnection,
    run_id: &str,
    event_type: EventType,
    payload: Value,
    created_at: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO events (id, run_id, type, payload, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(run_id)
    .bind(event_type.as_str())
    .bind(payload.to_string())
    .bind(created_at)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

pub(crate) async fn insert_audit_on_connection(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    entry: AuditEntry,
    created_at: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO audit_logs
         (id, actor_type, actor_id, action, object_type, object_id, outcome, context, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(entry.actor_type)
    .bind(entry.actor_id)
    .bind(entry.action)
    .bind(entry.object_type)
    .bind(entry.object_id)
    .bind(entry.outcome)
    .bind(entry.context.to_string())
    .bind(created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
