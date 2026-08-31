use crate::{RunStatus, ValidationError};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub status: RunStatus,
    pub latest_run_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub id: String,
    pub task_id: String,
    pub status: RunStatus,
    pub status_reason: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCheckpoint {
    Preparing,
    CallingModel,
    ExecutingTools,
    ValidatingCompletion,
    Reporting,
    WaitingForApproval,
    RetryScheduled,
    Recovering,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub status: PlanStepStatus,
    #[serde(default = "default_true")]
    pub required: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlan {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSnapshot {
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_duration_ms: u64,
    pub max_tool_duration_ms: u64,
    pub max_tool_output_bytes: usize,
    pub max_workspace_bytes: u64,
}

impl Default for BudgetSnapshot {
    fn default() -> Self {
        Self {
            max_model_turns: 20,
            max_tool_calls: 50,
            max_input_tokens: 200_000,
            max_output_tokens: 50_000,
            max_duration_ms: 30 * 60 * 1000,
            max_tool_duration_ms: 600_000,
            max_tool_output_bytes: 64 * 1024,
            max_workspace_bytes: 100 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub model_turns: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    ReadOnly,
    WorkspaceWrite,
    Process,
    Network,
    Privileged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Deny,
    RequireApproval,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    WaitingForApproval,
    Running,
    Completed,
    Failed,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallSummary {
    pub call_id: String,
    pub name: String,
    pub arguments_sha256: String,
    pub approval_preview: Option<serde_json::Value>,
    pub risk: ToolRisk,
    pub policy: PolicyDecision,
    pub status: ToolCallStatus,
    pub result_summary: Option<String>,
    pub error_category: Option<String>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSummary {
    pub id: String,
    pub tool_call_id: String,
    pub kind: String,
    pub summary: String,
    pub artifact_ref: Option<String>,
    pub content_sha256: Option<String>,
    pub workspace_revision: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinalReport {
    pub outcome: String,
    pub summary: String,
    pub completed: Vec<String>,
    pub incomplete: Vec<String>,
    pub risks: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub rollback: Option<String>,
    pub usage: UsageSnapshot,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub run_id: String,
    pub checkpoint: RuntimeCheckpoint,
    pub plan: AgentPlan,
    pub budget: BudgetSnapshot,
    pub usage: UsageSnapshot,
    pub workspace_revision: i64,
    pub tool_calls: Vec<ToolCallSummary>,
    pub evidence: Vec<EvidenceSummary>,
    pub report: Option<FinalReport>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionRequest {
    pub decision: ApprovalDecision,
    pub reason: Option<String>,
}

impl ApprovalDecisionRequest {
    pub fn normalize(mut self) -> Result<Self, ValidationError> {
        self.reason = self
            .reason
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if self
            .reason
            .as_ref()
            .is_some_and(|value| value.chars().count() > 1000)
        {
            return Err(ValidationError::new(
                "reason",
                "must contain at most 1000 characters",
            ));
        }
        Ok(self)
    }
}
