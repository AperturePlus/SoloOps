use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::header::HOST;
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_hostd_protocol::{HostdErrorCode, ManagedDeployResult};
use soloops_storage::{Database, NewManagedDeploymentRevision};
use tokio::{fs, process::Command};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ManagedDeployConfig {
    pub managed_root: PathBuf,
    pub docker_cli: PathBuf,
    pub caddy_cli: PathBuf,
    pub caddy_config: PathBuf,
    pub caddy_managed_dir: PathBuf,
    pub allowed_sites: HashSet<String>,
    pub allowed_ports: HashSet<u16>,
    pub health_origin: String,
    pub health_timeout: Duration,
    pub operation_lease: Duration,
    pub operation_renew: Duration,
    pub reconcile_interval: Duration,
}

impl ManagedDeployConfig {
    pub async fn from_environment() -> Result<Option<Self>> {
        if !parse_bool(&env("SOLOOPS_MANAGED_DEPLOY_ENABLED", "false"))? {
            return Ok(None);
        }
        let config = Self {
            managed_root: absolute_path("SOLOOPS_MANAGED_ROOT", "/var/lib/soloops/deployments")?,
            docker_cli: existing_file("SOLOOPS_DOCKER_CLI", "/usr/bin/docker")?,
            caddy_cli: existing_file("SOLOOPS_CADDY_CLI", "/usr/bin/caddy")?,
            caddy_config: existing_file("SOLOOPS_CADDY_CONFIG", "/etc/caddy/Caddyfile")?,
            caddy_managed_dir: absolute_path("SOLOOPS_CADDY_MANAGED_DIR", "/etc/caddy/soloops.d")?,
            allowed_sites: parse_string_set("SOLOOPS_DEPLOY_SITE_ALLOWLIST_JSON")?,
            allowed_ports: parse_port_set("SOLOOPS_DEPLOY_PORT_ALLOWLIST_JSON")?,
            health_origin: env("SOLOOPS_DEPLOY_HEALTH_ORIGIN", "http://127.0.0.1"),
            health_timeout: Duration::from_millis(parsed_u64("SOLOOPS_DEPLOY_HEALTH_TIMEOUT_MS", 60_000)?),
            operation_lease: Duration::from_millis(parsed_range(
                "SOLOOPS_DEPLOY_OPERATION_LEASE_MS",
                60_000,
                15_000,
                600_000,
            )?),
            operation_renew: Duration::from_millis(parsed_range(
                "SOLOOPS_DEPLOY_OPERATION_RENEW_MS",
                10_000,
                1_000,
                60_000,
            )?),
            reconcile_interval: Duration::from_millis(parsed_range(
                "SOLOOPS_DEPLOY_RECONCILE_INTERVAL_MS",
                15_000,
                1_000,
                300_000,
            )?),
        };
        if config.allowed_sites.is_empty() || config.allowed_ports.is_empty() {
            anyhow::bail!("managed deployment requires non-empty site and port allowlists");
        }
        if config.operation_renew >= config.operation_lease / 2 {
            anyhow::bail!("SOLOOPS_DEPLOY_OPERATION_RENEW_MS must be less than half the operation lease");
        }
        let health_origin = reqwest::Url::parse(&config.health_origin)
            .context("SOLOOPS_DEPLOY_HEALTH_ORIGIN must be an absolute URL")?;
        let loopback_host = matches!(health_origin.host_str(), Some("127.0.0.1" | "::1"));
        if health_origin.scheme() != "http"
            || !loopback_host
            || !health_origin.username().is_empty()
            || health_origin.password().is_some()
            || health_origin.path() != "/"
            || health_origin.query().is_some()
            || health_origin.fragment().is_some()
        {
            anyhow::bail!("SOLOOPS_DEPLOY_HEALTH_ORIGIN must be a bare loopback HTTP origin");
        }
        fs::create_dir_all(&config.managed_root).await?;
        fs::create_dir_all(&config.caddy_managed_dir).await?;
        Ok(Some(config))
    }
}

#[derive(Debug)]
pub struct ManagedDeployError {
    pub code: HostdErrorCode,
    pub message: String,
}

impl ManagedDeployError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: HostdErrorCode::InvalidRequest,
            message: message.into(),
        }
    }

    fn denied(message: impl Into<String>) -> Self {
        Self {
            code: HostdErrorCode::PolicyDenied,
            message: message.into(),
        }
    }

    fn execution(message: impl Into<String>) -> Self {
        Self {
            code: HostdErrorCode::Execution,
            message: message.into(),
        }
    }

    fn recovery(message: impl Into<String>) -> Self {
        Self {
            code: HostdErrorCode::RecoveryRequired,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: HostdErrorCode::Internal,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct CommandOutput {
    stdout: String,
    stderr: String,
}

#[async_trait]
trait ManagedCommandRunner: Send + Sync {
    async fn run(
        &self,
        program: &Path,
        arguments: &[String],
        timeout: Duration,
        output_limit: usize,
    ) -> Result<CommandOutput, ManagedDeployError>;
}

struct SystemCommandRunner;

struct OperationLeaseGuard {
    stopped: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
}

impl OperationLeaseGuard {
    fn start(
        database: Database,
        call_id: String,
        lease_token: String,
        lease: Duration,
        renew: Duration,
    ) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));
        let task_stopped = stopped.clone();
        let task_lost = lost.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(renew);
            let mut expires_at = tokio::time::Instant::now() + lease;
            interval.tick().await;
            while !task_stopped.load(Ordering::Relaxed) {
                interval.tick().await;
                if task_stopped.load(Ordering::Relaxed) {
                    break;
                }
                match database
                    .renew_managed_deployment_lease(
                        &call_id,
                        &lease_token,
                        lease.as_millis().try_into().unwrap_or(i64::MAX),
                    )
                    .await
                {
                    Ok(true) => expires_at = tokio::time::Instant::now() + lease,
                    Ok(false) => {
                        task_lost.store(true, Ordering::Relaxed);
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(%error, %call_id, "managed deployment lease renewal failed");
                        if tokio::time::Instant::now() >= expires_at {
                            task_lost.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            }
        });
        Self { stopped, lost }
    }

    fn ensure_owned(&self) -> Result<(), ManagedDeployError> {
        if self.lost.load(Ordering::Relaxed) {
            return Err(ManagedDeployError::recovery(
                "managed deployment lease was lost; reconciliation is required",
            ));
        }
        Ok(())
    }
}

impl Drop for OperationLeaseGuard {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

#[async_trait]
impl ManagedCommandRunner for SystemCommandRunner {
    async fn run(
        &self,
        program: &Path,
        arguments: &[String],
        timeout: Duration,
        output_limit: usize,
    ) -> Result<CommandOutput, ManagedDeployError> {
        let future = Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(timeout, future)
            .await
            .map_err(|_| ManagedDeployError {
                code: HostdErrorCode::Timeout,
                message: "managed deployment command timed out".into(),
            })?
            .map_err(|_| ManagedDeployError::execution("managed deployment command failed to start"))?;
        if output.stdout.len().saturating_add(output.stderr.len()) > output_limit {
            return Err(ManagedDeployError {
                code: HostdErrorCode::OutputLimit,
                message: "managed deployment command exceeded its output limit".into(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(ManagedDeployError::execution(format!(
                "managed deployment command exited unsuccessfully: {}",
                stderr.chars().take(1000).collect::<String>()
            )));
        }
        Ok(CommandOutput { stdout, stderr })
    }
}

#[derive(Clone)]
pub struct ManagedDeploymentService {
    database: Database,
    workspace_root: PathBuf,
    config: Arc<ManagedDeployConfig>,
    runner: Arc<dyn ManagedCommandRunner>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanArguments {
    project_id: String,
    compose_path: String,
    caddy_fragment_path: String,
    #[serde(default = "root_health_path")]
    health_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatusArguments {
    project_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyArguments {
    proposal_id: String,
    proposal_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RollbackArguments {
    project_id: String,
    expected_current_revision_id: String,
    target_revision_id: String,
}

fn root_health_path() -> String {
    "/".into()
}

impl ManagedDeploymentService {
    async fn ensure_operation_lease(
        &self,
        call_id: &str,
        lease_token: &str,
        guard: &OperationLeaseGuard,
    ) -> Result<(), ManagedDeployError> {
        guard.ensure_owned()?;
        match self
            .database
            .renew_managed_deployment_lease(
                call_id,
                lease_token,
                self.config
                    .operation_lease
                    .as_millis()
                    .try_into()
                    .unwrap_or(i64::MAX),
            )
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) => Err(ManagedDeployError::recovery(
                "managed deployment lease expired or was claimed by reconciliation",
            )),
            Err(_) => Err(ManagedDeployError::recovery(
                "managed deployment lease could not be verified before a side effect",
            )),
        }
    }

    pub fn new(database: Database, workspace_root: PathBuf, config: ManagedDeployConfig) -> Self {
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("redirect-free managed health client is valid");
        Self {
            database,
            workspace_root,
            config: Arc::new(config),
            runner: Arc::new(SystemCommandRunner),
            http,
        }
    }

    pub async fn reconcile(&self) -> Result<usize> {
        let mut reconciled = 0;
        loop {
            let lease_token = Uuid::new_v4().to_string();
            let Some(operation) = self
                .database
                .claim_expired_managed_deployment_operation(
                    &lease_token,
                    self.config
                        .operation_lease
                        .as_millis()
                        .try_into()
                        .unwrap_or(i64::MAX),
                )
                .await?
            else {
                break;
            };
            let lease = OperationLeaseGuard::start(
                self.database.clone(),
                operation.call_id.clone(),
                lease_token.clone(),
                self.config.operation_lease,
                self.config.operation_renew,
            );
            let revision = self
                .database
                .managed_deployment_revision(&operation.revision_id)
                .await?
                .with_context(|| {
                    format!(
                        "unfinished deployment revision {} is missing",
                        operation.revision_id
                    )
                });
            let revision = match revision {
                Ok(revision) => revision,
                Err(error) => {
                    let _ = self
                        .database
                        .record_managed_deployment_recovery_failure(
                            &operation.call_id,
                            &lease_token,
                            &error.to_string(),
                            self.config
                                .operation_lease
                                .as_millis()
                                .try_into()
                                .unwrap_or(i64::MAX),
                        )
                        .await;
                    tracing::warn!(%error, call_id = %operation.call_id, "managed deployment reconciliation failed");
                    continue;
                }
            };
            let previous = match operation.previous_revision_id.as_deref() {
                Some(id) => match self.database.managed_deployment_revision(id).await {
                    Ok(Some(previous)) => Some(previous),
                    Ok(None) => {
                        let message = format!("previous managed deployment revision {id} is missing");
                        let _ = self
                            .database
                            .record_managed_deployment_recovery_failure(
                                &operation.call_id,
                                &lease_token,
                                &message,
                                self.config
                                    .operation_lease
                                    .as_millis()
                                    .try_into()
                                    .unwrap_or(i64::MAX),
                            )
                            .await;
                        tracing::warn!(call_id = %operation.call_id, error = %message, "managed deployment reconciliation failed");
                        continue;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = self
                            .database
                            .record_managed_deployment_recovery_failure(
                                &operation.call_id,
                                &lease_token,
                                &message,
                                self.config
                                    .operation_lease
                                    .as_millis()
                                    .try_into()
                                    .unwrap_or(i64::MAX),
                            )
                            .await;
                        tracing::warn!(call_id = %operation.call_id, %error, "managed deployment reconciliation failed");
                        continue;
                    }
                },
                None => None,
            };
            let (status, rollback_succeeded, recovery_error) = if operation.phase == "prepared" {
                ("aborted_before_side_effect", true, None)
            } else {
                let outcome = self.compensate(&revision, previous.as_ref(), 256 * 1024).await;
                (
                    "reconciled_after_lease_expiry",
                    outcome.is_ok(),
                    outcome.err().map(|error| error.message),
                )
            };
            let result = json!({
                "action": operation.action,
                "projectId": operation.project_id,
                "status": status,
                "revisionId": operation.revision_id,
                "previousRevisionId": operation.previous_revision_id,
                "rollbackSucceeded": rollback_succeeded,
            });
            if rollback_succeeded && lease.ensure_owned().is_ok() {
                if let Err(error) = self
                    .database
                    .fail_managed_deployment_operation(
                        &operation.call_id,
                        &lease_token,
                        "hostd_lease_reconciled",
                        &result,
                    )
                    .await
                {
                    tracing::warn!(%error, call_id = %operation.call_id, "could not persist managed reconciliation result");
                } else {
                    reconciled += 1;
                }
            } else {
                let message = recovery_error.unwrap_or_else(|| "managed deployment lease was lost".into());
                let _ = self
                    .database
                    .record_managed_deployment_recovery_failure(
                        &operation.call_id,
                        &lease_token,
                        &message,
                        self.config
                            .operation_lease
                            .as_millis()
                            .try_into()
                            .unwrap_or(i64::MAX),
                    )
                    .await;
                tracing::warn!(call_id = %operation.call_id, error = %message, "managed deployment could not be reconciled; it will be retried");
            }
        }
        Ok(reconciled)
    }

    pub async fn run_reconciler(self) {
        loop {
            if let Err(error) = self.reconcile().await {
                tracing::warn!(%error, "managed deployment reconciliation scan failed");
            }
            tokio::time::sleep(self.config.reconcile_interval).await;
        }
    }

    pub async fn plan(
        &self,
        run_id: &str,
        call_id: &str,
        arguments: Value,
        output_limit: usize,
    ) -> Result<ManagedDeployResult, ManagedDeployError> {
        let input: PlanArguments = serde_json::from_value(arguments.clone())
            .map_err(|_| ManagedDeployError::invalid("managed deploy plan arguments are invalid"))?;
        validate_project_id(&input.project_id)?;
        validate_health_path(&input.health_path)?;
        let workspace = resolve_workspace(&self.workspace_root, run_id).await?;
        let compose_path = resolve_workspace_file(&workspace, &input.compose_path).await?;
        let caddy_path = resolve_workspace_file(&workspace, &input.caddy_fragment_path).await?;
        let compose_bytes = fs::read(&compose_path)
            .await
            .map_err(|_| ManagedDeployError::invalid("Compose source could not be read"))?;
        let caddy_bytes = fs::read(&caddy_path)
            .await
            .map_err(|_| ManagedDeployError::invalid("Caddy source could not be read"))?;
        if compose_bytes.len().saturating_add(caddy_bytes.len()) > output_limit {
            return Err(ManagedDeployError::denied(
                "deployment source exceeds the Tool output limit",
            ));
        }
        validate_compose_source(&compose_bytes, &self.config)?;
        let compose_arguments = vec![
            "compose".into(),
            "-f".into(),
            compose_path.to_string_lossy().into_owned(),
            "config".into(),
            "--format".into(),
            "json".into(),
        ];
        let normalized = self
            .runner
            .run(
                &self.config.docker_cli,
                &compose_arguments,
                Duration::from_secs(30),
                output_limit,
            )
            .await?;
        let compose: Value = serde_json::from_str(&normalized.stdout)
            .map_err(|_| ManagedDeployError::denied("docker compose returned invalid normalized JSON"))?;
        let compose_preview = validate_compose(&compose, &self.config)?;
        for image in compose_preview["images"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            self.runner
                .run(
                    &self.config.docker_cli,
                    &["image".into(), "inspect".into(), image.to_owned()],
                    Duration::from_secs(30),
                    output_limit,
                )
                .await
                .map_err(|_| {
                    ManagedDeployError::denied(format!("digest-pinned image is not preloaded: {image}"))
                })?;
        }
        let caddy_text = String::from_utf8(caddy_bytes.clone())
            .map_err(|_| ManagedDeployError::denied("Caddy fragment must be UTF-8"))?;
        let caddy_preview = validate_caddy(&caddy_text, &compose_preview, &self.config)?;
        let adapt_arguments = vec![
            "adapt".into(),
            "--adapter".into(),
            "caddyfile".into(),
            "--config".into(),
            caddy_path.to_string_lossy().into_owned(),
        ];
        self.runner
            .run(
                &self.config.caddy_cli,
                &adapt_arguments,
                Duration::from_secs(30),
                output_limit,
            )
            .await?;
        let compose_sha256 = sha256(&compose_bytes);
        let caddy_sha256 = sha256(&caddy_bytes);
        let source = json!({
            "projectId": input.project_id,
            "composePath": input.compose_path,
            "caddyFragmentPath": input.caddy_fragment_path,
            "healthPath": input.health_path,
        });
        let preview = json!({
            "projectId": input.project_id,
            "services": compose_preview["services"],
            "images": compose_preview["images"],
            "ports": compose_preview["ports"],
            "volumes": compose_preview["volumes"],
            "site": caddy_preview["site"],
            "routes": caddy_preview["routes"],
            "healthPath": input.health_path,
            "composeSha256": compose_sha256,
            "caddySha256": caddy_sha256,
        });
        let proposal_sha256 = sha256(format!("{}\n{}\n{}", source, compose_sha256, caddy_sha256).as_bytes());
        let bundle = self
            .config
            .managed_root
            .join(&input.project_id)
            .join("revisions")
            .join(&proposal_sha256);
        fs::create_dir_all(&bundle)
            .await
            .map_err(|_| ManagedDeployError::internal("could not create the managed bundle"))?;
        fs::write(bundle.join("compose.yaml"), &compose_bytes)
            .await
            .map_err(|_| ManagedDeployError::internal("could not persist the managed Compose file"))?;
        fs::write(bundle.join("site.caddy"), &caddy_bytes)
            .await
            .map_err(|_| ManagedDeployError::internal("could not persist the managed Caddy file"))?;
        let revision = self
            .database
            .create_managed_deployment_revision(NewManagedDeploymentRevision {
                project_id: &input.project_id,
                run_id,
                plan_call_id: call_id,
                proposal_sha256: &proposal_sha256,
                compose_sha256: &compose_sha256,
                caddy_sha256: &caddy_sha256,
                source: &source,
                preview: &preview,
                bundle_path: &bundle.to_string_lossy(),
            })
            .await
            .map_err(|_| ManagedDeployError::internal("could not persist the deployment proposal"))?;
        Ok(result_from_revision("plan", &revision, Some(preview)))
    }

    pub async fn status(&self, arguments: Value) -> Result<ManagedDeployResult, ManagedDeployError> {
        let input: StatusArguments = serde_json::from_value(arguments)
            .map_err(|_| ManagedDeployError::invalid("managed deploy status arguments are invalid"))?;
        validate_project_id(&input.project_id)?;
        match self
            .database
            .current_managed_deployment_revision(&input.project_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read deployment status"))?
        {
            Some(revision) => Ok(result_from_revision(
                "status",
                &revision,
                Some(revision.preview.clone()),
            )),
            None => Ok(ManagedDeployResult {
                action: "status".into(),
                project_id: input.project_id,
                status: "not_deployed".into(),
                revision_id: None,
                previous_revision_id: None,
                proposal_sha256: None,
                preview: None,
            }),
        }
    }

    pub async fn apply(
        &self,
        run_id: &str,
        call_id: &str,
        arguments: Value,
        output_limit: usize,
    ) -> Result<ManagedDeployResult, ManagedDeployError> {
        let input: ApplyArguments = serde_json::from_value(arguments)
            .map_err(|_| ManagedDeployError::invalid("managed deploy apply arguments are invalid"))?;
        if let Some(result) = self
            .replay_operation_result(run_id, call_id, "apply", None, &input.proposal_id, None)
            .await?
        {
            return Ok(result);
        }
        let revision = self
            .database
            .managed_deployment_revision(&input.proposal_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read the deployment proposal"))?
            .ok_or_else(|| ManagedDeployError::invalid("deployment proposal was not found"))?;
        if revision.proposal_sha256 != input.proposal_sha256 || revision.status != "proposed" {
            return Err(ManagedDeployError::denied(
                "deployment proposal digest or status mismatch",
            ));
        }
        let current = self
            .database
            .current_managed_deployment_revision(&revision.project_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read the active deployment"))?;
        if current.as_ref().map(|item| item.id.as_str()) != revision.previous_revision_id.as_deref() {
            return Err(ManagedDeployError::denied(
                "deployment proposal is stale relative to the active revision",
            ));
        }
        self.execute_change(run_id, call_id, "apply", revision, None, output_limit)
            .await
    }

    pub async fn rollback(
        &self,
        run_id: &str,
        call_id: &str,
        arguments: Value,
        output_limit: usize,
    ) -> Result<ManagedDeployResult, ManagedDeployError> {
        let input: RollbackArguments = serde_json::from_value(arguments)
            .map_err(|_| ManagedDeployError::invalid("managed deploy rollback arguments are invalid"))?;
        validate_project_id(&input.project_id)?;
        if let Some(result) = self
            .replay_operation_result(
                run_id,
                call_id,
                "rollback",
                Some(&input.project_id),
                &input.target_revision_id,
                Some(&input.expected_current_revision_id),
            )
            .await?
        {
            return Ok(result);
        }
        let current = self
            .database
            .current_managed_deployment_revision(&input.project_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read the active deployment"))?
            .ok_or_else(|| ManagedDeployError::invalid("project has no active deployment"))?;
        let target = self
            .database
            .managed_deployment_revision(&input.target_revision_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read the rollback target"))?
            .ok_or_else(|| ManagedDeployError::invalid("rollback target was not found"))?;
        if current.id != input.expected_current_revision_id
            || target.project_id != input.project_id
            || !matches!(target.status.as_str(), "active" | "superseded" | "rolled_back")
        {
            return Err(ManagedDeployError::denied(
                "current revision or rollback target mismatch",
            ));
        }
        self.execute_change(
            run_id,
            call_id,
            "rollback",
            target,
            Some(current.id),
            output_limit,
        )
        .await
    }

    async fn replay_operation_result(
        &self,
        run_id: &str,
        call_id: &str,
        action: &str,
        project_id: Option<&str>,
        revision_id: &str,
        previous_revision_id: Option<&str>,
    ) -> Result<Option<ManagedDeployResult>, ManagedDeployError> {
        let Some(operation) = self
            .database
            .managed_deployment_operation(call_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read the deployment operation"))?
        else {
            return Ok(None);
        };
        if operation.run_id != run_id
            || operation.action != action
            || operation.revision_id != revision_id
            || project_id.is_some_and(|expected| operation.project_id != expected)
            || (action == "rollback" && operation.previous_revision_id.as_deref() != previous_revision_id)
        {
            return Err(ManagedDeployError::denied(
                "deployment operation id was reused with different arguments",
            ));
        }
        if let Some(result) = operation.result {
            if operation.error_category.is_some() {
                return Err(ManagedDeployError::execution(
                    "persisted deployment operation previously failed or was reconciled",
                ));
            }
            return serde_json::from_value(result)
                .map(Some)
                .map_err(|_| ManagedDeployError::internal("persisted deployment result is invalid"));
        }
        if operation.phase != "prepared" {
            return Err(ManagedDeployError::execution(
                "deployment operation has an unresolved outcome and requires reconciliation",
            ));
        }
        Ok(None)
    }

    async fn execute_change(
        &self,
        run_id: &str,
        call_id: &str,
        action: &str,
        revision: soloops_storage::ManagedDeploymentRevision,
        compensation_revision_id: Option<String>,
        output_limit: usize,
    ) -> Result<ManagedDeployResult, ManagedDeployError> {
        let lease_token = Uuid::new_v4().to_string();
        if let Some(existing) = self
            .database
            .managed_deployment_operation(call_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read the deployment operation"))?
        {
            if let Some(result) = existing.result {
                if existing.error_category.is_some() {
                    return Err(ManagedDeployError::execution(
                        "persisted deployment operation previously failed or was reconciled",
                    ));
                }
                return serde_json::from_value(result)
                    .map_err(|_| ManagedDeployError::internal("persisted deployment result is invalid"));
            }
            if existing.phase != "prepared" {
                return Err(ManagedDeployError::denied(
                    "deployment operation requires hostd startup reconciliation",
                ));
            }
        }
        self.database
            .begin_managed_deployment_operation(
                call_id,
                run_id,
                &revision.project_id,
                action,
                &revision.id,
                compensation_revision_id
                    .as_deref()
                    .or(revision.previous_revision_id.as_deref()),
                &lease_token,
                self.config
                    .operation_lease
                    .as_millis()
                    .try_into()
                    .unwrap_or(i64::MAX),
            )
            .await
            .map_err(|error| ManagedDeployError::denied(error.to_string()))?;
        if self
            .database
            .managed_deployment_operation(call_id)
            .await
            .map_err(|_| ManagedDeployError::internal("could not read deployment lease"))?
            .and_then(|operation| operation.lease_token)
            .as_deref()
            != Some(lease_token.as_str())
        {
            return Err(ManagedDeployError::recovery(
                "deployment operation is owned by another executor",
            ));
        }
        let lease = OperationLeaseGuard::start(
            self.database.clone(),
            call_id.to_owned(),
            lease_token.clone(),
            self.config.operation_lease,
            self.config.operation_renew,
        );
        let previous_id = compensation_revision_id
            .as_deref()
            .or(revision.previous_revision_id.as_deref());
        let previous = match previous_id {
            Some(id) => self
                .database
                .managed_deployment_revision(id)
                .await
                .map_err(|_| ManagedDeployError::internal("could not read the previous revision"))?,
            None => None,
        };
        let advanced = self
            .database
            .update_managed_deployment_phase(call_id, &lease_token, "prepared", "applying_compose")
            .await
            .map_err(|_| ManagedDeployError::internal("could not checkpoint deployment"))?;
        if !advanced {
            return Err(ManagedDeployError::denied(
                "deployment operation phase changed concurrently",
            ));
        }
        lease.ensure_owned()?;
        let deployment = self
            .apply_revision(&revision, Some((call_id, lease_token.as_str())), output_limit)
            .await;
        let health = match deployment {
            Ok(health) => health,
            Err(error) => {
                self.ensure_operation_lease(call_id, &lease_token, &lease).await?;
                let rollback = self.compensate(&revision, previous.as_ref(), output_limit).await;
                let rollback_succeeded = rollback.is_ok();
                let result = json!({
                    "action": action,
                    "projectId": revision.project_id,
                    "status": "rolled_back",
                    "revisionId": revision.id,
                    "previousRevisionId": previous_id,
                    "proposalSha256": revision.proposal_sha256,
                    "preview": revision.preview,
                    "failure": error.message,
                    "rollbackSucceeded": rollback_succeeded,
                });
                if rollback_succeeded {
                    self.database
                        .fail_managed_deployment_operation(
                            call_id,
                            &lease_token,
                            "deployment_failed",
                            &result,
                        )
                        .await
                        .map_err(|_| ManagedDeployError::internal("could not persist deployment failure"))?;
                    return Err(ManagedDeployError::execution(
                        "deployment failed and the previous managed revision was restored",
                    ));
                }
                return Err(ManagedDeployError::recovery(
                    "deployment and automatic rollback both failed; the durable operation remains unfinished for hostd reconciliation",
                ));
            }
        };
        self.ensure_operation_lease(call_id, &lease_token, &lease).await?;
        let result = ManagedDeployResult {
            action: action.into(),
            project_id: revision.project_id.clone(),
            status: "active".into(),
            revision_id: Some(revision.id.clone()),
            previous_revision_id: previous_id.map(str::to_owned),
            proposal_sha256: Some(revision.proposal_sha256.clone()),
            preview: Some(revision.preview.clone()),
        };
        self.database
            .finish_managed_deployment_operation(
                call_id,
                &lease_token,
                &serde_json::to_value(&result)
                    .map_err(|_| ManagedDeployError::internal("could not encode deployment result"))?,
                &health,
            )
            .await
            .map_err(|_| ManagedDeployError::internal("could not commit deployment state"))?;
        Ok(result)
    }

    async fn apply_revision(
        &self,
        revision: &soloops_storage::ManagedDeploymentRevision,
        operation: Option<(&str, &str)>,
        output_limit: usize,
    ) -> Result<Value, ManagedDeployError> {
        self.validate_revision_bundle(revision, output_limit).await?;
        let bundle = PathBuf::from(&revision.bundle_path);
        let compose_file = bundle.join("compose.yaml");
        let compose_arguments = vec![
            "compose".into(),
            "--project-name".into(),
            format!("soloops-{}", revision.project_id),
            "--project-directory".into(),
            bundle.to_string_lossy().into_owned(),
            "-f".into(),
            compose_file.to_string_lossy().into_owned(),
            "up".into(),
            "-d".into(),
            "--pull".into(),
            "never".into(),
            "--remove-orphans".into(),
            "--wait".into(),
            "--wait-timeout".into(),
            self.config.health_timeout.as_secs().to_string(),
        ];
        let compose = self
            .runner
            .run(
                &self.config.docker_cli,
                &compose_arguments,
                self.config.health_timeout + Duration::from_secs(30),
                output_limit,
            )
            .await?;
        if let Some((call_id, lease_token)) = operation
            && !self
                .database
                .update_managed_deployment_phase(call_id, lease_token, "applying_compose", "applying_caddy")
                .await
                .map_err(|_| ManagedDeployError::internal("could not checkpoint Caddy apply"))?
        {
            return Err(ManagedDeployError::denied(
                "deployment operation phase changed before Caddy apply",
            ));
        }
        self.apply_caddy_fragment(revision, output_limit).await?;
        if let Some((call_id, lease_token)) = operation
            && !self
                .database
                .update_managed_deployment_phase(call_id, lease_token, "applying_caddy", "verifying_health")
                .await
                .map_err(|_| ManagedDeployError::internal("could not checkpoint health verification"))?
        {
            return Err(ManagedDeployError::denied(
                "deployment operation phase changed before health verification",
            ));
        }
        let site = revision.preview["site"]
            .as_str()
            .ok_or_else(|| ManagedDeployError::internal("deployment preview has no site"))?;
        let health_path = revision.source["healthPath"].as_str().unwrap_or("/");
        validate_health_path(health_path)?;
        let mut url = reqwest::Url::parse(&self.config.health_origin)
            .map_err(|_| ManagedDeployError::internal("managed health origin is invalid"))?;
        url.set_path(health_path);
        let response = probe_health(&self.http, url, site, self.config.health_timeout).await?;
        if !response.status().is_success() {
            return Err(ManagedDeployError::execution(format!(
                "managed route health check returned {}",
                response.status()
            )));
        }
        Ok(json!({
            "composeHealthy": true,
            "routeStatus": response.status().as_u16(),
            "composeOutputSha256": sha256(compose.stdout.as_bytes()),
            "composeStderrSha256": sha256(compose.stderr.as_bytes()),
        }))
    }

    async fn apply_caddy_fragment(
        &self,
        revision: &soloops_storage::ManagedDeploymentRevision,
        output_limit: usize,
    ) -> Result<(), ManagedDeployError> {
        let target = self
            .config
            .caddy_managed_dir
            .join(format!("{}.caddy", revision.project_id));
        let temporary = self
            .config
            .caddy_managed_dir
            .join(format!(".{}.{}.tmp", revision.project_id, revision.id));
        fs::copy(
            PathBuf::from(&revision.bundle_path).join("site.caddy"),
            &temporary,
        )
        .await
        .map_err(|_| ManagedDeployError::execution("could not stage the Caddy fragment"))?;
        fs::rename(&temporary, &target)
            .await
            .map_err(|_| ManagedDeployError::execution("could not publish the Caddy fragment"))?;
        let config_path = self.config.caddy_config.to_string_lossy().into_owned();
        self.runner
            .run(
                &self.config.caddy_cli,
                &["validate".into(), "--config".into(), config_path.clone()],
                Duration::from_secs(30),
                output_limit,
            )
            .await?;
        self.runner
            .run(
                &self.config.caddy_cli,
                &["reload".into(), "--config".into(), config_path],
                Duration::from_secs(30),
                output_limit,
            )
            .await?;
        Ok(())
    }

    async fn compensate(
        &self,
        failed: &soloops_storage::ManagedDeploymentRevision,
        previous: Option<&soloops_storage::ManagedDeploymentRevision>,
        output_limit: usize,
    ) -> Result<(), ManagedDeployError> {
        if let Some(previous) = previous {
            let _ = self.apply_revision(previous, None, output_limit).await?;
            return Ok(());
        }
        self.validate_revision_bundle(failed, output_limit).await?;
        let bundle = PathBuf::from(&failed.bundle_path);
        self.runner
            .run(
                &self.config.docker_cli,
                &[
                    "compose".into(),
                    "--project-name".into(),
                    format!("soloops-{}", failed.project_id),
                    "--project-directory".into(),
                    bundle.to_string_lossy().into_owned(),
                    "-f".into(),
                    bundle.join("compose.yaml").to_string_lossy().into_owned(),
                    "down".into(),
                    "--remove-orphans".into(),
                ],
                Duration::from_secs(60),
                output_limit,
            )
            .await?;
        let target = self
            .config
            .caddy_managed_dir
            .join(format!("{}.caddy", failed.project_id));
        match fs::remove_file(target).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(ManagedDeployError::execution(
                    "could not remove failed Caddy fragment",
                ));
            }
        }
        let config_path = self.config.caddy_config.to_string_lossy().into_owned();
        self.runner
            .run(
                &self.config.caddy_cli,
                &["validate".into(), "--config".into(), config_path.clone()],
                Duration::from_secs(30),
                output_limit,
            )
            .await?;
        self.runner
            .run(
                &self.config.caddy_cli,
                &["reload".into(), "--config".into(), config_path],
                Duration::from_secs(30),
                output_limit,
            )
            .await?;
        Ok(())
    }

    async fn validate_revision_bundle(
        &self,
        revision: &soloops_storage::ManagedDeploymentRevision,
        output_limit: usize,
    ) -> Result<(), ManagedDeployError> {
        let bundle = PathBuf::from(&revision.bundle_path);
        let compose_file = bundle.join("compose.yaml");
        let caddy_file = bundle.join("site.caddy");
        let compose_bytes = fs::read(&compose_file)
            .await
            .map_err(|_| ManagedDeployError::denied("managed Compose bundle is missing"))?;
        let caddy_bytes = fs::read(&caddy_file)
            .await
            .map_err(|_| ManagedDeployError::denied("managed Caddy bundle is missing"))?;
        if sha256(&compose_bytes) != revision.compose_sha256 || sha256(&caddy_bytes) != revision.caddy_sha256
        {
            return Err(ManagedDeployError::denied(
                "managed deployment bundle digest mismatch",
            ));
        }
        validate_compose_source(&compose_bytes, &self.config)?;
        let normalized = self
            .runner
            .run(
                &self.config.docker_cli,
                &[
                    "compose".into(),
                    "--project-directory".into(),
                    bundle.to_string_lossy().into_owned(),
                    "-f".into(),
                    compose_file.to_string_lossy().into_owned(),
                    "config".into(),
                    "--format".into(),
                    "json".into(),
                ],
                Duration::from_secs(30),
                output_limit,
            )
            .await?;
        let compose: Value = serde_json::from_str(&normalized.stdout)
            .map_err(|_| ManagedDeployError::denied("docker compose returned invalid normalized JSON"))?;
        let preview = validate_compose(&compose, &self.config)?;
        let caddy = String::from_utf8(caddy_bytes)
            .map_err(|_| ManagedDeployError::denied("Caddy fragment must be UTF-8"))?;
        validate_caddy(&caddy, &preview, &self.config)?;
        validate_health_path(revision.source["healthPath"].as_str().unwrap_or("/"))?;
        Ok(())
    }
}

async fn probe_health(
    client: &reqwest::Client,
    url: reqwest::Url,
    site: &str,
    timeout: Duration,
) -> Result<reqwest::Response, ManagedDeployError> {
    tokio::time::timeout(timeout, client.get(url).header(HOST, site).send())
        .await
        .map_err(|_| ManagedDeployError::execution("managed route health check timed out"))?
        .map_err(|_| ManagedDeployError::execution("managed route health check failed"))
}

fn result_from_revision(
    action: &str,
    revision: &soloops_storage::ManagedDeploymentRevision,
    preview: Option<Value>,
) -> ManagedDeployResult {
    ManagedDeployResult {
        action: action.into(),
        project_id: revision.project_id.clone(),
        status: revision.status.clone(),
        revision_id: Some(revision.id.clone()),
        previous_revision_id: revision.previous_revision_id.clone(),
        proposal_sha256: Some(revision.proposal_sha256.clone()),
        preview,
    }
}

fn validate_compose(compose: &Value, config: &ManagedDeployConfig) -> Result<Value, ManagedDeployError> {
    let services = compose["services"]
        .as_object()
        .ok_or_else(|| ManagedDeployError::denied("Compose must define services"))?;
    if services.is_empty() {
        return Err(ManagedDeployError::denied(
            "Compose must define at least one service",
        ));
    }
    if compose.get("include").is_some_and(|value| !value.is_null()) {
        return Err(ManagedDeployError::denied("Compose include is forbidden"));
    }
    reject_external_resources(&compose["networks"], "network")?;
    reject_external_resources(&compose["volumes"], "volume")?;
    if compose
        .get("secrets")
        .and_then(Value::as_object)
        .is_some_and(|items| !items.is_empty())
        || compose
            .get("configs")
            .and_then(Value::as_object)
            .is_some_and(|items| !items.is_empty())
    {
        return Err(ManagedDeployError::denied(
            "Compose secrets and configs are not supported by the MVP",
        ));
    }
    let mut images = Vec::new();
    let mut ports = Vec::new();
    let mut volumes = Vec::new();
    for (name, service) in services {
        let object = service
            .as_object()
            .ok_or_else(|| ManagedDeployError::denied("Compose service must be an object"))?;
        for forbidden in [
            "build",
            "devices",
            "cap_add",
            "container_name",
            "network_mode",
            "pid",
            "ipc",
            "extra_hosts",
            "env_file",
            "environment",
            "secrets",
            "configs",
            "sysctls",
            "extends",
        ] {
            if object.get(forbidden).is_some_and(|value| !value.is_null()) {
                return Err(ManagedDeployError::denied(format!(
                    "Compose service {name} uses forbidden {forbidden}"
                )));
            }
        }
        if object.get("privileged").and_then(Value::as_bool) == Some(true) {
            return Err(ManagedDeployError::denied(format!(
                "Compose service {name} requests host privileges"
            )));
        }
        let image = object
            .get("image")
            .and_then(Value::as_str)
            .ok_or_else(|| ManagedDeployError::denied(format!("service {name} has no image")))?;
        if !is_digest_pinned(image) {
            return Err(ManagedDeployError::denied(format!(
                "service {name} image is not pinned by SHA-256 digest"
            )));
        }
        images.push(image.to_owned());
        let health = object.get("healthcheck").and_then(Value::as_object);
        if health.is_none()
            || health
                .and_then(|item| item.get("disable"))
                .and_then(Value::as_bool)
                == Some(true)
        {
            return Err(ManagedDeployError::denied(format!(
                "service {name} must define an enabled healthcheck"
            )));
        }
        if let Some(items) = object.get("volumes").and_then(Value::as_array) {
            for item in items {
                let source = item
                    .get("source")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str().and_then(|value| value.split(':').next()));
                if let Some(source) = source {
                    if source.contains('/') || source.contains('\\') || source.starts_with('.') {
                        return Err(ManagedDeployError::denied("host bind mounts are forbidden"));
                    }
                    volumes.push(source.to_owned());
                }
            }
        }
        if let Some(items) = object.get("ports").and_then(Value::as_array) {
            for item in items {
                let host_ip = item.get("host_ip").and_then(Value::as_str).unwrap_or("");
                let published = item
                    .get("published")
                    .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
                    .and_then(|value| u16::try_from(value).ok())
                    .ok_or_else(|| ManagedDeployError::denied("published ports must be explicit"))?;
                if !matches!(host_ip, "127.0.0.1" | "::1") || !config.allowed_ports.contains(&published) {
                    return Err(ManagedDeployError::denied(format!(
                        "published port {host_ip}:{published} is not allowlisted loopback"
                    )));
                }
                ports.push(published);
            }
        }
    }
    images.sort();
    images.dedup();
    ports.sort_unstable();
    ports.dedup();
    volumes.sort();
    volumes.dedup();
    Ok(json!({
        "services": services.keys().cloned().collect::<Vec<_>>(),
        "images": images,
        "ports": ports,
        "volumes": volumes,
    }))
}

fn validate_caddy(
    source: &str,
    compose_preview: &Value,
    config: &ManagedDeployConfig,
) -> Result<Value, ManagedDeployError> {
    let lowered = source.to_ascii_lowercase();
    for forbidden in ["import ", "file_server", "php_fastcgi", "exec ", "admin "] {
        if lowered.contains(forbidden) {
            return Err(ManagedDeployError::denied(format!(
                "Caddy fragment contains forbidden directive {forbidden}"
            )));
        }
    }
    let lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    let header = lines
        .first()
        .ok_or_else(|| ManagedDeployError::denied("Caddy fragment has no site"))?;
    let header_tokens = header.split_whitespace().collect::<Vec<_>>();
    if header_tokens.len() != 2 || header_tokens[1] != "{" {
        return Err(ManagedDeployError::denied(
            "Caddy site header must contain exactly one site and an opening brace",
        ));
    }
    let site = header_tokens[0];
    if !config.allowed_sites.contains(site) {
        return Err(ManagedDeployError::denied("Caddy site is not allowlisted"));
    }
    let allowed_ports = compose_preview["ports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect::<HashSet<_>>();
    let mut routes = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        if index == 0 || matches!(line, "{" | "}") {
            continue;
        }
        if matches!(line, "encode gzip" | "encode zstd" | "encode zstd gzip") {
            continue;
        }
        let Some(target) = line.strip_prefix("reverse_proxy ") else {
            return Err(ManagedDeployError::denied(format!(
                "Caddy directive is outside the managed MVP allowlist: {line}"
            )));
        };
        let target_tokens = target.split_whitespace().collect::<Vec<_>>();
        if target_tokens.len() != 1 {
            return Err(ManagedDeployError::denied(
                "reverse_proxy must contain exactly one upstream",
            ));
        }
        let target = target_tokens[0];
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| ManagedDeployError::denied("reverse_proxy target must include a port"))?;
        let port = port
            .parse::<u64>()
            .map_err(|_| ManagedDeployError::denied("reverse_proxy target port is invalid"))?;
        if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") || !allowed_ports.contains(&port) {
            return Err(ManagedDeployError::denied(
                "reverse_proxy must target an allowlisted Compose loopback port",
            ));
        }
        routes.push(target.to_owned());
    }
    if routes.is_empty() {
        return Err(ManagedDeployError::denied(
            "Caddy fragment must contain reverse_proxy",
        ));
    }
    Ok(json!({"site": site, "routes": routes}))
}

fn reject_external_resources(value: &Value, kind: &str) -> Result<(), ManagedDeployError> {
    if let Some(items) = value.as_object() {
        for (name, item) in items {
            let allowed_driver = match kind {
                "volume" => item
                    .get("driver")
                    .and_then(Value::as_str)
                    .is_none_or(|driver| driver == "local"),
                "network" => item
                    .get("driver")
                    .and_then(Value::as_str)
                    .is_none_or(|driver| matches!(driver, "bridge" | "none")),
                _ => true,
            };
            if !allowed_driver {
                return Err(ManagedDeployError::denied(format!(
                    "Compose {kind} {name} uses a forbidden driver"
                )));
            }
            if item.get("driver_opts").is_some_and(|value| !value.is_null()) {
                return Err(ManagedDeployError::denied(format!(
                    "Compose {kind} {name} driver_opts are forbidden"
                )));
            }
            if item.get("external").and_then(Value::as_bool) == Some(true) {
                return Err(ManagedDeployError::denied(format!(
                    "external Compose {kind} {name} is forbidden"
                )));
            }
            if item.get("name").and_then(Value::as_str).is_some() {
                return Err(ManagedDeployError::denied(format!(
                    "explicit global Compose {kind} name for {name} is forbidden"
                )));
            }
        }
    }
    Ok(())
}

fn validate_health_path(path: &str) -> Result<(), ManagedDeployError> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains("..")
        || path.contains(['\\', '#', '?'])
        || path.chars().any(char::is_control)
    {
        return Err(ManagedDeployError::denied(
            "healthPath must be an absolute safe path without an authority, query, or fragment",
        ));
    }
    Ok(())
}

fn validate_compose_source(source: &[u8], config: &ManagedDeployConfig) -> Result<(), ManagedDeployError> {
    let mut documents = serde_yaml_ng::Deserializer::from_slice(source);
    let first = documents
        .next()
        .ok_or_else(|| ManagedDeployError::denied("Compose source is empty"))?;
    let mut yaml = serde_yaml_ng::Value::deserialize(first)
        .map_err(|_| ManagedDeployError::denied("Compose source is invalid YAML"))?;
    if documents.next().is_some() {
        return Err(ManagedDeployError::denied(
            "Compose source must contain exactly one YAML document",
        ));
    }
    yaml.apply_merge()
        .map_err(|_| ManagedDeployError::denied("Compose YAML merge keys are invalid"))?;
    let value = serde_json::to_value(yaml)
        .map_err(|_| ManagedDeployError::denied("Compose YAML keys must be strings"))?;
    let root = value
        .as_object()
        .ok_or_else(|| ManagedDeployError::denied("Compose source must be an object"))?;
    if root.get("include").is_some_and(|value| !value.is_null()) {
        return Err(ManagedDeployError::denied("Compose include is forbidden"));
    }
    if root
        .get("secrets")
        .and_then(Value::as_object)
        .is_some_and(|items| !items.is_empty())
        || root
            .get("configs")
            .and_then(Value::as_object)
            .is_some_and(|items| !items.is_empty())
    {
        return Err(ManagedDeployError::denied(
            "Compose secrets and configs are forbidden before normalization",
        ));
    }
    reject_external_resources(root.get("networks").unwrap_or(&Value::Null), "network")?;
    reject_external_resources(root.get("volumes").unwrap_or(&Value::Null), "volume")?;
    let services = root
        .get("services")
        .and_then(Value::as_object)
        .ok_or_else(|| ManagedDeployError::denied("Compose must define services"))?;
    for (name, service) in services {
        let service = service
            .as_object()
            .ok_or_else(|| ManagedDeployError::denied("Compose service must be an object"))?;
        for forbidden in [
            "extends",
            "env_file",
            "secrets",
            "configs",
            "label_file",
            "provider",
        ] {
            if service.get(forbidden).is_some_and(|value| !value.is_null()) {
                return Err(ManagedDeployError::denied(format!(
                    "Compose service {name} uses forbidden {forbidden}"
                )));
            }
        }
    }
    let _ = config;
    Ok(())
}

fn is_digest_pinned(image: &str) -> bool {
    let Some((_, digest)) = image.rsplit_once("@sha256:") else {
        return false;
    };
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn resolve_workspace(root: &Path, run_id: &str) -> Result<PathBuf, ManagedDeployError> {
    if run_id.is_empty() || run_id.contains(['/', '\\']) {
        return Err(ManagedDeployError::invalid("run ID is invalid"));
    }
    let path = root.join(run_id);
    fs::canonicalize(path)
        .await
        .map_err(|_| ManagedDeployError::invalid("run workspace does not exist"))
}

async fn resolve_workspace_file(root: &Path, relative: &str) -> Result<PathBuf, ManagedDeployError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ManagedDeployError::denied(
            "deployment source path escapes the Workspace",
        ));
    }
    let canonical = fs::canonicalize(root.join(path))
        .await
        .map_err(|_| ManagedDeployError::invalid("deployment source path does not exist"))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(ManagedDeployError::denied(
            "deployment source path escapes the Workspace",
        ));
    }
    Ok(canonical)
}

fn validate_project_id(value: &str) -> Result<(), ManagedDeployError> {
    if value.is_empty()
        || value.len() > 63
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(ManagedDeployError::invalid("projectId is invalid"));
    }
    Ok(())
}

fn sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn parse_bool(value: &str) -> Result<bool> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => anyhow::bail!("expected boolean value"),
    }
}

fn absolute_path(name: &str, default: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env(name, default));
    if !path.is_absolute() {
        anyhow::bail!("{name} must be absolute");
    }
    Ok(path)
}

fn existing_file(name: &str, default: &str) -> Result<PathBuf> {
    let path = absolute_path(name, default)?;
    if !path.is_file() {
        anyhow::bail!("{name} must point to an existing file");
    }
    Ok(path)
}

fn parse_string_set(name: &str) -> Result<HashSet<String>> {
    serde_json::from_str::<HashSet<String>>(&env(name, "[]"))
        .with_context(|| format!("{name} must be a JSON string array"))
}

fn parse_port_set(name: &str) -> Result<HashSet<u16>> {
    serde_json::from_str::<HashSet<u16>>(&env(name, "[]"))
        .with_context(|| format!("{name} must be a JSON TCP port array"))
}

fn parsed_u64(name: &str, default: u64) -> Result<u64> {
    env(name, &default.to_string())
        .parse::<u64>()
        .with_context(|| format!("{name} must be an unsigned integer"))
}

fn parsed_range(name: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64> {
    let value = parsed_u64(name, default)?;
    if !(minimum..=maximum).contains(&value) {
        anyhow::bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    use soloops_domain::{BudgetSnapshot, CreateTaskRequest, PolicyDecision, ToolRisk, UsageSnapshot};
    use soloops_storage::{NewToolCall, PersistModelResponse, RuntimeSessionConfig};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn config() -> ManagedDeployConfig {
        ManagedDeployConfig {
            managed_root: "/managed".into(),
            docker_cli: "/usr/bin/docker".into(),
            caddy_cli: "/usr/bin/caddy".into(),
            caddy_config: "/etc/caddy/Caddyfile".into(),
            caddy_managed_dir: "/etc/caddy/soloops.d".into(),
            allowed_sites: HashSet::from(["app.example.test".into()]),
            allowed_ports: HashSet::from([8080]),
            health_origin: "http://127.0.0.1".into(),
            health_timeout: Duration::from_secs(60),
            operation_lease: Duration::from_secs(60),
            operation_renew: Duration::from_secs(10),
            reconcile_interval: Duration::from_secs(15),
        }
    }

    #[test]
    fn rejects_unpinned_images_and_host_privileges() {
        let unpinned = json!({"services": {"web": {"image": "demo:latest", "healthcheck": {}, "ports": []}}});
        assert!(validate_compose(&unpinned, &config()).is_err());
        let privileged = json!({"services": {"web": {
            "image": format!("demo@sha256:{}", "a".repeat(64)),
            "healthcheck": {}, "privileged": true, "ports": []
        }}});
        assert!(validate_compose(&privileged, &config()).is_err());
    }

    #[test]
    fn accepts_bounded_compose_and_caddy_pair() {
        let compose = json!({"services": {"web": {
            "image": format!("demo@sha256:{}", "a".repeat(64)),
            "healthcheck": {"test": ["CMD", "true"]},
            "ports": [{"host_ip": "127.0.0.1", "published": 8080, "target": 8080}]
        }}});
        let preview = validate_compose(&compose, &config()).unwrap();
        let caddy = validate_caddy(
            "app.example.test {\n reverse_proxy 127.0.0.1:8080\n}\n",
            &preview,
            &config(),
        )
        .unwrap();
        assert_eq!(caddy["site"], "app.example.test");
        assert!(validate_caddy("app.example.test { respond / 200\n}\n", &preview, &config(),).is_err());
        assert!(
            validate_caddy(
                "app.example.test {\n reverse_proxy 127.0.0.1:8080 127.0.0.1:8080\n}\n",
                &preview,
                &config(),
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_unsafe_top_level_volume_and_network_drivers() {
        let image = format!("demo@sha256:{}", "a".repeat(64));
        let service = json!({
            "image": image,
            "healthcheck": {"test": ["CMD", "true"]},
            "ports": []
        });
        let bind = json!({
            "services": {"web": service.clone()},
            "volumes": {"data": {"driver": "local", "driver_opts": {
                "type": "none", "o": "bind", "device": "/etc"
            }}}
        });
        assert!(validate_compose(&bind, &config()).is_err());

        let remote_volume = json!({
            "services": {"web": service.clone()},
            "volumes": {"data": {"driver": "nfs"}}
        });
        assert!(validate_compose(&remote_volume, &config()).is_err());

        let macvlan = json!({
            "services": {"web": service.clone()},
            "networks": {"lan": {"driver": "macvlan"}}
        });
        assert!(validate_compose(&macvlan, &config()).is_err());

        let network_options = json!({
            "services": {"web": service},
            "networks": {"default": {"driver": "bridge", "driver_opts": {"parent": "eth0"}}}
        });
        assert!(validate_compose(&network_options, &config()).is_err());
    }

    #[test]
    fn raw_compose_preflight_rejects_file_loading_before_docker() {
        let digest = "a".repeat(64);
        let include =
            format!("include: /etc/compose.yaml\nservices:\n  web:\n    image: demo@sha256:{digest}\n");
        assert!(validate_compose_source(include.as_bytes(), &config()).is_err());

        let extends = format!(
            "services:\n  web:\n    image: demo@sha256:{digest}\n    extends: &external\n      file: /etc/compose.yaml\n      service: base\n  copy:\n    image: demo@sha256:{digest}\n    extends: *external\n"
        );
        assert!(validate_compose_source(extends.as_bytes(), &config()).is_err());

        let merged = format!(
            "x-base: &base\n  extends:\n    file: /etc/compose.yaml\n    service: base\nservices:\n  web:\n    <<: *base\n    image: demo@sha256:{digest}\n"
        );
        assert!(validate_compose_source(merged.as_bytes(), &config()).is_err());
    }

    #[test]
    fn validates_health_path_as_a_path_only() {
        assert!(validate_health_path("/").is_ok());
        assert!(validate_health_path("/health/live").is_ok());
        for invalid in ["health", "//169.254.169.254/", "/a?b", "/a#b", "/a\\b", "/a\nb"] {
            assert!(validate_health_path(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[tokio::test]
    async fn health_probe_does_not_follow_redirects() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
        let target_hit = Arc::new(AtomicBool::new(false));
        let hit = target_hit.clone();
        let target_task = tokio::spawn(async move {
            if let Ok(Ok((_stream, _))) =
                tokio::time::timeout(Duration::from_millis(250), target.accept()).await
            {
                hit.store(true, Ordering::Relaxed);
            }
        });

        let redirect = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_address = redirect.local_addr().unwrap();
        let redirect_task = tokio::spawn(async move {
            let (mut stream, _) = redirect.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{target_address}/metadata\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .unwrap();
        let response = probe_health(
            &client,
            reqwest::Url::parse(&format!("http://{redirect_address}/health")).unwrap(),
            "app.example.test",
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        redirect_task.await.unwrap();
        target_task.await.unwrap();
        assert!(!target_hit.load(Ordering::Relaxed));
    }

    #[tokio::test]
    #[ignore = "requires operator-provided preloaded images and disposable Compose files"]
    async fn live_compose_failure_can_restore_the_previous_revision_without_pull() {
        let docker = PathBuf::from(
            std::env::var("SOLOOPS_TEST_MANAGED_DOCKER_CLI").unwrap_or_else(|_| "/usr/bin/docker".into()),
        );
        let good = PathBuf::from(
            std::env::var("SOLOOPS_TEST_MANAGED_COMPOSE_GOOD")
                .expect("SOLOOPS_TEST_MANAGED_COMPOSE_GOOD is required"),
        );
        let bad = PathBuf::from(
            std::env::var("SOLOOPS_TEST_MANAGED_COMPOSE_BAD")
                .expect("SOLOOPS_TEST_MANAGED_COMPOSE_BAD is required"),
        );
        let project = std::env::var("SOLOOPS_TEST_MANAGED_PROJECT")
            .unwrap_or_else(|_| "soloops-managed-live-test".into());
        let runner = SystemCommandRunner;
        let apply = |file: &Path| {
            vec![
                "compose".into(),
                "--project-name".into(),
                project.clone(),
                "-f".into(),
                file.to_string_lossy().into_owned(),
                "up".into(),
                "-d".into(),
                "--pull".into(),
                "never".into(),
                "--remove-orphans".into(),
                "--wait".into(),
                "--wait-timeout".into(),
                "30".into(),
            ]
        };
        runner
            .run(&docker, &apply(&good), Duration::from_secs(60), 256 * 1024)
            .await
            .expect("known-good revision deploys");
        assert!(
            runner
                .run(&docker, &apply(&bad), Duration::from_secs(60), 256 * 1024)
                .await
                .is_err(),
            "unhealthy revision must fail"
        );
        runner
            .run(&docker, &apply(&good), Duration::from_secs(60), 256 * 1024)
            .await
            .expect("known-good revision restores after failure");
        runner
            .run(
                &docker,
                &[
                    "compose".into(),
                    "--project-name".into(),
                    project,
                    "-f".into(),
                    good.to_string_lossy().into_owned(),
                    "down".into(),
                    "--remove-orphans".into(),
                ],
                Duration::from_secs(60),
                256 * 1024,
            )
            .await
            .expect("live deployment cleanup succeeds");
    }

    #[tokio::test]
    #[ignore = "requires Docker Compose, a running disposable Caddy instance, and a preloaded digest-pinned image"]
    async fn live_managed_deployment_verifies_route_and_restores_the_active_revision() {
        let docker = PathBuf::from(
            std::env::var("SOLOOPS_TEST_MANAGED_DOCKER_CLI").unwrap_or_else(|_| "/usr/bin/docker".into()),
        );
        let caddy = PathBuf::from(
            std::env::var("SOLOOPS_TEST_MANAGED_CADDY_CLI").unwrap_or_else(|_| "/usr/bin/caddy".into()),
        );
        let caddy_config = PathBuf::from(
            std::env::var("SOLOOPS_TEST_MANAGED_CADDY_CONFIG")
                .expect("SOLOOPS_TEST_MANAGED_CADDY_CONFIG is required"),
        );
        let caddy_managed_dir = PathBuf::from(
            std::env::var("SOLOOPS_TEST_MANAGED_CADDY_DIR")
                .expect("SOLOOPS_TEST_MANAGED_CADDY_DIR is required"),
        );
        let image = std::env::var("SOLOOPS_TEST_MANAGED_IMAGE")
            .expect("SOLOOPS_TEST_MANAGED_IMAGE must be preloaded and digest-pinned");
        assert!(is_digest_pinned(&image));
        let app_port = std::env::var("SOLOOPS_TEST_MANAGED_APP_PORT")
            .unwrap_or_else(|_| "18080".into())
            .parse::<u16>()
            .unwrap();
        let caddy_port = std::env::var("SOLOOPS_TEST_MANAGED_CADDY_PORT")
            .unwrap_or_else(|_| "19090".into())
            .parse::<u16>()
            .unwrap();
        let site = format!("soloops-release.test:{caddy_port}");
        let project_id = "release-acceptance";
        let root = tempfile::tempdir().unwrap();
        let workspace_root = root.path().join("workspaces");
        let managed_root = root.path().join("managed");
        let database = Database::connect(root.path().join("release-acceptance.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let owner = database.create_owner("release-owner", "hash").await.unwrap();
        let task = database
            .create_task_with_run(
                &owner.id,
                &CreateTaskRequest {
                    title: "Managed deployment release acceptance".into(),
                    goal: "Exercise the real Docker and Caddy deployment Saga".into(),
                },
            )
            .await
            .unwrap();
        let run_id = task.latest_run_id;
        database
            .claim_next_run("release-acceptance-worker", 120_000)
            .await
            .unwrap()
            .unwrap();
        let workspace = workspace_root.join(&run_id);
        fs::create_dir_all(&workspace).await.unwrap();
        database
            .initialize_runtime(
                &run_id,
                &RuntimeSessionConfig {
                    provider: "release-test".into(),
                    model: "release-test".into(),
                    prompt_version: "release-test".into(),
                    workspace_path: workspace.to_string_lossy().into_owned(),
                    artifact_path: root.path().join("artifacts").to_string_lossy().into_owned(),
                    budget: BudgetSnapshot::default(),
                },
            )
            .await
            .unwrap();
        let attempt = database
            .prepare_model_attempt(&run_id, "release-acceptance", 1)
            .await
            .unwrap();
        database
            .persist_model_response(
                &run_id,
                PersistModelResponse {
                    attempt_id: &attempt,
                    provider_request_id: None,
                    items: &[],
                    calls: &[
                        NewToolCall {
                            call_id: "release-plan-good".into(),
                            ordinal: 0,
                            name: "managed.deploy.plan".into(),
                            arguments: json!({}),
                            risk: ToolRisk::ReadOnly,
                            policy: PolicyDecision::Allow,
                        },
                        NewToolCall {
                            call_id: "release-apply-good".into(),
                            ordinal: 1,
                            name: "managed.deploy.apply".into(),
                            arguments: json!({}),
                            risk: ToolRisk::Privileged,
                            policy: PolicyDecision::Allow,
                        },
                        NewToolCall {
                            call_id: "release-plan-bad".into(),
                            ordinal: 2,
                            name: "managed.deploy.plan".into(),
                            arguments: json!({}),
                            risk: ToolRisk::ReadOnly,
                            policy: PolicyDecision::Allow,
                        },
                        NewToolCall {
                            call_id: "release-apply-bad".into(),
                            ordinal: 3,
                            name: "managed.deploy.apply".into(),
                            arguments: json!({}),
                            risk: ToolRisk::Privileged,
                            policy: PolicyDecision::Allow,
                        },
                    ],
                    usage: &UsageSnapshot::default(),
                    made_progress: true,
                },
            )
            .await
            .unwrap();

        let good_compose = format!(
            "services:\n  web:\n    image: {image}\n    command: [\"sh\", \"-c\", \"mkdir -p /www && echo healthy > /www/health && httpd -f -p 8080 -h /www\"]\n    healthcheck:\n      test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"-\", \"http://127.0.0.1:8080/health\"]\n      interval: 1s\n      timeout: 1s\n      retries: 10\n    ports:\n      - \"127.0.0.1:{app_port}:8080\"\n"
        );
        let bad_compose = format!(
            "services:\n  web:\n    image: {image}\n    command: [\"sh\", \"-c\", \"sleep 300\"]\n    healthcheck:\n      test: [\"CMD\", \"false\"]\n      interval: 1s\n      timeout: 1s\n      retries: 2\n    ports:\n      - \"127.0.0.1:{app_port}:8080\"\n"
        );
        let caddy_fragment = format!("{site} {{\n  reverse_proxy 127.0.0.1:{app_port}\n}}\n");
        fs::write(workspace.join("compose-good.yaml"), good_compose)
            .await
            .unwrap();
        fs::write(workspace.join("compose-bad.yaml"), bad_compose)
            .await
            .unwrap();
        fs::write(workspace.join("site.caddy"), caddy_fragment)
            .await
            .unwrap();

        let service = ManagedDeploymentService::new(
            database.clone(),
            workspace_root,
            ManagedDeployConfig {
                managed_root,
                docker_cli: docker.clone(),
                caddy_cli: caddy.clone(),
                caddy_config: caddy_config.clone(),
                caddy_managed_dir: caddy_managed_dir.clone(),
                allowed_sites: HashSet::from([site.clone()]),
                allowed_ports: HashSet::from([app_port]),
                health_origin: format!("http://127.0.0.1:{caddy_port}"),
                health_timeout: Duration::from_secs(10),
                operation_lease: Duration::from_secs(60),
                operation_renew: Duration::from_secs(10),
                reconcile_interval: Duration::from_secs(15),
            },
        );
        let good = service
            .plan(
                &run_id,
                "release-plan-good",
                json!({
                    "projectId": project_id,
                    "composePath": "compose-good.yaml",
                    "caddyFragmentPath": "site.caddy",
                    "healthPath": "/health",
                }),
                256 * 1024,
            )
            .await
            .unwrap();
        let good_id = good.revision_id.clone().unwrap();
        let good_result = service
            .apply(
                &run_id,
                "release-apply-good",
                json!({
                    "proposalId": good_id,
                    "proposalSha256": good.proposal_sha256,
                }),
                256 * 1024,
            )
            .await
            .unwrap();
        assert_eq!(good_result.status, "active");
        assert_eq!(
            service
                .status(json!({"projectId": project_id}))
                .await
                .unwrap()
                .revision_id,
            Some(good_id.clone())
        );

        let bad = service
            .plan(
                &run_id,
                "release-plan-bad",
                json!({
                    "projectId": project_id,
                    "composePath": "compose-bad.yaml",
                    "caddyFragmentPath": "site.caddy",
                    "healthPath": "/health",
                }),
                256 * 1024,
            )
            .await
            .unwrap();
        let bad_id = bad.revision_id.clone().unwrap();
        let error = service
            .apply(
                &run_id,
                "release-apply-bad",
                json!({
                    "proposalId": bad_id,
                    "proposalSha256": bad.proposal_sha256,
                }),
                256 * 1024,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, HostdErrorCode::Execution);
        assert_eq!(
            service
                .status(json!({"projectId": project_id}))
                .await
                .unwrap()
                .revision_id,
            Some(good_id.clone())
        );
        let failed_operation = database
            .managed_deployment_operation("release-apply-bad")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            failed_operation.error_category.as_deref(),
            Some("deployment_failed")
        );
        assert_eq!(
            failed_operation.result.as_ref().unwrap()["rollbackSucceeded"],
            true
        );

        let good_revision = database
            .managed_deployment_revision(&good_id)
            .await
            .unwrap()
            .unwrap();
        let runner = SystemCommandRunner;
        service
            .compensate(&good_revision, None, 256 * 1024)
            .await
            .unwrap();
        service
            .compensate(&good_revision, None, 256 * 1024)
            .await
            .unwrap();
        let containers = runner
            .run(
                &docker,
                &[
                    "ps".into(),
                    "-aq".into(),
                    "--filter".into(),
                    format!("label=com.docker.compose.project=soloops-{project_id}"),
                ],
                Duration::from_secs(30),
                64 * 1024,
            )
            .await
            .unwrap();
        assert!(containers.stdout.trim().is_empty());
        assert!(!caddy_managed_dir.join(format!("{project_id}.caddy")).exists());
    }
}
