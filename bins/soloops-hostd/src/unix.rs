use std::{
    collections::HashMap,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_hostd_protocol::{
    HostdAction, HostdErrorCode, HostdRequestV3, HostdResponseV3, HostdResult, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES, ProcessExecResult, SandboxExecResult, read_frame, supports_protocol_version,
    write_frame,
};
use soloops_storage::{AuditEntry, Database};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt},
    net::{UnixListener, UnixStream},
    process::{Child, ChildStderr, ChildStdout, Command},
    sync::Mutex,
};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::deployment::{ManagedDeployConfig, ManagedDeploymentService};
use crate::sandbox::{
    DockerSandboxDriver, SandboxConfig, SandboxDriverError, SandboxExecArguments, SandboxLimits,
    SharedSandboxDriver, build_run_spec, wait_for_workspace_limit, workspace_size,
};

const MAX_TOOL_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone)]
struct HostdState {
    database: Database,
    workspace_root: Arc<PathBuf>,
    allowlist: Arc<HashMap<String, PathBuf>>,
    sandbox_config: Option<Arc<SandboxConfig>>,
    sandbox_driver: Option<SharedSandboxDriver>,
    managed_deploy: Option<ManagedDeploymentService>,
    allowed_uid: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecArguments {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "dot")]
    cwd: String,
    #[serde(default)]
    env: HashMap<String, String>,
    timeout_ms: Option<u64>,
}

fn dot() -> String {
    ".".into()
}

pub async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::new(env("SOLOOPS_LOG", "soloops=info")))
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize hostd telemetry: {error}"))?;
    let socket_path = required_absolute_path("SOLOOPS_HOSTD_SOCKET")?;
    let database = Database::connect(configured_path("SOLOOPS_DATABASE_PATH")?).await?;
    database.verify_schema().await?;
    let workspace_root = configured_path("SOLOOPS_WORKSPACE_ROOT")?;
    fs::create_dir_all(&workspace_root).await?;
    let workspace_root = fs::canonicalize(workspace_root).await?;
    let allowed_uid = std::env::var("SOLOOPS_HOSTD_ALLOWED_UID")
        .context("SOLOOPS_HOSTD_ALLOWED_UID is required")?
        .parse::<u32>()
        .context("SOLOOPS_HOSTD_ALLOWED_UID must be an unsigned integer")?;
    let allowlist = parse_allowlist()?;
    let (sandbox_config, sandbox_driver) = parse_sandbox_config()?;
    if let Some(driver) = sandbox_driver.as_ref() {
        driver
            .cleanup_stale()
            .await
            .context("failed to clean stale SoloOps sandboxes")?;
    }
    let managed_deploy = ManagedDeployConfig::from_environment()
        .await?
        .map(|config| ManagedDeploymentService::new(database.clone(), workspace_root.clone(), config));
    if let Some(service) = managed_deploy.as_ref() {
        if let Err(error) = service.reconcile().await {
            warn!(%error, "initial managed deployment reconciliation failed");
        }
        tokio::spawn(service.clone().run_reconciler());
    }
    prepare_socket_path(&socket_path).await?;
    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o660)).await?;
    let state = HostdState {
        database,
        workspace_root: Arc::new(workspace_root),
        allowlist: Arc::new(allowlist),
        sandbox_config: sandbox_config.map(Arc::new),
        sandbox_driver,
        managed_deploy,
        allowed_uid,
    };
    info!(socket = %socket_path.display(), allowed_uid, "hostd started");
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, state).await {
                        warn!(%error, "hostd request rejected");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    drop(listener);
    if socket_path.exists() {
        fs::remove_file(&socket_path).await?;
    }
    state.database.close().await;
    Ok(())
}

async fn handle_connection(mut stream: UnixStream, state: HostdState) -> Result<()> {
    let credentials = stream.peer_cred()?;
    if !peer_is_allowed(credentials.uid(), state.allowed_uid) {
        anyhow::bail!("peer UID is not authorized");
    }
    let request: HostdRequestV3 = read_frame(&mut stream, MAX_REQUEST_BYTES).await?;
    let request_id = request.request_id.clone();
    let response = if !supports_protocol_version(request.protocol_version) {
        HostdResponseV3::failure(
            request_id,
            HostdErrorCode::UnsupportedVersion,
            "unsupported hostd protocol version",
        )
    } else {
        dispatch(request, &state, &mut stream).await
    };
    write_frame(&mut stream, &response, MAX_RESPONSE_BYTES).await?;
    Ok(())
}

async fn dispatch(request: HostdRequestV3, state: &HostdState, stream: &mut UnixStream) -> HostdResponseV3 {
    let request_id = request.request_id.clone();
    match request.action {
        HostdAction::ProcessExec {
            arguments,
            max_output_bytes,
        } => {
            let digest = format!("{:x}", Sha256::digest(arguments.to_string().as_bytes()));
            match state
                .database
                .authorize_host_tool_call(&request.run_id, &request.call_id, "process.exec", &digest)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    audit(
                        state,
                        &request.run_id,
                        &request.call_id,
                        "hostd.process.exec",
                        "denied",
                        json!({"category": "approval_mismatch"}),
                    )
                    .await;
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::UnauthorizedAction,
                        "process execution is not authorized by the persisted approval",
                    );
                }
                Err(error) => {
                    error!(%error, "hostd authorization query failed");
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::Internal,
                        "hostd authorization failed",
                    );
                }
            }
            let input: ExecArguments = match serde_json::from_value(arguments) {
                Ok(value) => value,
                Err(_) => {
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::InvalidRequest,
                        "process arguments are invalid",
                    );
                }
            };
            match execute_process(state, stream, &request.run_id, &input, max_output_bytes).await {
                Ok(result) => {
                    let outcome = if result.exit_code == Some(0) {
                        "success"
                    } else {
                        "failure"
                    };
                    audit(
                        state,
                        &request.run_id,
                        &request.call_id,
                        "hostd.process.exec",
                        outcome,
                        json!({"exitCode": result.exit_code}),
                    )
                    .await;
                    HostdResponseV3::success(request_id, HostdResult::ProcessExec(result))
                }
                Err((code, message)) => {
                    audit(
                        state,
                        &request.run_id,
                        &request.call_id,
                        "hostd.process.exec",
                        "failure",
                        json!({"category": code}),
                    )
                    .await;
                    HostdResponseV3::failure(request_id, code, message)
                }
            }
        }
        HostdAction::SandboxExec {
            arguments,
            max_output_bytes,
        } => {
            let digest = format!("{:x}", Sha256::digest(arguments.to_string().as_bytes()));
            match state
                .database
                .authorize_host_tool_call(&request.run_id, &request.call_id, "sandbox.exec", &digest)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    audit(
                        state,
                        &request.run_id,
                        &request.call_id,
                        "hostd.sandbox.exec",
                        "denied",
                        json!({"category": "approval_mismatch"}),
                    )
                    .await;
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::UnauthorizedAction,
                        "sandbox execution is not authorized by the persisted approval",
                    );
                }
                Err(error) => {
                    error!(%error, "hostd authorization query failed");
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::Internal,
                        "hostd authorization failed",
                    );
                }
            }
            let input: SandboxExecArguments = match serde_json::from_value(arguments) {
                Ok(value) => value,
                Err(_) => {
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::InvalidRequest,
                        "sandbox arguments are invalid",
                    );
                }
            };
            let Some(config) = state.sandbox_config.as_ref() else {
                return HostdResponseV3::failure(
                    request_id,
                    HostdErrorCode::PolicyDenied,
                    "Docker sandbox is disabled",
                );
            };
            let Some(driver) = state.sandbox_driver.as_ref() else {
                return HostdResponseV3::failure(
                    request_id,
                    HostdErrorCode::Internal,
                    "Docker sandbox driver is unavailable",
                );
            };
            let workspace = match resolve_workspace(&state.workspace_root, &request.run_id).await {
                Ok(value) => value,
                Err(_) => {
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::PolicyDenied,
                        "run workspace is invalid",
                    );
                }
            };
            let max_workspace_bytes = match state.database.runtime_snapshot(&request.run_id).await {
                Ok(Some(snapshot)) => snapshot.budget.max_workspace_bytes,
                Ok(None) => {
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::UnauthorizedAction,
                        "sandbox runtime state is unavailable",
                    );
                }
                Err(error) => {
                    error!(%error, "hostd failed to load the persisted sandbox budget");
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::Internal,
                        "hostd could not load the sandbox budget",
                    );
                }
            };
            match workspace_size(&workspace, max_workspace_bytes).await {
                Ok(size) if size <= max_workspace_bytes => {}
                Ok(_) => {
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::PolicyDenied,
                        "workspace already exceeds its persisted size budget",
                    );
                }
                Err(error) => {
                    error!(%error, "hostd failed to measure the sandbox workspace");
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::Internal,
                        "hostd could not measure the sandbox workspace",
                    );
                }
            }
            if resolve_cwd(&state.workspace_root, &request.run_id, &input.cwd)
                .await
                .is_err()
            {
                return HostdResponseV3::failure(
                    request_id,
                    HostdErrorCode::PolicyDenied,
                    "cwd is outside the run workspace or does not exist",
                );
            }
            let spec = match build_run_spec(config, workspace, &request.call_id, input, max_output_bytes) {
                Ok(value) => value,
                Err(error) => {
                    return HostdResponseV3::failure(
                        request_id,
                        HostdErrorCode::PolicyDenied,
                        error.to_string(),
                    );
                }
            };
            let container_name = spec.container_name.clone();
            let timeout = Duration::from_millis(spec.timeout_ms);
            let (mut read_half, _write_half) = stream.split();
            let execution = driver.run(&spec);
            tokio::pin!(execution);
            let workspace_limit = wait_for_workspace_limit(&spec.workspace, max_workspace_bytes);
            tokio::pin!(workspace_limit);
            let result = tokio::select! {
                result = tokio::time::timeout(timeout, &mut execution) => match result {
                    Ok(value) => value.map_err(map_sandbox_driver_error),
                    Err(_) => Err((HostdErrorCode::Timeout, "sandbox execution timed out".into())),
                },
                disconnected = read_half.read_u8() => match disconnected {
                    Ok(_) => Err((HostdErrorCode::Execution, "worker sent unexpected data during sandbox execution".into())),
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                        Err((HostdErrorCode::Execution, "worker disconnected while sandbox was running".into()))
                    }
                    Err(_) => Err((HostdErrorCode::Execution, "sandbox connection failed".into())),
                },
                limit = &mut workspace_limit => match limit {
                    Ok(()) => Err((HostdErrorCode::PolicyDenied, "sandbox exceeded the persisted workspace size budget".into())),
                    Err(_) => Err((HostdErrorCode::Internal, "sandbox workspace monitoring failed".into())),
                },
            };
            let cleanup = remove_sandbox(driver, &container_name).await;
            let result = match (result, cleanup) {
                (Ok(output), Ok(())) => Ok(output),
                (Ok(_), Err(error)) => Err((HostdErrorCode::Internal, error.to_string())),
                (Err(error), _) => Err(error),
            };
            match result {
                Ok(output) => {
                    let outcome = if output.exit_code == Some(0) {
                        "success"
                    } else {
                        "failure"
                    };
                    audit(
                        state,
                        &request.run_id,
                        &request.call_id,
                        "hostd.sandbox.exec",
                        outcome,
                        json!({"exitCode": output.exit_code, "image": config.image}),
                    )
                    .await;
                    HostdResponseV3::success(
                        request_id,
                        HostdResult::SandboxExec(SandboxExecResult {
                            exit_code: output.exit_code,
                            stdout: output.stdout,
                            stderr: output.stderr,
                            image: config.image.clone(),
                        }),
                    )
                }
                Err((code, message)) => {
                    audit(
                        state,
                        &request.run_id,
                        &request.call_id,
                        "hostd.sandbox.exec",
                        "failure",
                        json!({"category": code}),
                    )
                    .await;
                    HostdResponseV3::failure(request_id, code, message)
                }
            }
        }
        HostdAction::ManagedDeployPlan {
            arguments,
            max_output_bytes,
        } => {
            dispatch_managed(
                state,
                request_id,
                &request.run_id,
                &request.call_id,
                arguments,
                max_output_bytes,
                ManagedAction::Plan,
            )
            .await
        }
        HostdAction::ManagedDeployStatus {
            arguments,
            max_output_bytes,
        } => {
            dispatch_managed(
                state,
                request_id,
                &request.run_id,
                &request.call_id,
                arguments,
                max_output_bytes,
                ManagedAction::Status,
            )
            .await
        }
        HostdAction::ManagedDeployApply {
            arguments,
            max_output_bytes,
        } => {
            dispatch_managed(
                state,
                request_id,
                &request.run_id,
                &request.call_id,
                arguments,
                max_output_bytes,
                ManagedAction::Apply,
            )
            .await
        }
        HostdAction::ManagedDeployRollback {
            arguments,
            max_output_bytes,
        } => {
            dispatch_managed(
                state,
                request_id,
                &request.run_id,
                &request.call_id,
                arguments,
                max_output_bytes,
                ManagedAction::Rollback,
            )
            .await
        }
    }
}

#[derive(Clone, Copy)]
enum ManagedAction {
    Plan,
    Status,
    Apply,
    Rollback,
}

impl ManagedAction {
    const fn tool_name(self) -> &'static str {
        match self {
            Self::Plan => "managed.deploy.plan",
            Self::Status => "managed.deploy.status",
            Self::Apply => "managed.deploy.apply",
            Self::Rollback => "managed.deploy.rollback",
        }
    }

    const fn audit_action(self) -> &'static str {
        match self {
            Self::Plan => "hostd.managed.deploy.plan",
            Self::Status => "hostd.managed.deploy.status",
            Self::Apply => "hostd.managed.deploy.apply",
            Self::Rollback => "hostd.managed.deploy.rollback",
        }
    }
}

async fn dispatch_managed(
    state: &HostdState,
    request_id: String,
    run_id: &str,
    call_id: &str,
    arguments: Value,
    max_output_bytes: usize,
    action: ManagedAction,
) -> HostdResponseV3 {
    if max_output_bytes == 0 || max_output_bytes > MAX_TOOL_OUTPUT_BYTES {
        return HostdResponseV3::failure(
            request_id,
            HostdErrorCode::PolicyDenied,
            "managed deployment output limit is invalid",
        );
    }
    let digest = format!("{:x}", Sha256::digest(arguments.to_string().as_bytes()));
    match state
        .database
        .authorize_host_tool_call(run_id, call_id, action.tool_name(), &digest)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            audit(
                state,
                run_id,
                call_id,
                action.audit_action(),
                "denied",
                json!({"category": "authorization_mismatch"}),
            )
            .await;
            return HostdResponseV3::failure(
                request_id,
                HostdErrorCode::UnauthorizedAction,
                "managed deployment action is not authorized by persisted policy",
            );
        }
        Err(error) => {
            error!(%error, "managed deployment authorization query failed");
            return HostdResponseV3::failure(
                request_id,
                HostdErrorCode::Internal,
                "managed deployment authorization failed",
            );
        }
    }
    let Some(service) = state.managed_deploy.as_ref() else {
        return HostdResponseV3::failure(
            request_id,
            HostdErrorCode::PolicyDenied,
            "managed deployment is disabled",
        );
    };
    let result = match action {
        ManagedAction::Plan => service.plan(run_id, call_id, arguments, max_output_bytes).await,
        ManagedAction::Status => service.status(arguments).await,
        ManagedAction::Apply => service.apply(run_id, call_id, arguments, max_output_bytes).await,
        ManagedAction::Rollback => {
            service
                .rollback(run_id, call_id, arguments, max_output_bytes)
                .await
        }
    };
    match result {
        Ok(result) => {
            audit(
                state,
                run_id,
                call_id,
                action.audit_action(),
                "success",
                json!({
                    "projectId": result.project_id,
                    "revisionId": result.revision_id,
                    "status": result.status,
                }),
            )
            .await;
            HostdResponseV3::success(request_id, HostdResult::ManagedDeploy(result))
        }
        Err(error) => {
            audit(
                state,
                run_id,
                call_id,
                action.audit_action(),
                "failure",
                json!({"category": error.code}),
            )
            .await;
            HostdResponseV3::failure(request_id, error.code, error.message)
        }
    }
}

fn map_sandbox_driver_error(error: SandboxDriverError) -> (HostdErrorCode, String) {
    match error {
        SandboxDriverError::OutputLimit => (
            HostdErrorCode::OutputLimit,
            "sandbox output exceeded the configured limit".into(),
        ),
        SandboxDriverError::Docker(message) => (HostdErrorCode::Execution, message),
    }
}

async fn remove_sandbox(
    driver: &SharedSandboxDriver,
    container_name: &str,
) -> std::result::Result<(), SandboxDriverError> {
    let first = driver.remove(container_name).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = driver.remove(container_name).await;
    match (first, second) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(first), Err(_)) => Err(first),
    }
}

async fn execute_process(
    state: &HostdState,
    stream: &mut UnixStream,
    run_id: &str,
    input: &ExecArguments,
    max_output_bytes: usize,
) -> std::result::Result<ProcessExecResult, (HostdErrorCode, String)> {
    if !(1024..=MAX_TOOL_OUTPUT_BYTES).contains(&max_output_bytes)
        || input.args.len() > 128
        || input.env.len() > 128
        || input
            .env
            .iter()
            .any(|(key, value)| !valid_environment_key(key) || value.len() > 4096 || value.contains('\0'))
        || input
            .timeout_ms
            .is_some_and(|timeout| !(1..=60_000).contains(&timeout))
    {
        return Err((HostdErrorCode::PolicyDenied, "process limits are invalid".into()));
    }
    let executable = state.allowlist.get(&input.program).ok_or_else(|| {
        (
            HostdErrorCode::PolicyDenied,
            "program alias is not allowlisted".into(),
        )
    })?;
    let cwd = resolve_cwd(&state.workspace_root, run_id, &input.cwd)
        .await
        .map_err(|_| {
            (
                HostdErrorCode::PolicyDenied,
                "cwd is outside the run workspace".into(),
            )
        })?;
    let timeout = Duration::from_millis(input.timeout_ms.unwrap_or(60_000));
    let mut command = Command::new(executable);
    command
        .args(&input.args)
        .current_dir(cwd)
        .env_clear()
        .envs(&input.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| {
        (
            HostdErrorCode::Execution,
            "failed to start allowlisted process".into(),
        )
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| (HostdErrorCode::Internal, "process stdout was not captured".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| (HostdErrorCode::Internal, "process stderr was not captured".into()))?;
    let (read_half, _write_half) = stream.split();
    let result =
        capture_or_disconnect(&mut child, stdout, stderr, max_output_bytes, timeout, read_half).await;
    match result {
        Ok(output) => Ok(ProcessExecResult {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Err(CaptureFailure::Timeout) => Err((HostdErrorCode::Timeout, "process timed out".into())),
        Err(CaptureFailure::OutputLimit) => Err((
            HostdErrorCode::OutputLimit,
            "process output exceeded the configured limit".into(),
        )),
        Err(CaptureFailure::Disconnected) => Err((
            HostdErrorCode::Execution,
            "worker disconnected while process was running".into(),
        )),
        Err(CaptureFailure::Io) => Err((HostdErrorCode::Execution, "process I/O failed".into())),
    }
}

struct CapturedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Default)]
struct Capture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    total: usize,
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug)]
enum CaptureFailure {
    Io,
    Timeout,
    OutputLimit,
    Disconnected,
}

async fn drain<R: AsyncRead + Unpin>(
    mut reader: R,
    kind: StreamKind,
    capture: Arc<Mutex<Capture>>,
    limit: usize,
) -> std::result::Result<(), CaptureFailure> {
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader.read(&mut buffer).await.map_err(|_| CaptureFailure::Io)?;
        if read == 0 {
            return Ok(());
        }
        let mut capture = capture.lock().await;
        let keep = limit.saturating_sub(capture.total).min(read);
        match kind {
            StreamKind::Stdout => capture.stdout.extend_from_slice(&buffer[..keep]),
            StreamKind::Stderr => capture.stderr.extend_from_slice(&buffer[..keep]),
        }
        capture.total += keep;
        if keep < read {
            return Err(CaptureFailure::OutputLimit);
        }
    }
}

async fn capture_or_disconnect(
    child: &mut Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    limit: usize,
    timeout: Duration,
    mut peer: tokio::net::unix::ReadHalf<'_>,
) -> std::result::Result<CapturedOutput, CaptureFailure> {
    let capture = Arc::new(Mutex::new(Capture::default()));
    let outcome = {
        let execute = async {
            tokio::try_join!(
                drain(stdout, StreamKind::Stdout, capture.clone(), limit),
                drain(stderr, StreamKind::Stderr, capture.clone(), limit),
            )?;
            child.wait().await.map_err(|_| CaptureFailure::Io)
        };
        tokio::pin!(execute);
        tokio::select! {
            result = tokio::time::timeout(timeout, &mut execute) => match result {
                Ok(value) => value,
                Err(_) => Err(CaptureFailure::Timeout),
            },
            disconnected = peer.read_u8() => match disconnected {
                Ok(_) => Err(CaptureFailure::Disconnected),
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Err(CaptureFailure::Disconnected),
                Err(_) => Err(CaptureFailure::Io),
            },
        }
    };
    if outcome.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return outcome.map(|_| unreachable!());
    }
    let status = outcome.expect("checked above");
    let mut capture = capture.lock().await;
    Ok(CapturedOutput {
        status,
        stdout: std::mem::take(&mut capture.stdout),
        stderr: std::mem::take(&mut capture.stderr),
    })
}

async fn resolve_cwd(root: &Path, run_id: &str, cwd: &str) -> Result<PathBuf> {
    if run_id.is_empty()
        || run_id.contains(['/', '\\'])
        || Path::new(cwd).is_absolute()
        || Path::new(cwd).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("invalid workspace path");
    }
    let run_root = fs::canonicalize(root.join(run_id)).await?;
    if !run_root.starts_with(root) {
        anyhow::bail!("run workspace escaped root");
    }
    let candidate = fs::canonicalize(run_root.join(cwd)).await?;
    if !candidate.starts_with(&run_root) || !fs::metadata(&candidate).await?.is_dir() {
        anyhow::bail!("cwd escaped run workspace");
    }
    Ok(candidate)
}

async fn resolve_workspace(root: &Path, run_id: &str) -> Result<PathBuf> {
    if run_id.is_empty() || run_id.contains(['/', '\\']) {
        anyhow::bail!("invalid run ID");
    }
    let workspace = fs::canonicalize(root.join(run_id)).await?;
    if !workspace.starts_with(root) || !fs::metadata(&workspace).await?.is_dir() {
        anyhow::bail!("run workspace escaped root");
    }
    Ok(workspace)
}

async fn audit(
    state: &HostdState,
    run_id: &str,
    call_id: &str,
    action: &'static str,
    outcome: &'static str,
    context: Value,
) {
    if let Err(error) = state
        .database
        .write_audit(AuditEntry {
            actor_type: "hostd",
            actor_id: None,
            action,
            object_type: Some("tool_call"),
            object_id: Some(call_id.to_owned()),
            outcome,
            context: json!({"runId": run_id, "details": context}),
        })
        .await
    {
        error!(%error, run_id, call_id, "failed to persist hostd audit");
    }
}

fn parse_sandbox_config() -> Result<(Option<SandboxConfig>, Option<SharedSandboxDriver>)> {
    if !parse_bool(&env("SOLOOPS_SANDBOX_ENABLED", "false"))? {
        return Ok((None, None));
    }
    let programs: HashMap<String, String> =
        serde_json::from_str(&env("SOLOOPS_SANDBOX_PROGRAM_ALLOWLIST_JSON", "{}"))
            .context("SOLOOPS_SANDBOX_PROGRAM_ALLOWLIST_JSON must be a JSON object")?;
    let config = SandboxConfig {
        docker_socket: PathBuf::from(env("SOLOOPS_DOCKER_SOCKET", "/var/run/docker.sock")),
        image: std::env::var("SOLOOPS_SANDBOX_IMAGE")
            .context("SOLOOPS_SANDBOX_IMAGE is required when the sandbox is enabled")?,
        programs,
        limits: SandboxLimits {
            memory_bytes: parsed_env("SOLOOPS_SANDBOX_MEMORY_BYTES", 512 * 1024 * 1024)?,
            nano_cpus: parsed_env("SOLOOPS_SANDBOX_NANO_CPUS", 1_000_000_000)?,
            pids_limit: parsed_env("SOLOOPS_SANDBOX_PIDS_LIMIT", 128)?,
            tmpfs_bytes: parsed_env("SOLOOPS_SANDBOX_TMPFS_BYTES", 64 * 1024 * 1024)?,
        },
    };
    config.validate().context("Docker sandbox policy is invalid")?;
    let driver = Arc::new(DockerSandboxDriver::connect(&config.docker_socket)?) as SharedSandboxDriver;
    Ok((Some(config), Some(driver)))
}

fn parsed_env(name: &str, default: i64) -> Result<i64> {
    env(name, &default.to_string())
        .parse()
        .with_context(|| format!("{name} must be an integer"))
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("invalid boolean value: {value}"),
    }
}

fn parse_allowlist() -> Result<HashMap<String, PathBuf>> {
    let values: HashMap<String, String> = serde_json::from_str(&env("SOLOOPS_PROCESS_ALLOWLIST_JSON", "{}"))
        .context("SOLOOPS_PROCESS_ALLOWLIST_JSON must be a JSON object")?;
    values
        .into_iter()
        .map(|(alias, value)| {
            let path = PathBuf::from(value);
            if alias.trim().is_empty() || !path.is_absolute() {
                anyhow::bail!("hostd allowlist aliases must be non-empty and paths must be absolute");
            }
            let path = std::fs::canonicalize(&path)
                .with_context(|| format!("allowlisted program {alias} could not be resolved"))?;
            if !std::fs::metadata(&path)?.is_file() {
                anyhow::bail!("allowlisted program {alias} is not a file");
            }
            Ok((alias, path))
        })
        .collect()
}

async fn prepare_socket_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    if path.exists() {
        let metadata = fs::symlink_metadata(path).await?;
        if !metadata.file_type().is_socket() {
            anyhow::bail!("refusing to replace a non-socket hostd path");
        }
        fs::remove_file(path).await?;
    }
    Ok(())
}

fn required_absolute_path(name: &str) -> Result<PathBuf> {
    let value = std::env::var(name).with_context(|| format!("{name} is required"))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        anyhow::bail!("{name} must be absolute");
    }
    Ok(path)
}

fn configured_path(name: &str) -> Result<PathBuf> {
    let value = std::env::var(name).with_context(|| format!("{name} is required"))?;
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(path))
    }
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn peer_is_allowed(actual_uid: u32, allowed_uid: u32) -> bool {
    actual_uid == allowed_uid
}

fn valid_environment_key(key: &str) -> bool {
    key.strip_prefix("SOLOOPS_TOOL_").is_some_and(|suffix| {
        !suffix.is_empty()
            && key.len() <= 256
            && suffix
                .bytes()
                .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[tokio::test]
    async fn cwd_resolution_rejects_parent_and_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspaces");
        let run = root.join("run-1");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&run).await.unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        symlink(&outside, run.join("escape")).unwrap();
        let root = fs::canonicalize(root).await.unwrap();

        assert!(resolve_cwd(&root, "run-1", "..").await.is_err());
        assert!(resolve_cwd(&root, "run-1", "escape").await.is_err());
        assert_eq!(resolve_cwd(&root, "run-1", ".").await.unwrap(), run);
    }

    #[tokio::test]
    async fn output_limit_kills_and_reaps_the_process() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "while :; do printf '01234567890123456789\\n'; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (mut server, _client) = UnixStream::pair().unwrap();
        let (read_half, _write_half) = server.split();

        let result = capture_or_disconnect(
            &mut child,
            stdout,
            stderr,
            1024,
            Duration::from_secs(5),
            read_half,
        )
        .await;
        assert!(matches!(result, Err(CaptureFailure::OutputLimit)));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn timeout_kills_and_reaps_the_process() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (mut server, _client) = UnixStream::pair().unwrap();
        let (read_half, _write_half) = server.split();

        let result = capture_or_disconnect(
            &mut child,
            stdout,
            stderr,
            1024,
            Duration::from_millis(10),
            read_half,
        )
        .await;
        assert!(matches!(result, Err(CaptureFailure::Timeout)));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn disconnect_kills_and_reaps_the_process() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (mut server, client) = UnixStream::pair().unwrap();
        drop(client);
        let (read_half, _write_half) = server.split();

        let result = capture_or_disconnect(
            &mut child,
            stdout,
            stderr,
            1024,
            Duration::from_secs(5),
            read_half,
        )
        .await;
        assert!(matches!(result, Err(CaptureFailure::Disconnected)));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn policy_rejects_unknown_program_and_invalid_environment() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_root = temp.path().join("workspaces");
        fs::create_dir_all(workspace_root.join("run-1")).await.unwrap();
        let workspace_root = fs::canonicalize(workspace_root).await.unwrap();
        let state = HostdState {
            database: Database::connect(":memory:").await.unwrap(),
            workspace_root: Arc::new(workspace_root),
            allowlist: Arc::new(HashMap::new()),
            sandbox_config: None,
            sandbox_driver: None,
            managed_deploy: None,
            allowed_uid: 1000,
        };
        let (mut server, _client) = UnixStream::pair().unwrap();
        let unknown = ExecArguments {
            program: "unknown".into(),
            args: vec![],
            cwd: ".".into(),
            env: HashMap::new(),
            timeout_ms: Some(1000),
        };
        assert!(matches!(
            execute_process(&state, &mut server, "run-1", &unknown, 1024).await,
            Err((HostdErrorCode::PolicyDenied, _))
        ));

        let invalid_environment = ExecArguments {
            program: "unknown".into(),
            args: vec![],
            cwd: ".".into(),
            env: HashMap::from([("PATH".into(), "/tmp".into())]),
            timeout_ms: Some(1000),
        };
        assert!(matches!(
            execute_process(&state, &mut server, "run-1", &invalid_environment, 1024).await,
            Err((HostdErrorCode::PolicyDenied, _))
        ));
    }

    #[test]
    fn peer_uid_must_match_exactly() {
        assert!(peer_is_allowed(1000, 1000));
        assert!(!peer_is_allowed(1001, 1000));
    }

    #[test]
    fn environment_keys_are_namespaced_and_restricted() {
        assert!(valid_environment_key("SOLOOPS_TOOL_MODE_1"));
        assert!(!valid_environment_key("PATH"));
        assert!(!valid_environment_key("SOLOOPS_TOOL_"));
        assert!(!valid_environment_key("SOLOOPS_TOOL_bad"));
        assert!(!valid_environment_key("SOLOOPS_TOOL_BAD=VALUE"));
    }
}
