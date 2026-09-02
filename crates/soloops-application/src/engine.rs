mod execution;
mod report;

use report::{FinishRequest, budget_exhausted, policy_for, project_journal, remaining_budget, render_report};

use crate::tools::{
    WorkspaceWriteRecovery, managed_deploy_tool_output, normalized_workspace_write_path,
    prepare_workspace_write, workspace_temp_prefix,
};
use crate::{
    HostManagedDeployOutput, HostdClient, ModelProvider, ModelRequest, ProviderErrorCategory, RuntimeConfig,
    Tool, ToolContext, ToolError, ToolOutput, ToolRegistry,
};

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_domain::{
    AgentPlan, BudgetSnapshot, FinalReport, PlanStepStatus, PolicyDecision, RunStatus, RuntimeCheckpoint,
    ToolCallStatus, ToolRisk, UsageSnapshot,
};
use soloops_storage::{
    Database, ManagedDeploymentOperation, NewAgentItem, NewEvidence, NewToolCall, PersistModelResponse,
    RuntimeSessionConfig, RuntimeToolCall, StorageError, now_ms,
};
use thiserror::Error;
use tokio::task::JoinHandle;
use uuid::Uuid;

const PROMPT_VERSION: &str = "agent-runtime-v1";

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("runtime invariant failed: {0}")]
    Invariant(String),
    #[error("runtime I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[async_trait]
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Default)]
pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        now_ms()
    }
}

#[derive(Clone)]
pub struct RuntimeEngine {
    database: Database,
    provider: Arc<dyn ModelProvider>,
    registry: ToolRegistry,
    config: RuntimeConfig,
    clock: Arc<dyn Clock>,
}

impl RuntimeEngine {
    pub fn new(
        database: Database,
        provider: Arc<dyn ModelProvider>,
        config: RuntimeConfig,
    ) -> Result<Self, RuntimeError> {
        let host_executor = config
            .hostd_socket
            .clone()
            .map(|path| Arc::new(HostdClient::new(path)) as Arc<dyn crate::HostExecutor>);
        Self::new_with_host_executor(database, provider, config, host_executor)
    }

    pub fn new_with_host_executor(
        database: Database,
        provider: Arc<dyn ModelProvider>,
        config: RuntimeConfig,
        host_executor: Option<Arc<dyn crate::HostExecutor>>,
    ) -> Result<Self, RuntimeError> {
        let registry = ToolRegistry::standard(
            host_executor,
            config.sandbox_enabled,
            config.managed_deploy_enabled,
        )?;
        Ok(Self {
            database,
            provider,
            registry,
            config,
            clock: Arc::new(SystemClock),
        })
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub async fn run_once(&self, worker_id: &str) -> Result<Option<String>, RuntimeError> {
        self.database.promote_due_retries().await?;
        self.database.recover_expired_runs().await?;
        self.database.recover_safe_runs().await?;
        let run_id = if let Some(run_id) = self
            .database
            .claim_approved_run(worker_id, self.config.lease_ms)
            .await?
        {
            run_id
        } else if let Some(run) = self
            .database
            .claim_next_run(worker_id, self.config.lease_ms)
            .await?
        {
            run.id
        } else {
            return Ok(None);
        };

        let workspace = self.config.workspace_root.join(&run_id);
        let artifacts = self.config.artifact_root.join(&run_id);
        tokio::fs::create_dir_all(&workspace).await?;
        tokio::fs::create_dir_all(&artifacts).await?;
        self.database
            .initialize_runtime(
                &run_id,
                &RuntimeSessionConfig {
                    provider: "openai_compatible_chat_completions".to_owned(),
                    model: self.config.model_name.clone(),
                    prompt_version: PROMPT_VERSION.to_owned(),
                    workspace_path: workspace.to_string_lossy().into_owned(),
                    artifact_path: artifacts.to_string_lossy().into_owned(),
                    budget: self.config.budget.clone(),
                },
            )
            .await?;

        let lease_lost = Arc::new(AtomicBool::new(false));
        let renewer = self.spawn_lease_renewer(run_id.clone(), worker_id.to_owned(), lease_lost.clone());
        let result = self.execute_run(&run_id, lease_lost).await;
        renewer.abort();
        if let Err(error) = result {
            let lost_state_race = matches!(
                &error,
                RuntimeError::Storage(
                    StorageError::InvalidTransition { .. } | StorageError::ToolCallStateConflict { .. }
                )
            );
            if !(lost_state_race && self.is_cancelled(&run_id).await?) {
                return Err(error);
            }
        }
        Ok(Some(run_id))
    }

    fn spawn_lease_renewer(
        &self,
        run_id: String,
        worker_id: String,
        lease_lost: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        let database = self.database.clone();
        let renew_ms = self.config.lease_renew_ms;
        let lease_ms = self.config.lease_ms;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(renew_ms));
            interval.tick().await;
            loop {
                interval.tick().await;
                match database.renew_lease(&run_id, &worker_id, lease_ms).await {
                    Ok(true) => {}
                    Ok(false) | Err(_) => {
                        lease_lost.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        })
    }

    async fn execute_run(&self, run_id: &str, lease_lost: Arc<AtomicBool>) -> Result<(), RuntimeError> {
        loop {
            if lease_lost.load(Ordering::Acquire) || self.is_cancelled(run_id).await? {
                return Ok(());
            }
            let state = self
                .database
                .runtime_execution_state(run_id)
                .await?
                .ok_or_else(|| RuntimeError::Invariant("runtime session disappeared".to_owned()))?;
            if state.consecutive_no_progress >= 3 {
                self.database
                    .transition_run(
                        run_id,
                        RunStatus::Blocked,
                        Some("Model made no progress for three consecutive turns"),
                    )
                    .await?;
                return Ok(());
            }
            if budget_exhausted(&state.snapshot.budget, &state.snapshot.usage) {
                self.database
                    .transition_run(run_id, RunStatus::Blocked, Some("budget_exhausted"))
                    .await?;
                return Ok(());
            }

            let pending = self.database.pending_tool_calls(run_id).await?;
            if !pending.is_empty() {
                if self
                    .execute_pending_tools(
                        run_id,
                        &state.workspace_path,
                        &state.artifact_path,
                        pending,
                        lease_lost.clone(),
                    )
                    .await?
                {
                    return Ok(());
                }
                continue;
            }

            if self
                .call_model(
                    run_id,
                    &state.snapshot.plan,
                    &state.snapshot.budget,
                    &state.snapshot.usage,
                    &state.snapshot.evidence,
                )
                .await?
            {
                return Ok(());
            }
        }
    }

    async fn call_model(
        &self,
        run_id: &str,
        plan: &AgentPlan,
        budget: &BudgetSnapshot,
        current_usage: &UsageSnapshot,
        evidence: &[soloops_domain::EvidenceSummary],
    ) -> Result<bool, RuntimeError> {
        let goal = self
            .database
            .get_run_goal(run_id)
            .await?
            .ok_or_else(|| RuntimeError::Invariant("run goal disappeared".to_owned()))?;
        let logical_request_key = format!("{run_id}:turn:{}", current_usage.model_turns + 1);
        let attempt = self
            .database
            .next_model_attempt(run_id, &logical_request_key)
            .await?;
        let attempt_id = self
            .database
            .prepare_model_attempt(run_id, &logical_request_key, attempt)
            .await?;
        let max_context_bytes = remaining_budget(budget, current_usage)
            .max_input_tokens
            .saturating_mul(4)
            .min(256 * 1024) as usize;
        let request = ModelRequest {
            logical_request_key,
            attempt_key: attempt_id.clone(),
            goal,
            plan: plan.clone(),
            journal: project_journal(
                self.database.load_agent_items(run_id, 60).await?,
                max_context_bytes,
            ),
            evidence: evidence.to_vec(),
            tools: self.registry.descriptors(),
            remaining_budget: remaining_budget(budget, current_usage),
        };
        let response = match self.provider.complete(request).await {
            Ok(response) => response,
            Err(error) => {
                if self.is_cancelled(run_id).await? {
                    return Ok(true);
                }
                let category = error.category.as_str();
                if error.category.retryable() && attempt < 3 {
                    let fallback = 1u64 << (attempt.saturating_sub(1) as u32);
                    let delay = error
                        .retry_after
                        .unwrap_or(Duration::from_secs(fallback))
                        .min(Duration::from_secs(60));
                    self.database
                        .schedule_model_retry(
                            run_id,
                            &attempt_id,
                            category,
                            self.clock.now_ms() + delay.as_millis() as i64,
                        )
                        .await?;
                    return Ok(true);
                }
                if error.category == ProviderErrorCategory::Authentication
                    || (error.category.retryable() && attempt >= 3)
                {
                    self.database
                        .record_model_error(run_id, &attempt_id, category, &error.message)
                        .await?;
                    self.database
                        .transition_run(run_id, RunStatus::Failed, Some(&format!("model_{category}")))
                        .await?;
                    return Ok(true);
                }
                let errors = self
                    .database
                    .record_model_error(run_id, &attempt_id, category, &error.message)
                    .await?;
                if errors >= 3 {
                    self.database
                        .transition_run(run_id, RunStatus::Blocked, Some(&format!("model_{category}")))
                        .await?;
                    return Ok(true);
                }
                return Ok(false);
            }
        };

        if self.is_cancelled(run_id).await? {
            return Ok(true);
        }

        let mut usage = current_usage.clone();
        usage.model_turns = usage.model_turns.saturating_add(1);
        usage.input_tokens = usage.input_tokens.saturating_add(response.usage.input_tokens);
        usage.output_tokens = usage.output_tokens.saturating_add(response.usage.output_tokens);
        usage.cached_input_tokens = usage
            .cached_input_tokens
            .saturating_add(response.usage.cached_input_tokens);
        usage.cache_write_input_tokens = usage
            .cache_write_input_tokens
            .saturating_add(response.usage.cache_write_input_tokens);
        usage.tool_calls = usage.tool_calls.saturating_add(response.tool_calls.len() as u32);
        // `elapsed_ms` stays the active-execution time observed on the state read;
        // the storage layer recomputes it from model-attempt and tool-call windows.
        let mut items = vec![NewAgentItem {
            kind: "model_response".to_owned(),
            payload: json!({"stopReason": response.stop_reason, "toolCallIds": response.tool_calls.iter().map(|call| &call.call_id).collect::<Vec<_>>() }),
        }];
        if let Some(message) = &response.assistant_message {
            items.push(NewAgentItem {
                kind: "assistant_message".to_owned(),
                payload: json!({"text": message}),
            });
        }
        let calls = response
            .tool_calls
            .iter()
            .enumerate()
            .map(|(ordinal, call)| {
                let descriptor = self.registry.get(&call.name).map(|tool| tool.descriptor());
                let risk = descriptor
                    .as_ref()
                    .map_or(ToolRisk::Privileged, |descriptor| descriptor.risk);
                NewToolCall {
                    call_id: if call.call_id.is_empty() {
                        Uuid::new_v4().to_string()
                    } else {
                        call.call_id.clone()
                    },
                    ordinal: ordinal as i64,
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                    risk,
                    policy: policy_for(risk),
                }
            })
            .collect::<Vec<_>>();
        // Defense against stop_reason=length: a truncated assistant message can carry
        // salvaged tool call arguments that pass JSON validation but are incomplete.
        // Never execute them; fail the whole batch as model-visible errors so the next
        // turn re-emits the calls. Three consecutive truncated turns count as no
        // progress and let the run block instead of burning the budget.
        let truncated_by_length = !calls.is_empty()
            && response
                .stop_reason
                .as_deref()
                .is_some_and(|reason| reason.eq_ignore_ascii_case("length"));
        let made_progress = !calls.is_empty() && !truncated_by_length;
        self.database
            .persist_model_response(
                run_id,
                PersistModelResponse {
                    attempt_id: &attempt_id,
                    provider_request_id: response.provider_request_id.as_deref(),
                    items: &items,
                    calls: &calls,
                    usage: &usage,
                    made_progress,
                },
            )
            .await?;
        if truncated_by_length {
            for call in &calls {
                self.database
                    .fail_tool_call_before_start(
                        run_id,
                        &call.call_id,
                        "truncated_response",
                        "The assistant message was truncated by the output token limit \
                         (stop_reason=length); its tool calls were discarded without execution. \
                         Re-issue the tool calls.",
                    )
                    .await?;
            }
        }
        Ok(false)
    }
}
