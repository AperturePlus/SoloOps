use serde::Deserialize;
use soloops_domain::{BudgetSnapshot, PolicyDecision, ToolRisk, UsageSnapshot};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FinishRequest {
    pub(super) outcome: String,
    pub(super) summary: String,
    #[serde(default)]
    pub(super) completed: Vec<String>,
    #[serde(default)]
    pub(super) incomplete: Vec<String>,
    #[serde(default)]
    pub(super) risks: Vec<String>,
    pub(super) evidence_ids: Vec<String>,
    pub(super) rollback: Option<String>,
}

pub(super) fn policy_for(risk: ToolRisk) -> PolicyDecision {
    match risk {
        ToolRisk::ReadOnly => PolicyDecision::Allow,
        ToolRisk::WorkspaceWrite | ToolRisk::Process | ToolRisk::Privileged => {
            PolicyDecision::RequireApproval
        }
        ToolRisk::Network => PolicyDecision::Deny,
    }
}

pub(super) fn budget_exhausted(budget: &BudgetSnapshot, usage: &UsageSnapshot) -> bool {
    usage.model_turns >= budget.max_model_turns
        || usage.tool_calls >= budget.max_tool_calls
        || usage.input_tokens >= budget.max_input_tokens
        || usage.output_tokens >= budget.max_output_tokens
        || usage.elapsed_ms >= budget.max_duration_ms
}

pub(super) fn remaining_budget(budget: &BudgetSnapshot, usage: &UsageSnapshot) -> BudgetSnapshot {
    BudgetSnapshot {
        max_model_turns: budget.max_model_turns.saturating_sub(usage.model_turns),
        max_tool_calls: budget.max_tool_calls.saturating_sub(usage.tool_calls),
        max_input_tokens: budget.max_input_tokens.saturating_sub(usage.input_tokens),
        max_output_tokens: budget.max_output_tokens.saturating_sub(usage.output_tokens),
        max_duration_ms: budget.max_duration_ms.saturating_sub(usage.elapsed_ms),
        max_tool_duration_ms: budget.max_tool_duration_ms,
        max_tool_output_bytes: budget.max_tool_output_bytes,
        max_workspace_bytes: budget.max_workspace_bytes,
    }
}

pub(super) fn project_journal(
    items: Vec<soloops_storage::StoredAgentItem>,
    max_bytes: usize,
) -> Vec<soloops_storage::StoredAgentItem> {
    let mut size = 0usize;
    let mut projected = Vec::new();
    for item in items.into_iter().rev() {
        let item_size = item.kind.len().saturating_add(item.payload.to_string().len());
        if !projected.is_empty() && size.saturating_add(item_size) > max_bytes {
            break;
        }
        size = size.saturating_add(item_size);
        projected.push(item);
    }
    projected.reverse();
    projected
}

pub(super) fn render_report(request: &FinishRequest, usage: &UsageSnapshot) -> String {
    fn section(title: &str, items: &[String]) -> String {
        if items.is_empty() {
            return format!("## {title}\n\nNone.\n\n");
        }
        format!(
            "## {title}\n\n{}\n\n",
            items
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
    format!(
        "# SoloOps Run Report\n\n**Outcome:** succeeded\n\n{}\n\n{}{}{}## Evidence\n\n{}\n\n## Rollback\n\n{}\n\n## Usage\n\n- Model turns: {}\n- Tool calls: {}\n- Input tokens: {}\n- Output tokens: {}\n- Cached input tokens: {}\n- Cache write input tokens: {}\n",
        request.summary,
        section("Completed", &request.completed),
        section("Incomplete", &request.incomplete),
        section("Risks", &request.risks),
        request
            .evidence_ids
            .iter()
            .map(|id| format!("- `{id}`"))
            .collect::<Vec<_>>()
            .join("\n"),
        request
            .rollback
            .as_deref()
            .unwrap_or("No rollback procedure was provided."),
        usage.model_turns,
        usage.tool_calls,
        usage.input_tokens,
        usage.output_tokens,
        usage.cached_input_tokens,
        usage.cache_write_input_tokens,
    )
}
