mod error;
mod event;
mod requests;
mod run;
mod runtime;
mod security;

pub use error::{ApiErrorBody, ApiErrorResponse, ValidationError};
pub use event::{EventEnvelope, EventType, ParseEventTypeError};
pub use requests::{
    CreateTaskRequest, IpNotificationRecipientStatus, IpNotificationSettings, LoginRequest, ModelSettings,
    Owner, SessionResponse, SmtpSettings, TestIpNotificationResponse, TestModelSettingsResponse,
    TestSmtpDeliveryRequest, TestSmtpDeliveryResponse, UpdateIpNotificationSettingsRequest,
    UpdateModelSettingsRequest, UpdateSmtpSettingsRequest,
};
pub use run::{ParseRunStatusError, RUN_STATUSES, RunStatus};
pub use runtime::{
    AgentPlan, ApprovalDecision, ApprovalDecisionRequest, BudgetSnapshot, EvidenceSummary, FinalReport,
    PlanStep, PlanStepStatus, PolicyDecision, RunDetail, RuntimeCheckpoint, RuntimeSnapshot, TaskSummary,
    ToolCallStatus, ToolCallSummary, ToolRisk, UsageSnapshot,
};
pub use security::{
    SshAccessReport, SshAuthorizedKeyEntry, SshAuthorizedKeysFile, SshKeyFileRole, SshMachineSummary,
};

#[cfg(test)]
mod tests;
