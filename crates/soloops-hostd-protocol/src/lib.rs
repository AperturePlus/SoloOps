use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: u16 = 3;
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024 + 64 * 1024;

pub const fn supports_protocol_version(version: u16) -> bool {
    version == PROTOCOL_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostdRequestV3 {
    pub protocol_version: u16,
    pub request_id: String,
    pub run_id: String,
    pub call_id: String,
    pub action: HostdAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostdAction {
    ProcessExec {
        arguments: Value,
        max_output_bytes: usize,
    },
    SandboxExec {
        arguments: Value,
        max_output_bytes: usize,
    },
    ManagedDeployPlan {
        arguments: Value,
        max_output_bytes: usize,
    },
    ManagedDeployStatus {
        arguments: Value,
        max_output_bytes: usize,
    },
    ManagedDeployApply {
        arguments: Value,
        max_output_bytes: usize,
    },
    ManagedDeployRollback {
        arguments: Value,
        max_output_bytes: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostdResponseV3 {
    pub protocol_version: u16,
    pub request_id: String,
    pub result: Option<HostdResult>,
    pub error: Option<HostdError>,
}

impl HostdResponseV3 {
    pub fn success(request_id: String, result: HostdResult) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(request_id: String, code: HostdErrorCode, message: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            result: None,
            error: Some(HostdError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum HostdResult {
    ProcessExec(ProcessExecResult),
    SandboxExec(SandboxExecResult),
    ManagedDeploy(ManagedDeployResult),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessExecResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxExecResult {
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedDeployResult {
    pub action: String,
    pub project_id: String,
    pub status: String,
    pub revision_id: Option<String>,
    pub previous_revision_id: Option<String>,
    pub proposal_sha256: Option<String>,
    pub preview: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostdError {
    pub code: HostdErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostdErrorCode {
    InvalidRequest,
    UnsupportedVersion,
    UnauthenticatedPeer,
    UnauthorizedAction,
    PolicyDenied,
    Timeout,
    OutputLimit,
    Execution,
    RecoveryRequired,
    Internal,
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame length {actual} exceeds limit {limit}")]
    TooLarge { actual: usize, limit: usize },
    #[error("frame I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn write_frame<W, T>(writer: &mut W, value: &T, limit: usize) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value)?;
    if payload.len() > limit {
        return Err(FrameError::TooLarge {
            actual: payload.len(),
            limit,
        });
    }
    writer.write_u32(payload.len() as u32).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_frame<R, T>(reader: &mut R, limit: usize) -> Result<T, FrameError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let length = reader.read_u32().await? as usize;
    if length > limit {
        return Err(FrameError::TooLarge {
            actual: length,
            limit,
        });
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).await?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_round_trip_and_size_limit() {
        let request = HostdRequestV3 {
            protocol_version: PROTOCOL_VERSION,
            request_id: "request-1".into(),
            run_id: "run-1".into(),
            call_id: "call-1".into(),
            action: HostdAction::ProcessExec {
                arguments: serde_json::json!({"program": "git"}),
                max_output_bytes: 4096,
            },
        };
        let (mut writer, mut reader) = tokio::io::duplex(4096);
        write_frame(&mut writer, &request, MAX_REQUEST_BYTES)
            .await
            .unwrap();
        let decoded: HostdRequestV3 = read_frame(&mut reader, MAX_REQUEST_BYTES).await.unwrap();
        assert_eq!(decoded, request);

        let error = write_frame(&mut writer, &request, 1).await.unwrap_err();
        assert!(matches!(error, FrameError::TooLarge { .. }));
    }

    #[test]
    fn v3_results_are_action_specific() {
        assert!(!supports_protocol_version(2));
        assert!(supports_protocol_version(3));
        let response = HostdResponseV3::success(
            "request-2".into(),
            HostdResult::SandboxExec(SandboxExecResult {
                exit_code: Some(0),
                stdout: "ok".into(),
                stderr: String::new(),
                image: "example@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            }),
        );
        let encoded = serde_json::to_value(response).unwrap();
        assert_eq!(encoded["protocolVersion"], 3);
        assert_eq!(encoded["result"]["type"], "sandbox_exec");
    }

    #[test]
    fn v3_managed_deployment_result_is_typed() {
        let response = HostdResponseV3::success(
            "request-managed".into(),
            HostdResult::ManagedDeploy(ManagedDeployResult {
                action: "plan".into(),
                project_id: "demo".into(),
                status: "proposed".into(),
                revision_id: Some("revision-1".into()),
                previous_revision_id: None,
                proposal_sha256: Some("a".repeat(64)),
                preview: Some(serde_json::json!({"services": ["web"]})),
            }),
        );
        let encoded = serde_json::to_value(response).unwrap();
        assert_eq!(encoded["protocolVersion"], 3);
        assert_eq!(encoded["result"]["type"], "managed_deploy");
        assert_eq!(encoded["result"]["value"]["projectId"], "demo");
    }
}
