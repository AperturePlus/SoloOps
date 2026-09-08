use crate::ValidationError;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub id: String,
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub owner: Owner,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IpNotificationRecipientStatus {
    pub email: String,
    pub last_notified_ipv4: Option<String>,
    pub last_notified_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IpNotificationSettings {
    pub enabled: bool,
    pub smtp_configured: bool,
    pub current_ipv4: Option<String>,
    pub last_checked_at: Option<i64>,
    pub last_changed_at: Option<i64>,
    pub recipients: Vec<IpNotificationRecipientStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIpNotificationSettingsRequest {
    pub enabled: bool,
    pub recipients: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestIpNotificationResponse {
    pub sent_count: usize,
    pub failed_recipients: Vec<String>,
}

/// Effective SMTP transport as returned to the Owner. The credential is
/// never serialized; only whether one is stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SmtpSettings {
    pub configured: bool,
    /// `database` when saved from the WebUI, `environment` when falling back
    /// to `SOLOOPS_SMTP_*` variables, or `None` when unconfigured.
    pub source: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub security: Option<String>,
    pub from: Option<String>,
    pub username: Option<String>,
    pub password_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSmtpSettingsRequest {
    pub host: String,
    pub port: u16,
    pub security: String,
    pub from: String,
    #[serde(default)]
    pub username: Option<String>,
    /// Absent keeps the stored credential, an empty string clears it, and a
    /// non-empty string replaces it.
    #[serde(default)]
    pub password: Option<String>,
}

impl UpdateSmtpSettingsRequest {
    pub fn normalize(self) -> Result<Self, ValidationError> {
        let host = self.host.trim().to_owned();
        if host.is_empty() || host.chars().count() > 253 {
            return Err(ValidationError::new(
                "host",
                "must contain between 1 and 253 characters",
            ));
        }
        if !(1..=65535).contains(&self.port) {
            return Err(ValidationError::new("port", "must be a valid TCP port"));
        }
        let security = self.security.trim().to_ascii_lowercase();
        if security != "tls" && security != "starttls" {
            return Err(ValidationError::new("security", "must be tls or starttls"));
        }
        let from = self.from.trim().to_owned();
        if from.is_empty() || from.chars().count() > 320 || !from.contains('@') {
            return Err(ValidationError::new("from", "must be a valid sender mailbox"));
        }
        let username = match self.username {
            Some(value) => {
                let value = value.trim().to_owned();
                if value.is_empty() {
                    None
                } else if value.chars().count() > 320 {
                    return Err(ValidationError::new(
                        "username",
                        "must contain at most 320 characters",
                    ));
                } else {
                    Some(value)
                }
            }
            None => None,
        };
        let password = match self.password {
            Some(value) if !value.is_empty() && value.chars().count() > 1024 => {
                return Err(ValidationError::new(
                    "password",
                    "must contain at most 1024 characters",
                ));
            }
            other => other,
        };
        Ok(Self {
            host,
            port: self.port,
            security,
            from,
            username,
            password,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestSmtpDeliveryRequest {
    pub recipient: String,
}

impl TestSmtpDeliveryRequest {
    pub fn normalize(self) -> Result<Self, ValidationError> {
        let recipient = self.recipient.trim().to_ascii_lowercase();
        if recipient.is_empty() || recipient.chars().count() > 254 || !recipient.contains('@') {
            return Err(ValidationError::new("recipient", "must be a valid email address"));
        }
        Ok(Self { recipient })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestSmtpDeliveryResponse {
    pub delivered: bool,
    pub error: Option<String>,
}

/// Effective LLM API settings as returned to the Owner. The API key is never
/// serialized; only whether one is stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub configured: bool,
    /// `database` when saved from the WebUI, `environment` when falling back
    /// to `SOLOOPS_MODEL_*` variables, or `None` when unconfigured.
    pub source: Option<String>,
    pub base_url: Option<String>,
    pub model_name: Option<String>,
    pub api_key_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModelSettingsRequest {
    pub base_url: String,
    pub model_name: String,
    /// Absent keeps the stored credential, an empty string clears it (for
    /// endpoints that authenticate nothing), and a non-empty string replaces it.
    #[serde(default)]
    pub api_key: Option<String>,
}

impl UpdateModelSettingsRequest {
    pub fn normalize(self) -> Result<Self, ValidationError> {
        let base_url = self.base_url.trim().to_owned();
        let valid_scheme = base_url.starts_with("http://") || base_url.starts_with("https://");
        if base_url.is_empty() || base_url.chars().count() > 2048 || !valid_scheme {
            return Err(ValidationError::new(
                "baseUrl",
                "must be a valid http(s) base URL, for example https://api.openai.com/v1",
            ));
        }
        let model_name = self.model_name.trim().to_owned();
        if model_name.is_empty() || model_name.chars().count() > 200 {
            return Err(ValidationError::new(
                "modelName",
                "must contain between 1 and 200 characters",
            ));
        }
        let api_key = match self.api_key {
            Some(value) if !value.is_empty() && value.chars().count() > 4096 => {
                return Err(ValidationError::new(
                    "apiKey",
                    "must contain at most 4096 characters",
                ));
            }
            other => other,
        };
        Ok(Self {
            base_url,
            model_name,
            api_key,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestModelSettingsResponse {
    pub responded: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    pub title: String,
    pub goal: String,
}

impl CreateTaskRequest {
    pub fn normalize(self) -> Result<Self, ValidationError> {
        let title = self.title.trim().to_owned();
        let goal = self.goal.trim().to_owned();
        if title.is_empty() || title.chars().count() > 160 {
            return Err(ValidationError::new(
                "title",
                "must contain between 1 and 160 characters",
            ));
        }
        if goal.is_empty() || goal.chars().count() > 20_000 {
            return Err(ValidationError::new(
                "goal",
                "must contain between 1 and 20000 characters",
            ));
        }
        Ok(Self { title, goal })
    }
}

impl LoginRequest {
    pub fn normalize(self) -> Result<Self, ValidationError> {
        let username = self.username.trim().to_lowercase();
        if username.is_empty() || username.chars().count() > 64 {
            return Err(ValidationError::new(
                "username",
                "must contain between 1 and 64 characters",
            ));
        }
        if self.password.is_empty() || self.password.chars().count() > 1024 {
            return Err(ValidationError::new(
                "password",
                "must contain between 1 and 1024 characters",
            ));
        }
        Ok(Self {
            username,
            password: self.password,
        })
    }
}
