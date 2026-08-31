use std::{collections::HashMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_TOOL_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const WORKSPACE_TARGET: &str = "/workspace";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxLimits {
    pub memory_bytes: i64,
    pub nano_cpus: i64,
    pub pids_limit: i64,
    pub tmpfs_bytes: i64,
}

impl SandboxLimits {
    pub fn validate(&self) -> Result<(), SandboxPolicyError> {
        if !(64 * 1024 * 1024..=16 * 1024 * 1024 * 1024).contains(&self.memory_bytes)
            || !(100_000_000..=8_000_000_000).contains(&self.nano_cpus)
            || !(16..=4096).contains(&self.pids_limit)
            || !(1024 * 1024..=1024 * 1024 * 1024).contains(&self.tmpfs_bytes)
        {
            return Err(SandboxPolicyError::InvalidLimits);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub docker_socket: PathBuf,
    pub image: String,
    pub programs: HashMap<String, String>,
    pub limits: SandboxLimits,
}

impl SandboxConfig {
    pub fn validate(&self) -> Result<(), SandboxPolicyError> {
        if !self.docker_socket.is_absolute() {
            return Err(SandboxPolicyError::InvalidDockerSocket);
        }
        validate_digest_image(&self.image)?;
        if self.programs.is_empty() {
            return Err(SandboxPolicyError::EmptyProgramAllowlist);
        }
        for (alias, executable) in &self.programs {
            if !valid_alias(alias) || !valid_container_executable(executable) {
                return Err(SandboxPolicyError::InvalidProgramAlias(alias.clone()));
            }
        }
        self.limits.validate()
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxExecArguments {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "dot")]
    pub cwd: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub timeout_ms: Option<u64>,
}

fn dot() -> String {
    ".".into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRunSpec {
    pub container_name: String,
    pub image: String,
    pub executable: String,
    pub args: Vec<String>,
    pub working_dir: String,
    pub environment: Vec<String>,
    pub workspace: PathBuf,
    pub max_output_bytes: usize,
    pub timeout_ms: u64,
    pub limits: SandboxLimits,
}

pub fn build_run_spec(
    config: &SandboxConfig,
    workspace: PathBuf,
    call_id: &str,
    input: SandboxExecArguments,
    max_output_bytes: usize,
) -> Result<SandboxRunSpec, SandboxPolicyError> {
    config.validate()?;
    if !workspace.is_absolute() || call_id.is_empty() {
        return Err(SandboxPolicyError::InvalidWorkspace);
    }
    if !(1024..=MAX_TOOL_OUTPUT_BYTES).contains(&max_output_bytes)
        || input.args.len() > 128
        || input
            .args
            .iter()
            .any(|value| value.len() > 16 * 1024 || value.contains('\0'))
        || input.env.len() > 128
        || input
            .env
            .iter()
            .any(|(key, value)| !valid_environment_key(key) || value.len() > 4096 || value.contains('\0'))
        || input
            .timeout_ms
            .is_some_and(|timeout| !(1..=60_000).contains(&timeout))
    {
        return Err(SandboxPolicyError::InvalidArguments);
    }
    let executable = config
        .programs
        .get(&input.program)
        .cloned()
        .ok_or_else(|| SandboxPolicyError::ProgramNotAllowed(input.program.clone()))?;
    let working_dir = workspace_target(&input.cwd)?;
    let mut environment = input
        .env
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    environment.sort();
    Ok(SandboxRunSpec {
        container_name: container_name(call_id),
        image: config.image.clone(),
        executable,
        args: input.args,
        working_dir,
        environment,
        workspace,
        max_output_bytes,
        timeout_ms: input.timeout_ms.unwrap_or(60_000),
        limits: config.limits.clone(),
    })
}

fn workspace_target(cwd: &str) -> Result<String, SandboxPolicyError> {
    if cwd.contains('\\') || cwd.starts_with('/') || cwd.contains('\0') {
        return Err(SandboxPolicyError::InvalidCwd);
    }
    let mut components = Vec::new();
    for component in cwd.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err(SandboxPolicyError::InvalidCwd),
            value => components.push(value),
        }
    }
    if components.is_empty() {
        Ok(WORKSPACE_TARGET.into())
    } else {
        Ok(format!("{WORKSPACE_TARGET}/{}", components.join("/")))
    }
}

fn container_name(call_id: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(call_id.as_bytes()));
    format!("soloops-{}", &digest[..24])
}

fn validate_digest_image(image: &str) -> Result<(), SandboxPolicyError> {
    let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
        return Err(SandboxPolicyError::ImageMustBeDigestPinned);
    };
    if repository.trim().is_empty()
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(SandboxPolicyError::ImageMustBeDigestPinned);
    }
    Ok(())
}

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_container_executable(value: &str) -> bool {
    if !value.starts_with('/')
        || value.contains('\0')
        || value.contains("//")
        || value.split('/').any(|part| part == "." || part == "..")
    {
        return false;
    }
    !matches!(
        value
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "sh" | "bash" | "dash" | "zsh" | "fish" | "pwsh" | "powershell" | "cmd"
    )
}

pub fn valid_environment_key(key: &str) -> bool {
    key.strip_prefix("SOLOOPS_TOOL_").is_some_and(|suffix| {
        !suffix.is_empty()
            && key.len() <= 256
            && suffix
                .bytes()
                .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
    })
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxPolicyError {
    #[error("Docker socket must be an absolute path")]
    InvalidDockerSocket,
    #[error("sandbox image must be pinned by sha256 digest")]
    ImageMustBeDigestPinned,
    #[error("sandbox program allowlist must not be empty")]
    EmptyProgramAllowlist,
    #[error("sandbox program alias is invalid: {0}")]
    InvalidProgramAlias(String),
    #[error("sandbox resource limits are invalid")]
    InvalidLimits,
    #[error("sandbox workspace is invalid")]
    InvalidWorkspace,
    #[error("sandbox arguments exceed policy limits")]
    InvalidArguments,
    #[error("sandbox program alias is not allowlisted: {0}")]
    ProgramNotAllowed(String),
    #[error("sandbox cwd must stay below /workspace")]
    InvalidCwd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxOutput {
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum SandboxDriverError {
    #[error("Docker operation failed: {0}")]
    Docker(String),
    #[error("sandbox output exceeded the configured limit")]
    OutputLimit,
}

#[async_trait]
pub trait SandboxDriver: Send + Sync {
    async fn cleanup_stale(&self) -> Result<(), SandboxDriverError>;
    async fn run(&self, spec: &SandboxRunSpec) -> Result<SandboxOutput, SandboxDriverError>;
    async fn remove(&self, container_name: &str) -> Result<(), SandboxDriverError>;
}

pub type SharedSandboxDriver = Arc<dyn SandboxDriver>;

pub async fn workspace_size(root: &std::path::Path, stop_after: u64) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut pending = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(path) = pending.pop_front() {
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
            if total > stop_after {
                return Ok(total);
            }
        } else if metadata.is_dir() {
            let mut entries = tokio::fs::read_dir(path).await?;
            while let Some(entry) = entries.next_entry().await? {
                pending.push_back(entry.path());
            }
        }
    }
    Ok(total)
}

pub async fn wait_for_workspace_limit(root: &std::path::Path, maximum: u64) -> std::io::Result<()> {
    loop {
        if workspace_size(root, maximum).await? > maximum {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

pub struct DockerSandboxDriver {
    docker: bollard::Docker,
}

fn container_create_body(spec: &SandboxRunSpec) -> bollard::models::ContainerCreateBody {
    use bollard::models::{ContainerCreateBody, HostConfig, Mount, MountTypeEnum};

    let mut command = Vec::with_capacity(spec.args.len() + 1);
    command.push(spec.executable.clone());
    command.extend(spec.args.iter().cloned());
    let mount_source = spec.workspace.to_string_lossy().into_owned();
    let host_config = HostConfig {
        cap_drop: Some(vec!["ALL".into()]),
        init: Some(true),
        memory: Some(spec.limits.memory_bytes),
        mounts: Some(vec![Mount {
            target: Some(WORKSPACE_TARGET.into()),
            source: Some(mount_source),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        }]),
        nano_cpus: Some(spec.limits.nano_cpus),
        network_mode: Some("none".into()),
        pids_limit: Some(spec.limits.pids_limit),
        privileged: Some(false),
        readonly_rootfs: Some(true),
        security_opt: Some(vec!["no-new-privileges:true".into()]),
        tmpfs: Some(HashMap::from([(
            "/tmp".into(),
            format!("rw,noexec,nosuid,nodev,size={}", spec.limits.tmpfs_bytes),
        )])),
        ..Default::default()
    };
    ContainerCreateBody {
        image: Some(spec.image.clone()),
        cmd: Some(command),
        working_dir: Some(spec.working_dir.clone()),
        env: Some(spec.environment.clone()),
        user: Some("65532:65532".into()),
        labels: Some(HashMap::from([
            ("soloops.managed".into(), "true".into()),
            ("soloops.call".into(), spec.container_name.clone()),
        ])),
        network_disabled: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        open_stdin: Some(false),
        tty: Some(false),
        host_config: Some(host_config),
        ..Default::default()
    }
}

impl DockerSandboxDriver {
    #[cfg(unix)]
    pub fn connect(socket: &std::path::Path) -> Result<Self, SandboxDriverError> {
        let socket = socket
            .to_str()
            .ok_or_else(|| SandboxDriverError::Docker("Docker socket path is not UTF-8".into()))?;
        let docker = bollard::Docker::connect_with_unix(socket, 120, bollard::API_DEFAULT_VERSION)
            .map_err(|error| SandboxDriverError::Docker(error.to_string()))?;
        Ok(Self { docker })
    }

    fn append_output(
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
        message: bollard::container::LogOutput,
        limit: usize,
    ) -> Result<(), SandboxDriverError> {
        let total = stdout.len().saturating_add(stderr.len());
        let (target, bytes) = match message {
            bollard::container::LogOutput::StdOut { message }
            | bollard::container::LogOutput::Console { message } => (stdout, message),
            bollard::container::LogOutput::StdErr { message } => (stderr, message),
            bollard::container::LogOutput::StdIn { .. } => return Ok(()),
        };
        if bytes.len() > limit.saturating_sub(total) {
            return Err(SandboxDriverError::OutputLimit);
        }
        target.extend_from_slice(&bytes);
        Ok(())
    }

    fn output_from_wait_result(
        result: Result<bollard::models::ContainerWaitResponse, bollard::errors::Error>,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<SandboxOutput, SandboxDriverError> {
        let exit_code = match result {
            Ok(status) => status.status_code,
            Err(bollard::errors::Error::DockerContainerWaitError { code, .. }) => code,
            Err(error) => return Err(SandboxDriverError::Docker(error.to_string())),
        };
        Ok(SandboxOutput {
            exit_code: Some(exit_code),
            stdout: String::from_utf8_lossy(stdout).into_owned(),
            stderr: String::from_utf8_lossy(stderr).into_owned(),
        })
    }
}

#[async_trait]
impl SandboxDriver for DockerSandboxDriver {
    async fn cleanup_stale(&self) -> Result<(), SandboxDriverError> {
        use bollard::query_parameters::ListContainersOptionsBuilder;

        let filters = HashMap::from([("label", vec!["soloops.managed=true"])]);
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|error| SandboxDriverError::Docker(error.to_string()))?;
        for container in containers {
            if let Some(id) = container.id {
                self.remove(&id).await?;
            }
        }
        Ok(())
    }

    async fn run(&self, spec: &SandboxRunSpec) -> Result<SandboxOutput, SandboxDriverError> {
        use bollard::query_parameters::{
            CreateContainerOptionsBuilder, LogsOptionsBuilder, StartContainerOptions, WaitContainerOptions,
        };

        let config = container_create_body(spec);
        self.docker
            .create_container(
                Some(
                    CreateContainerOptionsBuilder::default()
                        .name(&spec.container_name)
                        .build(),
                ),
                config,
            )
            .await
            .map_err(|error| SandboxDriverError::Docker(error.to_string()))?;
        self.docker
            .start_container(&spec.container_name, None::<StartContainerOptions>)
            .await
            .map_err(|error| SandboxDriverError::Docker(error.to_string()))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut logs = self.docker.logs(
            &spec.container_name,
            Some(
                LogsOptionsBuilder::default()
                    .follow(true)
                    .stdout(true)
                    .stderr(true)
                    .build(),
            ),
        );
        while let Some(message) = logs.next().await {
            Self::append_output(
                &mut stdout,
                &mut stderr,
                message.map_err(|error| SandboxDriverError::Docker(error.to_string()))?,
                spec.max_output_bytes,
            )?;
        }
        let mut wait = self
            .docker
            .wait_container(&spec.container_name, None::<WaitContainerOptions>);
        let result = wait
            .next()
            .await
            .ok_or_else(|| SandboxDriverError::Docker("container wait returned no status".into()))?;
        Self::output_from_wait_result(result, &stdout, &stderr)
    }

    async fn remove(&self, container_name: &str) -> Result<(), SandboxDriverError> {
        use bollard::query_parameters::RemoveContainerOptionsBuilder;

        match self
            .docker
            .remove_container(
                container_name,
                Some(
                    RemoveContainerOptionsBuilder::default()
                        .force(true)
                        .v(true)
                        .build(),
                ),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(()),
            Err(error) => Err(SandboxDriverError::Docker(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeSandboxDriver {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SandboxDriver for FakeSandboxDriver {
        async fn cleanup_stale(&self) -> Result<(), SandboxDriverError> {
            self.calls.lock().unwrap().push("cleanup".into());
            Ok(())
        }

        async fn run(&self, spec: &SandboxRunSpec) -> Result<SandboxOutput, SandboxDriverError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("run:{}", spec.container_name));
            Ok(SandboxOutput {
                exit_code: Some(0),
                stdout: "ok".into(),
                stderr: String::new(),
            })
        }

        async fn remove(&self, container_name: &str) -> Result<(), SandboxDriverError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("remove:{container_name}"));
            Ok(())
        }
    }

    fn config() -> SandboxConfig {
        SandboxConfig {
            docker_socket: if cfg!(windows) {
                PathBuf::from("C:\\docker.sock")
            } else {
                PathBuf::from("/var/run/docker.sock")
            },
            image: "example/sandbox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into(),
            programs: HashMap::from([("cargo".into(), "/usr/local/bin/cargo".into())]),
            limits: SandboxLimits {
                memory_bytes: 512 * 1024 * 1024,
                nano_cpus: 1_000_000_000,
                pids_limit: 128,
                tmpfs_bytes: 64 * 1024 * 1024,
            },
        }
    }

    #[test]
    fn spec_is_offline_bounded_and_deterministic() {
        let spec = build_run_spec(
            &config(),
            std::env::current_dir().unwrap(),
            "call-1",
            SandboxExecArguments {
                program: "cargo".into(),
                args: vec!["test".into()],
                cwd: "crates/domain".into(),
                env: HashMap::from([("SOLOOPS_TOOL_MODE".into(), "test".into())]),
                timeout_ms: Some(5000),
            },
            4096,
        )
        .unwrap();
        assert_eq!(spec.working_dir, "/workspace/crates/domain");
        assert_eq!(spec.executable, "/usr/local/bin/cargo");
        assert_eq!(spec.environment, ["SOLOOPS_TOOL_MODE=test"]);
        assert_eq!(spec.container_name, container_name("call-1"));

        let body = container_create_body(&spec);
        assert_eq!(body.user.as_deref(), Some("65532:65532"));
        assert_eq!(body.network_disabled, Some(true));
        let host = body.host_config.unwrap();
        assert_eq!(host.network_mode.as_deref(), Some("none"));
        assert_eq!(host.readonly_rootfs, Some(true));
        assert_eq!(host.privileged, Some(false));
        assert_eq!(host.cap_drop, Some(vec!["ALL".into()]));
        assert_eq!(host.security_opt, Some(vec!["no-new-privileges:true".into()]));
        assert_eq!(host.memory, Some(512 * 1024 * 1024));
        assert_eq!(host.nano_cpus, Some(1_000_000_000));
        assert_eq!(host.pids_limit, Some(128));
        let mounts = host.mounts.unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].target.as_deref(), Some("/workspace"));
        assert_eq!(mounts[0].read_only, Some(false));
    }

    #[test]
    fn policy_rejects_unpinned_images_shells_and_path_escape() {
        let mut invalid = config();
        invalid.image = "example/sandbox:latest".into();
        assert_eq!(
            invalid.validate().unwrap_err(),
            SandboxPolicyError::ImageMustBeDigestPinned
        );

        let mut invalid = config();
        invalid.programs.insert("shell".into(), "/bin/sh".into());
        assert!(matches!(
            invalid.validate(),
            Err(SandboxPolicyError::InvalidProgramAlias(_))
        ));

        let result = build_run_spec(
            &config(),
            std::env::current_dir().unwrap(),
            "call-2",
            SandboxExecArguments {
                program: "cargo".into(),
                args: vec![],
                cwd: "../outside".into(),
                env: HashMap::new(),
                timeout_ms: None,
            },
            4096,
        );
        assert_eq!(result.unwrap_err(), SandboxPolicyError::InvalidCwd);
    }

    #[test]
    fn policy_rejects_untrusted_environment_and_unknown_alias() {
        let result = build_run_spec(
            &config(),
            std::env::current_dir().unwrap(),
            "call-3",
            SandboxExecArguments {
                program: "unknown".into(),
                args: vec![],
                cwd: ".".into(),
                env: HashMap::new(),
                timeout_ms: None,
            },
            4096,
        );
        assert!(matches!(result, Err(SandboxPolicyError::ProgramNotAllowed(_))));

        let result = build_run_spec(
            &config(),
            std::env::current_dir().unwrap(),
            "call-4",
            SandboxExecArguments {
                program: "cargo".into(),
                args: vec![],
                cwd: ".".into(),
                env: HashMap::from([("PATH".into(), "/tmp".into())]),
                timeout_ms: None,
            },
            4096,
        );
        assert_eq!(result.unwrap_err(), SandboxPolicyError::InvalidArguments);
    }

    #[tokio::test]
    async fn driver_contract_supports_startup_and_idempotent_cleanup() {
        let driver = FakeSandboxDriver::default();
        let spec = build_run_spec(
            &config(),
            std::env::current_dir().unwrap(),
            "fake-driver-call",
            SandboxExecArguments {
                program: "cargo".into(),
                args: vec!["test".into()],
                cwd: ".".into(),
                env: HashMap::new(),
                timeout_ms: None,
            },
            4096,
        )
        .unwrap();
        driver.cleanup_stale().await.unwrap();
        assert_eq!(driver.run(&spec).await.unwrap().exit_code, Some(0));
        driver.remove(&spec.container_name).await.unwrap();
        driver.remove(&spec.container_name).await.unwrap();
        let calls = driver.calls.lock().unwrap();
        assert_eq!(calls[0], "cleanup");
        assert_eq!(calls[1], format!("run:{}", spec.container_name));
        assert_eq!(calls[2], format!("remove:{}", spec.container_name));
        assert_eq!(calls[3], format!("remove:{}", spec.container_name));
    }

    #[test]
    fn wait_result_preserves_output_for_zero_and_nonzero_exit_codes() {
        let success = DockerSandboxDriver::output_from_wait_result(
            Ok(bollard::models::ContainerWaitResponse {
                status_code: 0,
                error: None,
            }),
            b"success output",
            b"",
        )
        .unwrap();
        assert_eq!(success.exit_code, Some(0));
        assert_eq!(success.stdout, "success output");
        assert_eq!(success.stderr, "");

        let failure = DockerSandboxDriver::output_from_wait_result(
            Err(bollard::errors::Error::DockerContainerWaitError {
                error: "program failed".into(),
                code: 7,
            }),
            b"captured stdout",
            b"captured stderr",
        )
        .unwrap();
        assert_eq!(failure.exit_code, Some(7));
        assert_eq!(failure.stdout, "captured stdout");
        assert_eq!(failure.stderr, "captured stderr");
    }

    #[test]
    fn wait_result_keeps_non_exit_failures_as_docker_errors() {
        let result = DockerSandboxDriver::output_from_wait_result(
            Err(bollard::errors::Error::RequestTimeoutError),
            b"stdout",
            b"stderr",
        );
        assert!(matches!(result, Err(SandboxDriverError::Docker(_))));
    }

    #[tokio::test]
    async fn workspace_size_stops_after_crossing_the_budget() {
        let workspace = tempfile::tempdir().unwrap();
        tokio::fs::write(workspace.path().join("one"), [0u8; 8])
            .await
            .unwrap();
        tokio::fs::create_dir(workspace.path().join("nested"))
            .await
            .unwrap();
        tokio::fs::write(workspace.path().join("nested/two"), [0u8; 8])
            .await
            .unwrap();
        assert!(workspace_size(workspace.path(), 10).await.unwrap() > 10);
        assert_eq!(workspace_size(workspace.path(), 100).await.unwrap(), 16);
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "requires a Linux Docker daemon and a preloaded digest-pinned test image"]
    async fn docker_exec_writes_workspace_and_leaves_no_container() {
        use std::os::unix::fs::PermissionsExt;

        let image = std::env::var("SOLOOPS_TEST_SANDBOX_IMAGE")
            .expect("SOLOOPS_TEST_SANDBOX_IMAGE must name a preloaded digest-pinned image");
        let socket = PathBuf::from(
            std::env::var("SOLOOPS_TEST_DOCKER_SOCKET").unwrap_or_else(|_| "/var/run/docker.sock".into()),
        );
        let executable =
            std::env::var("SOLOOPS_TEST_SANDBOX_COPY_PROGRAM").unwrap_or_else(|_| "/bin/cp".into());
        let workspace = tempfile::tempdir().unwrap();
        std::fs::set_permissions(workspace.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let config = SandboxConfig {
            docker_socket: socket.clone(),
            image,
            programs: HashMap::from([("copy".into(), executable)]),
            limits: config().limits,
        };
        let spec = build_run_spec(
            &config,
            workspace.path().to_path_buf(),
            "docker-integration-call",
            SandboxExecArguments {
                program: "copy".into(),
                args: vec!["/etc/os-release".into(), "/workspace/os-release".into()],
                cwd: ".".into(),
                env: HashMap::new(),
                timeout_ms: Some(10_000),
            },
            64 * 1024,
        )
        .unwrap();
        let driver = DockerSandboxDriver::connect(&socket).unwrap();
        let output = tokio::time::timeout(std::time::Duration::from_secs(15), driver.run(&spec))
            .await
            .unwrap()
            .unwrap();
        driver.remove(&spec.container_name).await.unwrap();
        driver.remove(&spec.container_name).await.unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert!(workspace.path().join("os-release").is_file());
    }
}
