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
