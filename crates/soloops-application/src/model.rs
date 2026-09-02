use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use reqwest::{StatusCode, header::RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_domain::{AgentPlan, BudgetSnapshot, EvidenceSummary, UsageSnapshot};
use soloops_storage::StoredAgentItem;
use thiserror::Error;

use crate::{SecretValue, tools::ToolDescriptor};

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub logical_request_key: String,
    pub attempt_key: String,
    pub goal: String,
    pub plan: AgentPlan,
    pub journal: Vec<StoredAgentItem>,
    pub evidence: Vec<EvidenceSummary>,
    pub tools: Vec<ToolDescriptor>,
    pub remaining_budget: BudgetSnapshot,
}

#[derive(Debug, Clone)]
pub struct ModelToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub assistant_message: Option<String>,
    pub tool_calls: Vec<ModelToolCall>,
    pub usage: UsageSnapshot,
    pub stop_reason: Option<String>,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorCategory {
    Authentication,
    RateLimited,
    Timeout,
    Server,
    Refused,
    ContextLimit,
    Protocol,
    Transport,
}

impl ProviderErrorCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Server => "server",
            Self::Refused => "refused",
            Self::ContextLimit => "context_limit",
            Self::Protocol => "protocol",
            Self::Transport => "transport",
        }
    }

    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::Timeout | Self::Server | Self::Transport
        )
    }
}

#[derive(Debug, Error)]
#[error("model provider {category:?}: {message}")]
pub struct ProviderError {
    pub category: ProviderErrorCategory,
    pub message: String,
    pub retry_after: Option<Duration>,
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError>;
}

#[derive(Clone)]
pub struct ChatCompletionsProvider {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: Arc<SecretValue>,
    prompt_cache_key_enabled: bool,
}

impl ChatCompletionsProvider {
    pub fn new(base_url: &str, model: String, api_key: SecretValue) -> Result<Self, ProviderError> {
        let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| ProviderError {
                category: ProviderErrorCategory::Transport,
                message: error.to_string(),
                retry_after: None,
            })?;
        Ok(Self {
            client,
            endpoint,
            model,
            api_key: Arc::new(api_key),
            prompt_cache_key_enabled: false,
        })
    }

    pub const fn with_prompt_cache_key(mut self, enabled: bool) -> Self {
        self.prompt_cache_key_enabled = enabled;
        self
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    tools: Vec<ChatTool>,
    tool_choice: &'static str,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptContext {
    goal: String,
    recent_journal: Vec<StoredAgentItem>,
    evidence: Vec<EvidenceSummary>,
    plan: AgentPlan,
    remaining_budget: BudgetSnapshot,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ChatTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatFunction,
}

#[derive(Serialize)]
struct ChatFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Deserialize)]
struct ChatResponse {
    id: Option<String>,
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatResponseToolCall>,
    refusal: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponseToolCall {
    id: String,
    function: ChatResponseFunction,
}

#[derive(Deserialize)]
struct ChatResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    prompt_tokens_details: Option<PromptTokenDetails>,
}

#[derive(Deserialize)]
struct PromptTokenDetails {
    cached_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
}

#[async_trait]
impl ModelProvider for ChatCompletionsProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let system = concat!(
            "You are the SoloOps execution agent. Use only the supplied tools. ",
            "Keep the persisted plan current with plan.update. Never claim success directly: ",
            "request completion with run.finish and cite current evidence IDs. ",
            "Do not emit private reasoning or invent tool results."
        );
        let context = PromptContext {
            goal: request.goal,
            recent_journal: request.journal,
            evidence: request.evidence,
            plan: request.plan,
            remaining_budget: request.remaining_budget.clone(),
        };
        let tools = request
            .tools
            .into_iter()
            .map(|descriptor| ChatTool {
                kind: "function",
                function: ChatFunction {
                    name: descriptor.name,
                    description: format!(
                        "{} Risk: {:?}. Timeout: {} ms. Output limit: {} bytes.",
                        descriptor.description,
                        descriptor.risk,
                        descriptor.timeout.as_millis(),
                        descriptor.output_limit,
                    ),
                    parameters: descriptor.input_schema,
                },
            })
            .collect::<Vec<_>>();
        let prompt_cache_key = match self.prompt_cache_key_enabled {
            true => Some(stable_prompt_cache_key(&self.model, system, &tools)?),
            false => None,
        };
        let body = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system.to_owned(),
                },
                ChatMessage {
                    role: "user",
                    content: serde_json::to_string(&context).map_err(|error| ProviderError {
                        category: ProviderErrorCategory::Protocol,
                        message: format!("prompt context is not serializable: {error}"),
                        retry_after: None,
                    })?,
                },
            ],
            tools,
            tool_choice: "auto",
            max_tokens: request.remaining_budget.max_output_tokens,
            prompt_cache_key,
        };
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(self.api_key.expose_secret())
            .header("Idempotency-Key", &request.logical_request_key)
            .header("X-Client-Request-Id", &request.attempt_key)
            .json(&body)
            .send()
            .await
            .map_err(map_transport_error)?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|seconds| Duration::from_secs(seconds.min(60)));
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            let lower = text.to_ascii_lowercase();
            let category = if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                ProviderErrorCategory::Authentication
            } else if status == StatusCode::TOO_MANY_REQUESTS {
                ProviderErrorCategory::RateLimited
            } else if status.is_server_error() {
                ProviderErrorCategory::Server
            } else if lower.contains("context") && lower.contains("length") {
                ProviderErrorCategory::ContextLimit
            } else {
                ProviderErrorCategory::Protocol
            };
            return Err(ProviderError {
                category,
                message: format!("HTTP {status}: {}", text.chars().take(500).collect::<String>()),
                retry_after,
            });
        }
        let decoded: ChatResponse = response.json().await.map_err(|error| ProviderError {
            category: ProviderErrorCategory::Protocol,
            message: format!("invalid response JSON: {error}"),
            retry_after: None,
        })?;
        let choice = decoded.choices.into_iter().next().ok_or_else(|| ProviderError {
            category: ProviderErrorCategory::Protocol,
            message: "response has no choices".to_owned(),
            retry_after: None,
        })?;
        if let Some(refusal) = choice.message.refusal {
            return Err(ProviderError {
                category: ProviderErrorCategory::Refused,
                message: refusal,
                retry_after: None,
            });
        }
        let tool_calls = choice
            .message
            .tool_calls
            .into_iter()
            .map(|call| {
                let arguments =
                    serde_json::from_str(&call.function.arguments).map_err(|error| ProviderError {
                        category: ProviderErrorCategory::Protocol,
                        message: format!("tool {} has invalid arguments: {error}", call.function.name),
                        retry_after: None,
                    })?;
                Ok(ModelToolCall {
                    call_id: call.id,
                    name: call.function.name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        let usage = decoded.usage.unwrap_or(ChatUsage {
            prompt_tokens: None,
            completion_tokens: None,
            prompt_tokens_details: None,
        });
        let prompt_details = usage.prompt_tokens_details.unwrap_or(PromptTokenDetails {
            cached_tokens: None,
            cache_write_tokens: None,
        });
        Ok(ModelResponse {
            assistant_message: choice.message.content.filter(|value| !value.trim().is_empty()),
            tool_calls,
            usage: UsageSnapshot {
                model_turns: 1,
                tool_calls: 0,
                input_tokens: usage.prompt_tokens.unwrap_or(0),
                output_tokens: usage.completion_tokens.unwrap_or(0),
                cached_input_tokens: prompt_details.cached_tokens.unwrap_or(0),
                cache_write_input_tokens: prompt_details.cache_write_tokens.unwrap_or(0),
                elapsed_ms: 0,
            },
            stop_reason: choice.finish_reason,
            provider_request_id: decoded.id.or(request_id),
        })
    }
}

fn stable_prompt_cache_key(model: &str, system: &str, tools: &[ChatTool]) -> Result<String, ProviderError> {
    let stable_prefix = serde_json::to_vec(&json!({
        "model": model,
        "system": system,
        "tools": tools,
    }))
    .map_err(|error| ProviderError {
        category: ProviderErrorCategory::Protocol,
        message: format!("prompt cache key input is not serializable: {error}"),
        retry_after: None,
    })?;
    Ok(format!("{:x}", Sha256::digest(stable_prefix)))
}

fn map_transport_error(error: reqwest::Error) -> ProviderError {
    ProviderError {
        category: if error.is_timeout() {
            ProviderErrorCategory::Timeout
        } else {
            ProviderErrorCategory::Transport
        },
        message: error.to_string(),
        retry_after: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_serializes_the_remaining_output_budget() {
        let request = ChatRequest {
            model: "test-model".to_owned(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: "auto",
            max_tokens: 321,
            prompt_cache_key: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["max_tokens"], 321);
        assert!(value.get("prompt_cache_key").is_none());

        let enabled = ChatRequest {
            model: "test-model".to_owned(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: "auto",
            max_tokens: 321,
            prompt_cache_key: Some("cache-key".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(enabled).unwrap()["prompt_cache_key"],
            "cache-key"
        );
    }

    #[test]
    fn prompt_cache_key_is_stable_and_prompt_context_orders_dynamic_fields_last() {
        let tools = vec![ChatTool {
            kind: "function",
            function: ChatFunction {
                name: "workspace.read".to_owned(),
                description: "Read".to_owned(),
                parameters: json!({"type": "object"}),
            },
        }];
        let first = stable_prompt_cache_key("test-model", "system", &tools).unwrap();
        let second = stable_prompt_cache_key("test-model", "system", &tools).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(!first.contains("run"));

        let context = PromptContext {
            goal: "goal".to_owned(),
            recent_journal: Vec::new(),
            evidence: Vec::new(),
            plan: AgentPlan::default(),
            remaining_budget: BudgetSnapshot::default(),
        };
        let encoded = serde_json::to_string(&context).unwrap();
        assert!(encoded.find("goal").unwrap() < encoded.find("recentJournal").unwrap());
        assert!(encoded.find("recentJournal").unwrap() < encoded.find("evidence").unwrap());
        assert!(encoded.find("evidence").unwrap() < encoded.find("plan").unwrap());
        assert!(encoded.find("plan").unwrap() < encoded.find("remainingBudget").unwrap());
    }

    #[test]
    fn chat_usage_accepts_cache_details_and_missing_details() {
        let with_cache: ChatUsage = serde_json::from_value(json!({
            "prompt_tokens": 2006,
            "completion_tokens": 300,
            "prompt_tokens_details": {"cached_tokens": 1920, "cache_write_tokens": 64}
        }))
        .unwrap();
        let details = with_cache.prompt_tokens_details.unwrap();
        assert_eq!(details.cached_tokens, Some(1920));
        assert_eq!(details.cache_write_tokens, Some(64));

        let without_cache: ChatUsage = serde_json::from_value(json!({
            "prompt_tokens": 10,
            "completion_tokens": 2
        }))
        .unwrap();
        assert!(without_cache.prompt_tokens_details.is_none());
    }
}
