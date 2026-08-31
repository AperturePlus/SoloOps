use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const RUN_STATUSES: &[RunStatus] = &[
    RunStatus::Draft,
    RunStatus::Queued,
    RunStatus::Leased,
    RunStatus::Planning,
    RunStatus::Running,
    RunStatus::WaitingForApproval,
    RunStatus::Paused,
    RunStatus::Verifying,
    RunStatus::Reporting,
    RunStatus::RetryScheduled,
    RunStatus::Blocked,
    RunStatus::NeedsRecovery,
    RunStatus::Succeeded,
    RunStatus::Failed,
    RunStatus::Cancelled,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Draft,
    Queued,
    Leased,
    Planning,
    Running,
    WaitingForApproval,
    Paused,
    Verifying,
    Reporting,
    RetryScheduled,
    Blocked,
    NeedsRecovery,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Planning => "planning",
            Self::Running => "running",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Paused => "paused",
            Self::Verifying => "verifying",
            Self::Reporting => "reporting",
            Self::RetryScheduled => "retry_scheduled",
            Self::Blocked => "blocked",
            Self::NeedsRecovery => "needs_recovery",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Blocked | Self::Succeeded | Self::Failed | Self::Cancelled
        )
    }

    pub fn can_transition_to(self, to: Self) -> bool {
        if !self.is_terminal() && to == Self::Cancelled {
            return true;
        }
        match self {
            Self::Draft => matches!(to, Self::Queued),
            Self::Queued => matches!(to, Self::Leased),
            Self::Leased => matches!(to, Self::Planning | Self::NeedsRecovery),
            Self::Planning => matches!(
                to,
                Self::Running | Self::RetryScheduled | Self::Blocked | Self::NeedsRecovery | Self::Failed
            ),
            Self::Running => matches!(
                to,
                Self::WaitingForApproval
                    | Self::Paused
                    | Self::Verifying
                    | Self::RetryScheduled
                    | Self::Blocked
                    | Self::NeedsRecovery
                    | Self::Failed
            ),
            Self::WaitingForApproval => matches!(to, Self::Running | Self::Blocked),
            Self::Paused => matches!(to, Self::Running),
            Self::Verifying => matches!(
                to,
                Self::Reporting | Self::Running | Self::Blocked | Self::NeedsRecovery | Self::Failed
            ),
            Self::Reporting => matches!(
                to,
                Self::Succeeded | Self::Failed | Self::Blocked | Self::NeedsRecovery
            ),
            Self::RetryScheduled => matches!(to, Self::Queued),
            Self::NeedsRecovery => {
                matches!(to, Self::Queued | Self::Blocked | Self::Failed)
            }
            Self::Blocked | Self::Succeeded | Self::Failed | Self::Cancelled => false,
        }
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RunStatus {
    type Err = ParseRunStatusError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        RUN_STATUSES
            .iter()
            .copied()
            .find(|status| status.as_str() == value)
            .ok_or_else(|| ParseRunStatusError(value.to_owned()))
    }
}

#[derive(Debug, Error)]
#[error("invalid run status: {0}")]
pub struct ParseRunStatusError(String);
