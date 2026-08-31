use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;
#[cfg(unix)]
use soloops_hostd_protocol::{
    HostdAction, HostdErrorCode, HostdRequestV3, HostdResponseV3, HostdResult, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES, PROTOCOL_VERSION, read_frame, write_frame,
};
#[cfg(unix)]
use uuid::Uuid;

use crate::{ToolContext, ToolError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSandboxOutput {
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
    pub image: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostManagedDeployOutput {
    pub action: String,
    pub project_id: String,
    pub status: String,
    pub revision_id: Option<String>,
    pub previous_revision_id: Option<String>,
    pub proposal_sha256: Option<String>,
    pub preview: Option<Value>,
}

#[async_trait]
pub trait HostExecutor: Send + Sync {
    async fn execute_process(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostProcessOutput, ToolError>;

    async fn execute_sandbox(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostSandboxOutput, ToolError>;

    async fn managed_deploy_plan(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        let _ = (context, arguments);
        Err(ToolError::PolicyDenied("managed deployment is disabled".into()))
    }

    async fn managed_deploy_status(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        let _ = (context, arguments);
        Err(ToolError::PolicyDenied("managed deployment is disabled".into()))
    }

    async fn managed_deploy_apply(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        let _ = (context, arguments);
        Err(ToolError::PolicyDenied("managed deployment is disabled".into()))
    }

    async fn managed_deploy_rollback(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        let _ = (context, arguments);
        Err(ToolError::PolicyDenied("managed deployment is disabled".into()))
    }
}

#[derive(Debug, Clone)]
pub struct HostdClient {
    socket_path: PathBuf,
}

impl HostdClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    async fn execute_managed(
        &self,
        context: &ToolContext,
        arguments: Value,
        expected_action: &'static str,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        #[cfg(unix)]
        {
            let mut stream = tokio::net::UnixStream::connect(&self.socket_path)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd connection failed: {error}")))?;
            let request_id = Uuid::new_v4().to_string();
            let action = match expected_action {
                "plan" => HostdAction::ManagedDeployPlan {
                    arguments,
                    max_output_bytes: context.max_output_bytes,
                },
                "status" => HostdAction::ManagedDeployStatus {
                    arguments,
                    max_output_bytes: context.max_output_bytes,
                },
                "apply" => HostdAction::ManagedDeployApply {
                    arguments,
                    max_output_bytes: context.max_output_bytes,
                },
                "rollback" => HostdAction::ManagedDeployRollback {
                    arguments,
                    max_output_bytes: context.max_output_bytes,
                },
                _ => return Err(ToolError::Execution("unknown managed deployment action".into())),
            };
            let request = HostdRequestV3 {
                protocol_version: PROTOCOL_VERSION,
                request_id: request_id.clone(),
                run_id: context.run_id.clone(),
                call_id: context.call_id.clone(),
                action,
            };
            write_frame(&mut stream, &request, MAX_REQUEST_BYTES)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd request failed: {error}")))?;
            let response: HostdResponseV3 = read_frame(&mut stream, MAX_RESPONSE_BYTES)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd response failed: {error}")))?;
            if response.protocol_version != PROTOCOL_VERSION || response.request_id != request_id {
                return Err(ToolError::Execution(
                    "hostd returned a mismatched protocol response".into(),
                ));
            }
            if let Some(error) = response.error {
                return Err(map_hostd_error(error, context.max_output_bytes));
            }
            match response
                .result
                .ok_or_else(|| ToolError::Execution("hostd response had no result".into()))?
            {
                HostdResult::ManagedDeploy(result) if result.action == expected_action => {
                    Ok(HostManagedDeployOutput {
                        action: result.action,
                        project_id: result.project_id,
                        status: result.status,
                        revision_id: result.revision_id,
                        previous_revision_id: result.previous_revision_id,
                        proposal_sha256: result.proposal_sha256,
                        preview: result.preview,
                    })
                }
                HostdResult::ManagedDeploy(_) => Err(ToolError::Execution(
                    "hostd returned a mismatched managed deployment action".into(),
                )),
                HostdResult::ProcessExec(_) | HostdResult::SandboxExec(_) => Err(ToolError::Execution(
                    "hostd returned a mismatched result type".into(),
                )),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (context, arguments, expected_action);
            Err(ToolError::PolicyDenied(format!(
                "hostd Unix socket transport is not supported on this platform ({})",
                self.socket_path.display()
            )))
        }
    }
}

#[async_trait]
impl HostExecutor for HostdClient {
    async fn execute_process(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostProcessOutput, ToolError> {
        #[cfg(unix)]
        {
            let mut stream = tokio::net::UnixStream::connect(&self.socket_path)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd connection failed: {error}")))?;
            let request_id = Uuid::new_v4().to_string();
            let request = HostdRequestV3 {
                protocol_version: PROTOCOL_VERSION,
                request_id: request_id.clone(),
                run_id: context.run_id.clone(),
                call_id: context.call_id.clone(),
                action: HostdAction::ProcessExec {
                    arguments,
                    max_output_bytes: context.max_output_bytes,
                },
            };
            write_frame(&mut stream, &request, MAX_REQUEST_BYTES)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd request failed: {error}")))?;
            let response: HostdResponseV3 = read_frame(&mut stream, MAX_RESPONSE_BYTES)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd response failed: {error}")))?;
            if response.protocol_version != PROTOCOL_VERSION || response.request_id != request_id {
                return Err(ToolError::Execution(
                    "hostd returned a mismatched protocol response".into(),
                ));
            }
            if let Some(error) = response.error {
                return Err(match error.code {
                    HostdErrorCode::InvalidRequest => ToolError::InvalidArguments(error.message),
                    HostdErrorCode::PolicyDenied
                    | HostdErrorCode::UnauthorizedAction
                    | HostdErrorCode::UnauthenticatedPeer => ToolError::PolicyDenied(error.message),
                    HostdErrorCode::Timeout => ToolError::Timeout,
                    HostdErrorCode::OutputLimit => ToolError::OutputLimit {
                        limit: context.max_output_bytes,
                        preview: error.message,
                    },
                    HostdErrorCode::RecoveryRequired => ToolError::RecoveryRequired(error.message),
                    HostdErrorCode::UnsupportedVersion
                    | HostdErrorCode::Execution
                    | HostdErrorCode::Internal => ToolError::Execution(error.message),
                });
            }
            let result = response
                .result
                .ok_or_else(|| ToolError::Execution("hostd response had no result".into()))?;
            match result {
                HostdResult::ProcessExec(result) => Ok(HostProcessOutput {
                    exit_code: result.exit_code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                }),
                HostdResult::SandboxExec(_) | HostdResult::ManagedDeploy(_) => Err(ToolError::Execution(
                    "hostd returned a mismatched result type".into(),
                )),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (context, arguments);
            Err(ToolError::PolicyDenied(format!(
                "hostd Unix socket transport is not supported on this platform ({})",
                self.socket_path.display()
            )))
        }
    }

    async fn execute_sandbox(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostSandboxOutput, ToolError> {
        #[cfg(unix)]
        {
            let mut stream = tokio::net::UnixStream::connect(&self.socket_path)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd connection failed: {error}")))?;
            let request_id = Uuid::new_v4().to_string();
            let request = HostdRequestV3 {
                protocol_version: PROTOCOL_VERSION,
                request_id: request_id.clone(),
                run_id: context.run_id.clone(),
                call_id: context.call_id.clone(),
                action: HostdAction::SandboxExec {
                    arguments,
                    max_output_bytes: context.max_output_bytes,
                },
            };
            write_frame(&mut stream, &request, MAX_REQUEST_BYTES)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd request failed: {error}")))?;
            let response: HostdResponseV3 = read_frame(&mut stream, MAX_RESPONSE_BYTES)
                .await
                .map_err(|error| ToolError::Execution(format!("hostd response failed: {error}")))?;
            if response.protocol_version != PROTOCOL_VERSION || response.request_id != request_id {
                return Err(ToolError::Execution(
                    "hostd returned a mismatched protocol response".into(),
                ));
            }
            if let Some(error) = response.error {
                return Err(map_hostd_error(error, context.max_output_bytes));
            }
            match response
                .result
                .ok_or_else(|| ToolError::Execution("hostd response had no result".into()))?
            {
                HostdResult::SandboxExec(result) => Ok(HostSandboxOutput {
                    exit_code: result.exit_code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                    image: result.image,
                }),
                HostdResult::ProcessExec(_) | HostdResult::ManagedDeploy(_) => Err(ToolError::Execution(
                    "hostd returned a mismatched result type".into(),
                )),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (context, arguments);
            Err(ToolError::PolicyDenied(format!(
                "hostd Unix socket transport is not supported on this platform ({})",
                self.socket_path.display()
            )))
        }
    }

    async fn managed_deploy_plan(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        self.execute_managed(context, arguments, "plan").await
    }

    async fn managed_deploy_status(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        self.execute_managed(context, arguments, "status").await
    }

    async fn managed_deploy_apply(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        self.execute_managed(context, arguments, "apply").await
    }

    async fn managed_deploy_rollback(
        &self,
        context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        self.execute_managed(context, arguments, "rollback").await
    }
}

#[cfg(unix)]
fn map_hostd_error(error: soloops_hostd_protocol::HostdError, output_limit: usize) -> ToolError {
    match error.code {
        HostdErrorCode::InvalidRequest => ToolError::InvalidArguments(error.message),
        HostdErrorCode::PolicyDenied
        | HostdErrorCode::UnauthorizedAction
        | HostdErrorCode::UnauthenticatedPeer => ToolError::PolicyDenied(error.message),
        HostdErrorCode::Timeout => ToolError::Timeout,
        HostdErrorCode::OutputLimit => ToolError::OutputLimit {
            limit: output_limit,
            preview: error.message,
        },
        HostdErrorCode::RecoveryRequired => ToolError::RecoveryRequired(error.message),
        HostdErrorCode::UnsupportedVersion | HostdErrorCode::Execution | HostdErrorCode::Internal => {
            ToolError::Execution(error.message)
        }
    }
}
