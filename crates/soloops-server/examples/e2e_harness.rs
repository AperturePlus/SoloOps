use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use async_trait::async_trait;
use axum::{
    Json,
    routing::{get, post},
};
use serde_json::json;
use soloops_application::{
    HostExecutor, HostManagedDeployOutput, HostProcessOutput, HostSandboxOutput, ModelProvider, ModelRequest,
    ModelResponse, ModelToolCall, ProviderError, RuntimeConfig, RuntimeEngine, SecretRef, ToolContext,
    ToolError,
};
use soloops_domain::{BudgetSnapshot, PlanStepStatus, UsageSnapshot};
use soloops_server::{AppConfig, build_router};
use soloops_storage::{Database, NewManagedDeploymentRevision};
use tempfile::TempDir;
use tokio::sync::{Mutex, oneshot, watch};

const E2E_PASSWORD: &str = "correct horse battery staple";

#[derive(Clone)]
struct HarnessControl {
    errors: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    worker_shutdown: watch::Sender<bool>,
    worker: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

struct ScriptedProvider;

#[derive(Clone)]
struct ScriptedHostExecutor {
    database: Database,
}

#[async_trait]
impl HostExecutor for ScriptedHostExecutor {
    async fn execute_process(
        &self,
        _context: &ToolContext,
        _arguments: serde_json::Value,
    ) -> Result<HostProcessOutput, ToolError> {
        Err(ToolError::PolicyDenied("not used by E2E".into()))
    }

    async fn execute_sandbox(
        &self,
        _context: &ToolContext,
        _arguments: serde_json::Value,
    ) -> Result<HostSandboxOutput, ToolError> {
        Err(ToolError::PolicyDenied("not used by E2E".into()))
    }

    async fn managed_deploy_plan(
        &self,
        context: &ToolContext,
        _arguments: serde_json::Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        let proposal_sha256 = "a".repeat(64);
        let compose_sha256 = "b".repeat(64);
        let caddy_sha256 = "c".repeat(64);
        let preview = json!({
            "projectId": "e2e-app",
            "services": ["web"],
            "images": [format!("example/web@sha256:{}", "d".repeat(64))],
            "ports": [18080],
            "volumes": [],
            "site": "e2e.example.test",
            "routes": ["127.0.0.1:18080"],
            "healthPath": "/health"
        });
        let source = json!({"healthPath": "/health"});
        let revision = self
            .database
            .create_managed_deployment_revision(NewManagedDeploymentRevision {
                project_id: "e2e-app",
                run_id: &context.run_id,
                plan_call_id: &context.call_id,
                proposal_sha256: &proposal_sha256,
                compose_sha256: &compose_sha256,
                caddy_sha256: &caddy_sha256,
                source: &source,
                preview: &preview,
                bundle_path: "/managed/e2e-app/revision",
            })
            .await
            .map_err(|error| ToolError::Execution(error.to_string()))?;
        Ok(HostManagedDeployOutput {
            action: "plan".into(),
            project_id: "e2e-app".into(),
            status: "proposed".into(),
            revision_id: Some(revision.id),
            previous_revision_id: None,
            proposal_sha256: Some(proposal_sha256),
            preview: Some(preview),
        })
    }

    async fn managed_deploy_apply(
        &self,
        _context: &ToolContext,
        arguments: serde_json::Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        Ok(HostManagedDeployOutput {
            action: "apply".into(),
            project_id: "e2e-app".into(),
            status: "active".into(),
            revision_id: arguments
                .get("proposalId")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            previous_revision_id: None,
            proposal_sha256: arguments
                .get("proposalSha256")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            preview: None,
        })
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let managed = request.goal.contains("managed deployment");
        let call = if managed {
            managed_deployment_call(&request)
        } else if request.plan.steps.is_empty() {
            ModelToolCall {
                call_id: "e2e-plan-start".into(),
                name: "plan.update".into(),
                arguments: json!({
                    "summary": "Create one verified workspace artifact",
                    "steps": [{
                        "id": "create-artifact",
                        "title": "Create the E2E artifact",
                        "status": "in_progress",
                        "required": true
                    }]
                }),
            }
        } else if request.evidence.is_empty() {
            ModelToolCall {
                call_id: "e2e-workspace-create".into(),
                name: "workspace.create".into(),
                arguments: json!({
                    "path": "e2e-result.txt",
                    "content": "SoloOps E2E completed\n",
                    "createParents": false
                }),
            }
        } else if !request.evidence.iter().any(|item| item.kind == "file_snapshot") {
            ModelToolCall {
                call_id: "e2e-workspace-read".into(),
                name: "workspace.read".into(),
                arguments: json!({
                    "path": "e2e-result.txt",
                    "startLine": 1,
                    "maxLines": 20
                }),
            }
        } else if request
            .plan
            .steps
            .iter()
            .any(|step| step.status != PlanStepStatus::Completed)
        {
            ModelToolCall {
                call_id: "e2e-plan-complete".into(),
                name: "plan.update".into(),
                arguments: json!({
                    "summary": "Created and verified one workspace artifact",
                    "steps": [{
                        "id": "create-artifact",
                        "title": "Create the E2E artifact",
                        "status": "completed",
                        "required": true
                    }]
                }),
            }
        } else {
            ModelToolCall {
                call_id: "e2e-finish".into(),
                name: "run.finish".into(),
                arguments: json!({
                    "outcome": "succeeded",
                    "summary": "The browser-driven E2E task completed successfully.",
                    "completed": ["Created e2e-result.txt"],
                    "incomplete": [],
                    "risks": [],
                    "evidenceIds": [request.evidence.iter().find(|item| item.kind == "file_snapshot").expect("snapshot evidence exists").id],
                    "rollback": "Delete e2e-result.txt"
                }),
            }
        };
        Ok(ModelResponse {
            assistant_message: Some("E2E scripted provider advanced the run.".into()),
            tool_calls: vec![call],
            usage: UsageSnapshot {
                input_tokens: 10,
                output_tokens: 10,
                ..UsageSnapshot::default()
            },
            stop_reason: Some("tool_calls".into()),
            provider_request_id: None,
        })
    }
}

fn managed_deployment_call(request: &ModelRequest) -> ModelToolCall {
    if request.plan.steps.is_empty() {
        return ModelToolCall {
            call_id: "e2e-deploy-plan-start".into(),
            name: "plan.update".into(),
            arguments: json!({
                "summary": "Deploy one approved managed application",
                "steps": [{
                    "id": "deploy",
                    "title": "Deploy and verify e2e-app",
                    "status": "in_progress",
                    "required": true
                }]
            }),
        };
    }
    if !request
        .evidence
        .iter()
        .any(|item| item.kind == "deployment_proposal")
    {
        return ModelToolCall {
            call_id: "e2e-managed-plan".into(),
            name: "managed.deploy.plan".into(),
            arguments: json!({
                "projectId": "e2e-app",
                "composePath": "compose.yaml",
                "caddyFragmentPath": "site.caddy",
                "healthPath": "/health"
            }),
        };
    }
    if !request
        .evidence
        .iter()
        .any(|item| item.kind == "managed_deployment")
    {
        let result = request
            .journal
            .iter()
            .rev()
            .find(|item| item.kind == "tool_result" && item.payload["callId"] == "e2e-managed-plan")
            .expect("managed deployment proposal result exists");
        return ModelToolCall {
            call_id: "e2e-managed-apply".into(),
            name: "managed.deploy.apply".into(),
            arguments: json!({
                "proposalId": result.payload["result"]["revisionId"],
                "proposalSha256": result.payload["result"]["proposalSha256"]
            }),
        };
    }
    if request
        .plan
        .steps
        .iter()
        .any(|step| step.status != PlanStepStatus::Completed)
    {
        return ModelToolCall {
            call_id: "e2e-deploy-plan-complete".into(),
            name: "plan.update".into(),
            arguments: json!({
                "summary": "Deployed and verified e2e-app",
                "steps": [{
                    "id": "deploy",
                    "title": "Deploy and verify e2e-app",
                    "status": "completed",
                    "required": true
                }]
            }),
        };
    }
    let evidence = request
        .evidence
        .iter()
        .find(|item| item.kind == "managed_deployment")
        .expect("managed deployment evidence exists");
    ModelToolCall {
        call_id: "e2e-deploy-finish".into(),
        name: "run.finish".into(),
        arguments: json!({
            "outcome": "succeeded",
            "summary": "The managed deployment completed successfully.",
            "completed": ["Applied the approved Compose and Caddy proposal"],
            "incomplete": [],
            "risks": [],
            "evidenceIds": [evidence.id],
            "rollback": "Use managed.deploy.rollback with the recorded previous revision"
        }),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let root = TempDir::new().context("failed to create E2E temp directory")?;
    let database_path = root.path().join("e2e.db");
    let database = Database::connect(&database_path).await?;
    database.migrate().await?;
    let salt = SaltString::encode_b64(b"soloops-e2e-salt").expect("static salt is valid");
    let password_hash = Argon2::default()
        .hash_password(E2E_PASSWORD.as_bytes(), &salt)
        .expect("E2E password hashes")
        .to_string();
    database.create_owner("owner", &password_hash).await?;

    let port = std::env::var("SOLOOPS_E2E_PORT")
        .unwrap_or_else(|_| "4173".into())
        .parse::<u16>()
        .context("SOLOOPS_E2E_PORT must be a TCP port")?;
    let web_dist = std::env::var("SOLOOPS_E2E_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("apps/web/build"));
    if !web_dist.join("index.html").is_file() {
        anyhow::bail!("build the WebUI before starting the E2E harness");
    }
    let origin = format!("http://127.0.0.1:{port}");
    let mut app_config = AppConfig::test(database_path);
    app_config.web_origin = origin.clone();
    app_config.web_dist = web_dist;

    let runtime_config = RuntimeConfig {
        model_base_url: "http://unused".into(),
        model_name: "e2e-scripted".into(),
        model_api_key_ref: SecretRef::Env("UNUSED".into()),
        prompt_cache_key_enabled: false,
        workspace_root: root.path().join("workspaces"),
        artifact_root: root.path().join("artifacts"),
        hostd_socket: Some(PathBuf::from("unused-e2e-hostd.sock")),
        sandbox_enabled: false,
        managed_deploy_enabled: true,
        budget: BudgetSnapshot::default(),
        lease_ms: 5_000,
        lease_renew_ms: 1_000,
    };
    let engine = Arc::new(RuntimeEngine::new_with_host_executor(
        database.clone(),
        Arc::new(ScriptedProvider),
        runtime_config,
        Some(Arc::new(ScriptedHostExecutor {
            database: database.clone(),
        })),
    )?);
    let errors = Arc::new(Mutex::new(Vec::new()));
    let worker_errors = errors.clone();
    let (worker_shutdown, mut worker_shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(async move {
        loop {
            if let Err(error) = engine.run_once("e2e-worker").await {
                let message = format!("E2E worker failed: {error}");
                eprintln!("{message}");
                worker_errors.lock().await.push(message);
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(200)) => {}
                changed = worker_shutdown_rx.changed() => {
                    if changed.is_err() || *worker_shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    });
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let control = HarnessControl {
        errors,
        shutdown: Arc::new(Mutex::new(Some(shutdown_tx))),
        worker_shutdown,
        worker: Arc::new(Mutex::new(Some(worker))),
    };
    let status_control = control.clone();
    let shutdown_control = control.clone();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("SoloOps E2E harness ready at {origin}");
    axum::serve(
        listener,
        build_router(database.clone(), app_config)
            .route(
                "/__e2e/status",
                get(move || harness_status(status_control.clone())),
            )
            .route(
                "/__e2e/shutdown",
                post(move || harness_shutdown(shutdown_control.clone())),
            )
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = shutdown_rx => {}
        }
    })
    .await?;
    stop_worker(&control).await;
    database.close().await;
    drop(root);
    Ok(())
}

async fn harness_status(control: HarnessControl) -> Json<serde_json::Value> {
    let errors = control.errors.lock().await.clone();
    Json(json!({"ok": errors.is_empty(), "errors": errors}))
}

async fn harness_shutdown(control: HarnessControl) -> Json<serde_json::Value> {
    stop_worker(&control).await;
    let errors = control.errors.lock().await.clone();
    if let Some(sender) = control.shutdown.lock().await.take() {
        let _ = sender.send(());
    }
    Json(json!({"ok": errors.is_empty(), "errors": errors, "shutdown": true}))
}

async fn stop_worker(control: &HarnessControl) {
    let _ = control.worker_shutdown.send(true);
    if let Some(worker) = control.worker.lock().await.take()
        && let Err(error) = worker.await
    {
        control
            .errors
            .lock()
            .await
            .push(format!("E2E worker task failed during shutdown: {error}"));
    }
}
