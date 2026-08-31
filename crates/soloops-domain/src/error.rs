use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
#[derive(Debug, Clone, Serialize)]
pub struct ApiErrorResponse {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiErrorResponse {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorBody {
                code: code.into(),
                message: message.into(),
                details: None,
            },
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.error.details = Some(details);
        self
    }
}

#[derive(Debug, Clone, Error, Serialize)]
#[error("{field} {message}")]
pub struct ValidationError {
    pub field: &'static str,
    pub message: &'static str,
}

impl ValidationError {
    pub const fn new(field: &'static str, message: &'static str) -> Self {
        Self { field, message }
    }
}
