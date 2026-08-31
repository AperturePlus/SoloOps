//! Shared fake providers and host executors used by the runtime engine tests.

use async_trait::async_trait;
use serde_json::{Value, json};
use soloops_domain::UsageSnapshot;
use soloops_storage::Database;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    HostExecutor, HostManagedDeployOutput, HostProcessOutput, HostSandboxOutput, ModelProvider,
    ModelRequest, ModelResponse, ModelToolCall, ProviderError, ProviderErrorCategory, ToolContext,
    ToolError,
};

pub struct FakeProvider {
    pub responses: Mutex<VecDeque<ModelResponse>>,
}

#[async_trait]
impl ModelProvider for FakeProvider {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        Ok(self.responses.lock().unwrap().pop_front().unwrap())
    }
}

pub struct WorkflowProvider {
    pub turn: AtomicUsize,
}

pub struct ProcessWorkflowProvider {
    pub turn: AtomicUsize,
}

pub struct SandboxWorkflowProvider {
    pub turn: AtomicUsize,
}

pub struct ManagedRecoveryProvider;

#[derive(Default)]
pub struct FakeHostExecutor {
    pub calls: Mutex<Vec<Value>>,
}

#[derive(Default)]
pub struct SlowManagedExecutor {
    pub calls: AtomicUsize,
}

#[async_trait]
impl HostExecutor for FakeHostExecutor {
    async fn execute_process(
        &self,
        _context: &ToolContext,
        arguments: Value,
    ) -> Result<HostProcessOutput, ToolError> {
        self.calls.lock().unwrap().push(arguments);
        Ok(HostProcessOutput {
            exit_code: Some(0),
            stdout: "fake hostd output".into(),
            stderr: String::new(),
        })
    }

    async fn execute_sandbox(
        &self,
        _context: &ToolContext,
        arguments: Value,
    ) -> Result<HostSandboxOutput, ToolError> {
        self.calls.lock().unwrap().push(arguments);
        Ok(HostSandboxOutput {
            exit_code: Some(0),
            stdout: "fake sandbox output".into(),
            stderr: String::new(),
            image: "sandbox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        })
    }

    async fn managed_deploy_apply(
        &self,
        _context: &ToolContext,
        arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        self.calls.lock().unwrap().push(arguments);
        Ok(HostManagedDeployOutput {
            action: "apply".into(),
            project_id: "demo".into(),
            status: "active".into(),
            revision_id: Some("revision-1".into()),
            previous_revision_id: None,
            proposal_sha256: Some("a".repeat(64)),
            preview: Some(json!({"services": ["web"], "healthPath": "/health"})),
        })
    }
}

#[async_trait]
impl HostExecutor for SlowManagedExecutor {
    async fn execute_process(
        &self,
        _context: &ToolContext,
        _arguments: Value,
    ) -> Result<HostProcessOutput, ToolError> {
        unreachable!()
    }

    async fn execute_sandbox(
        &self,
        _context: &ToolContext,
        _arguments: Value,
    ) -> Result<HostSandboxOutput, ToolError> {
        unreachable!()
    }

    async fn managed_deploy_apply(
        &self,
        _context: &ToolContext,
        _arguments: Value,
    ) -> Result<HostManagedDeployOutput, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        unreachable!("the engine deadline should elapse first")
    }
}

pub struct ReplaceWorkflowProvider {
    pub turn: AtomicUsize,
}

pub struct CancellingProvider {
    pub database: Database,
    pub owner_id: String,
}

pub struct RetryingProvider {
    pub calls: Mutex<Vec<(String, String)>>,
    pub invocation: AtomicUsize,
}

#[async_trait]
impl ModelProvider for RetryingProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.calls
            .lock()
            .unwrap()
            .push((request.logical_request_key, request.attempt_key));
        if self.invocation.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(ProviderError {
                category: ProviderErrorCategory::Server,
                message: "retry once".into(),
                retry_after: Some(Duration::ZERO),
            });
        }
        Ok(ModelResponse {
            assistant_message: Some("no progress".into()),
            tool_calls: vec![],
            usage: UsageSnapshot::default(),
            stop_reason: Some("stop".into()),
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl ModelProvider for CancellingProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let run_id = request.logical_request_key.split(":turn:").next().unwrap();
        self.database.cancel_run(run_id, &self.owner_id).await.unwrap();
        Err(ProviderError {
            category: ProviderErrorCategory::Server,
            message: "retryable failure after cancellation".into(),
            retry_after: None,
        })
    }
}

#[async_trait]
impl ModelProvider for WorkflowProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        let tool_calls = match turn {
            0 => vec![
                ModelToolCall {
                    call_id: "plan-1".into(),
                    name: "plan.update".into(),
                    arguments: json!({"summary": "Create and verify a file", "steps": [{"id": "write", "title": "Write demo", "status": "in_progress", "required": true}]}),
                },
                ModelToolCall {
                    call_id: "create-1".into(),
                    name: "workspace.create".into(),
                    arguments: json!({"path": "demo.txt", "content": "hello"}),
                },
            ],
            1 => vec![ModelToolCall {
                call_id: "read-1".into(),
                name: "workspace.read".into(),
                arguments: json!({"path": "demo.txt"}),
            }],
            _ => {
                let evidence_id = request
                    .evidence
                    .iter()
                    .find(|item| item.kind == "file_snapshot")
                    .unwrap()
                    .id
                    .clone();
                vec![
                    ModelToolCall {
                        call_id: "plan-2".into(),
                        name: "plan.update".into(),
                        arguments: json!({"summary": "Created and verified a file", "steps": [{"id": "write", "title": "Write demo", "status": "completed", "required": true}]}),
                    },
                    ModelToolCall {
                        call_id: "finish-1".into(),
                        name: "run.finish".into(),
                        arguments: json!({"outcome": "succeeded", "summary": "Created demo.txt", "completed": ["Created demo.txt"], "incomplete": [], "risks": [], "evidenceIds": [evidence_id], "rollback": "Delete demo.txt"}),
                    },
                ]
            }
        };
        Ok(ModelResponse {
            assistant_message: None,
            tool_calls,
            usage: UsageSnapshot::default(),
            stop_reason: Some("tool_calls".into()),
            provider_request_id: Some(format!("request-{turn}")),
        })
    }
}

#[async_trait]
impl ModelProvider for ProcessWorkflowProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        let tool_calls = if turn == 0 {
            vec![
                ModelToolCall {
                    call_id: "host-plan".into(),
                    name: "plan.update".into(),
                    arguments: json!({"summary": "Run an approved host command", "steps": [{"id": "exec", "title": "Execute command", "status": "in_progress", "required": true}]}),
                },
                ModelToolCall {
                    call_id: "host-exec".into(),
                    name: "process.exec".into(),
                    arguments: json!({"program": "echo", "args": ["hello"]}),
                },
            ]
        } else {
            let evidence_id = request
                .evidence
                .iter()
                .find(|item| item.kind == "command_result")
                .unwrap()
                .id
                .clone();
            vec![
                ModelToolCall {
                    call_id: "host-plan-complete".into(),
                    name: "plan.update".into(),
                    arguments: json!({"summary": "Ran an approved host command", "steps": [{"id": "exec", "title": "Execute command", "status": "completed", "required": true}]}),
                },
                ModelToolCall {
                    call_id: "host-finish".into(),
                    name: "run.finish".into(),
                    arguments: json!({"outcome": "succeeded", "summary": "Executed the approved command", "completed": ["Executed command"], "incomplete": [], "risks": [], "evidenceIds": [evidence_id], "rollback": null}),
                },
            ]
        };
        Ok(ModelResponse {
            assistant_message: None,
            tool_calls,
            usage: UsageSnapshot::default(),
            stop_reason: Some("tool_calls".into()),
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl ModelProvider for SandboxWorkflowProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        let tool_calls = if turn == 0 {
            vec![
                ModelToolCall {
                    call_id: "sandbox-plan".into(),
                    name: "plan.update".into(),
                    arguments: json!({"summary": "Run a sandboxed command", "steps": [{"id": "exec", "title": "Execute in sandbox", "status": "in_progress", "required": true}]}),
                },
                ModelToolCall {
                    call_id: "sandbox-exec".into(),
                    name: "sandbox.exec".into(),
                    arguments: json!({"program": "cargo", "args": ["test"]}),
                },
            ]
        } else {
            let evidence_id = request
                .evidence
                .iter()
                .find(|item| item.kind == "command_result")
                .unwrap()
                .id
                .clone();
            vec![
                ModelToolCall {
                    call_id: "sandbox-plan-complete".into(),
                    name: "plan.update".into(),
                    arguments: json!({"summary": "Ran a sandboxed command", "steps": [{"id": "exec", "title": "Execute in sandbox", "status": "completed", "required": true}]}),
                },
                ModelToolCall {
                    call_id: "sandbox-finish".into(),
                    name: "run.finish".into(),
                    arguments: json!({"outcome": "succeeded", "summary": "Executed in the sandbox", "completed": ["Executed sandbox command"], "incomplete": [], "risks": [], "evidenceIds": [evidence_id], "rollback": null}),
                },
            ]
        };
        Ok(ModelResponse {
            assistant_message: None,
            tool_calls,
            usage: UsageSnapshot::default(),
            stop_reason: Some("tool_calls".into()),
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl ModelProvider for ManagedRecoveryProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let evidence_id = request
            .evidence
            .iter()
            .find(|item| item.kind == "managed_deployment")
            .expect("managed deployment evidence")
            .id
            .clone();
        Ok(ModelResponse {
            assistant_message: None,
            tool_calls: vec![
                ModelToolCall {
                    call_id: "managed-plan-complete".into(),
                    name: "plan.update".into(),
                    arguments: json!({"summary": "Recovered managed deployment", "steps": [{"id": "deploy", "title": "Deploy", "status": "completed", "required": true}]}),
                },
                ModelToolCall {
                    call_id: "managed-finish".into(),
                    name: "run.finish".into(),
                    arguments: json!({"outcome": "succeeded", "summary": "Recovered deployment result", "completed": ["Deployment active"], "incomplete": [], "risks": [], "evidenceIds": [evidence_id], "rollback": "Use managed.deploy.rollback with an explicit verified revision"}),
                },
            ],
            usage: UsageSnapshot::default(),
            stop_reason: Some("tool_calls".into()),
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl ModelProvider for ReplaceWorkflowProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        let tool_calls = match turn {
            0 => vec![
                ModelToolCall {
                    call_id: "plan-replace".into(),
                    name: "plan.update".into(),
                    arguments: json!({"summary": "Replace and verify a file", "steps": [{"id": "write", "title": "Replace demo", "status": "in_progress", "required": true}]}),
                },
                ModelToolCall {
                    call_id: "replace-1".into(),
                    name: "workspace.replace".into(),
                    arguments: json!({"path": "demo.txt", "oldContent": "hello", "newContent": "goodbye"}),
                },
            ],
            1 => vec![ModelToolCall {
                call_id: "read-replace".into(),
                name: "workspace.read".into(),
                arguments: json!({"path": "demo.txt"}),
            }],
            _ => {
                let evidence_id = request
                    .evidence
                    .iter()
                    .find(|item| item.kind == "file_snapshot")
                    .unwrap()
                    .id
                    .clone();
                vec![
                    ModelToolCall {
                        call_id: "plan-replace-complete".into(),
                        name: "plan.update".into(),
                        arguments: json!({"summary": "Replaced and verified a file", "steps": [{"id": "write", "title": "Replace demo", "status": "completed", "required": true}]}),
                    },
                    ModelToolCall {
                        call_id: "finish-replace".into(),
                        name: "run.finish".into(),
                        arguments: json!({"outcome": "succeeded", "summary": "Replaced demo.txt", "completed": ["Replaced demo.txt"], "incomplete": [], "risks": [], "evidenceIds": [evidence_id], "rollback": "Restore demo.txt"}),
                    },
                ]
            }
        };
        Ok(ModelResponse {
            assistant_message: None,
            tool_calls,
            usage: UsageSnapshot::default(),
            stop_reason: Some("tool_calls".into()),
            provider_request_id: Some(format!("replace-request-{turn}")),
        })
    }
}
