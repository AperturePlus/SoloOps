use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventType {
    #[serde(rename = "run.created")]
    RunCreated,
    #[serde(rename = "run.status_changed")]
    RunStatusChanged,
    #[serde(rename = "agent.plan_updated")]
    AgentPlanUpdated,
    #[serde(rename = "agent.message")]
    AgentMessage,
    #[serde(rename = "tool.call_started")]
    ToolCallStarted,
    #[serde(rename = "tool.call_completed")]
    ToolCallCompleted,
    #[serde(rename = "tool.call_failed")]
    ToolCallFailed,
    #[serde(rename = "run.reported")]
    RunReported,
}

impl EventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunCreated => "run.created",
            Self::RunStatusChanged => "run.status_changed",
            Self::AgentPlanUpdated => "agent.plan_updated",
            Self::AgentMessage => "agent.message",
            Self::ToolCallStarted => "tool.call_started",
            Self::ToolCallCompleted => "tool.call_completed",
            Self::ToolCallFailed => "tool.call_failed",
            Self::RunReported => "run.reported",
        }
    }
}

impl FromStr for EventType {
    type Err = ParseEventTypeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "run.created" => Ok(Self::RunCreated),
            "run.status_changed" => Ok(Self::RunStatusChanged),
            "agent.plan_updated" => Ok(Self::AgentPlanUpdated),
            "agent.message" => Ok(Self::AgentMessage),
            "tool.call_started" => Ok(Self::ToolCallStarted),
            "tool.call_completed" => Ok(Self::ToolCallCompleted),
            "tool.call_failed" => Ok(Self::ToolCallFailed),
            "run.reported" => Ok(Self::RunReported),
            _ => Err(ParseEventTypeError(value.to_owned())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid event type: {0}")]
pub struct ParseEventTypeError(String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub sequence: i64,
    pub id: String,
    pub run_id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub payload: Value,
    pub created_at: i64,
}
