use std::{
    collections::{HashMap, VecDeque},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soloops_domain::ToolRisk;
use thiserror::Error;
use tokio::fs;
use uuid::Uuid;

use crate::HostExecutor;

#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub risk: ToolRisk,
    pub timeout: Duration,
    pub output_limit: usize,
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub run_id: String,
    pub call_id: String,
    pub workspace: PathBuf,
    pub artifacts: PathBuf,
    pub max_output_bytes: usize,
    pub max_workspace_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWriteRecovery {
    pub(crate) operation: String,
    pub(crate) path: String,
    pub(crate) previous_sha256: Option<String>,
    pub(crate) expected_sha256: String,
    pub(crate) bytes: usize,
}

impl WorkspaceWriteRecovery {
    pub(crate) fn result(&self) -> Value {
        match self.previous_sha256.as_deref() {
            Some(previous) => json!({
                "path": self.path,
                "previousSha256": previous,
                "sha256": self.expected_sha256,
                "bytes": self.bytes,
                "recovered": true,
            }),
            None => json!({
                "path": self.path,
                "sha256": self.expected_sha256,
                "bytes": self.bytes,
                "recovered": true,
            }),
        }
    }

    pub(crate) fn summary(&self) -> String {
        let verb = if self.operation == "create" {
            "Created"
        } else {
            "Replaced"
        };
        format!("{verb} {}", self.path)
    }

    pub(crate) fn evidence_summary(&self) -> String {
        format!("{} at SHA-256 {}", self.summary(), self.expected_sha256)
    }
}

#[derive(Debug, Clone)]
pub struct ToolEvidence {
    pub kind: String,
    pub summary: String,
    pub artifact_ref: Option<String>,
    pub content_sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub value: Value,
    pub summary: String,
    pub evidence: Option<ToolEvidence>,
    pub increments_workspace: bool,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("path is outside the run workspace: {0}")]
    PathViolation(String),
    #[error("tool policy denied execution: {0}")]
    PolicyDenied(String),
    #[error("tool timed out")]
    Timeout,
    #[error("tool I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("tool execution failed: {0}")]
    Execution(String),
    #[error("tool outcome requires operator recovery: {0}")]
    RecoveryRequired(String),
    #[error("tool output exceeded {limit} bytes: {preview}")]
    OutputLimit { limit: usize, preview: String },
}

impl ToolError {
    pub const fn category(&self) -> &'static str {
        match self {
            Self::InvalidArguments(_) => "invalid_arguments",
            Self::PathViolation(_) => "path_violation",
            Self::PolicyDenied(_) => "policy_denied",
            Self::Timeout => "timeout",
            Self::Io(_) => "io",
            Self::Execution(_) => "execution",
            Self::RecoveryRequired(_) => "recovery_required",
            Self::OutputLimit { .. } => "output_limit",
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn descriptor(&self) -> ToolDescriptor;
    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError>;
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn standard(
        host_executor: Option<Arc<dyn HostExecutor>>,
        sandbox_enabled: bool,
        managed_deploy_enabled: bool,
    ) -> Result<Self, ToolError> {
        let mut registry = Self::default();
        registry.register(Arc::new(WorkspaceList))?;
        registry.register(Arc::new(WorkspaceRead))?;
        registry.register(Arc::new(WorkspaceSearch))?;
        registry.register(Arc::new(WorkspaceCreate))?;
        registry.register(Arc::new(WorkspaceReplace))?;
        if let Some(executor) = host_executor {
            registry.register(Arc::new(ProcessExec {
                executor: executor.clone(),
            }))?;
            if sandbox_enabled {
                registry.register(Arc::new(SandboxExec {
                    executor: executor.clone(),
                }))?;
            }
            if managed_deploy_enabled {
                registry.register(Arc::new(ManagedDeployTool::plan(executor.clone())))?;
                registry.register(Arc::new(ManagedDeployTool::status(executor.clone())))?;
                registry.register(Arc::new(ManagedDeployTool::apply(executor.clone())))?;
                registry.register(Arc::new(ManagedDeployTool::rollback(executor)))?;
            }
        }
        registry.register(Arc::new(ControlTool::plan()))?;
        registry.register(Arc::new(ControlTool::finish()))?;
        Ok(registry)
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let descriptor = tool.descriptor();
        if descriptor.name.trim().is_empty() || !descriptor.input_schema.is_object() {
            return Err(ToolError::InvalidArguments(
                "tool descriptor is invalid".to_owned(),
            ));
        }
        if self.tools.insert(descriptor.name.clone(), tool).is_some() {
            return Err(ToolError::InvalidArguments(format!(
                "duplicate tool name: {}",
                descriptor.name
            )));
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        let mut descriptors = self
            .tools
            .values()
            .map(|tool| tool.descriptor())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn descriptor(name: &str, description: &str, input_schema: Value, risk: ToolRisk) -> ToolDescriptor {
    descriptor_with_timeout(name, description, input_schema, risk, Duration::from_secs(60))
}

fn descriptor_with_timeout(
    name: &str,
    description: &str,
    input_schema: Value,
    risk: ToolRisk,
    timeout: Duration,
) -> ToolDescriptor {
    ToolDescriptor {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema,
        risk,
        timeout,
        output_limit: 64 * 1024,
    }
}

struct WorkspaceList;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListArguments {
    #[serde(default = "dot")]
    path: String,
    #[serde(default)]
    recursive: bool,
    #[serde(default = "default_entries")]
    max_entries: usize,
}

fn dot() -> String {
    ".".to_owned()
}
const fn default_entries() -> usize {
    200
}

#[async_trait]
impl Tool for WorkspaceList {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor(
            "workspace.list",
            "List files and directories inside the current run workspace.",
            schema(
                json!({
                    "path": {"type": "string", "default": "."},
                    "recursive": {"type": "boolean", "default": false},
                    "maxEntries": {"type": "integer", "minimum": 1, "maximum": 500}
                }),
                &[],
            ),
            ToolRisk::ReadOnly,
        )
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let input: ListArguments = parse(arguments)?;
        let limit = input.max_entries.clamp(1, 500);
        let start = resolve_existing(&context.workspace, &input.path, true).await?;
        let mut queue = VecDeque::from([start]);
        let mut entries = Vec::new();
        while let Some(directory) = queue.pop_front() {
            let mut reader = fs::read_dir(&directory).await?;
            while let Some(entry) = reader.next_entry().await? {
                let metadata = entry.file_type().await?;
                if metadata.is_symlink() {
                    continue;
                }
                let path = entry.path();
                let relative = path
                    .strip_prefix(&context.workspace)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                entries.push(
                    json!({"path": relative, "kind": if metadata.is_dir() {"directory"} else {"file"}}),
                );
                if input.recursive && metadata.is_dir() {
                    queue.push_back(path);
                }
                if entries.len() >= limit {
                    break;
                }
            }
            if entries.len() >= limit || !input.recursive {
                break;
            }
        }
        let summary = format!("Listed {} workspace entries", entries.len());
        let value = json!({"entries": entries, "truncated": entries.len() == limit});
        Ok(ToolOutput {
            summary: summary.clone(),
            evidence: Some(ToolEvidence {
                kind: "directory_snapshot".to_owned(),
                summary,
                artifact_ref: None,
                content_sha256: Some(format!("{:x}", Sha256::digest(value.to_string().as_bytes()))),
            }),
            value,
            increments_workspace: false,
        })
    }
}

struct WorkspaceRead;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadArguments {
    path: String,
    #[serde(default = "one")]
    start_line: usize,
    #[serde(default = "default_lines")]
    max_lines: usize,
}

const fn one() -> usize {
    1
}
const fn default_lines() -> usize {
    200
}

#[async_trait]
impl Tool for WorkspaceRead {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor(
            "workspace.read",
            "Read a bounded line range from a UTF-8 file in the run workspace.",
            schema(
                json!({
                    "path": {"type": "string"},
                    "startLine": {"type": "integer", "minimum": 1},
                    "maxLines": {"type": "integer", "minimum": 1, "maximum": 500}
                }),
                &["path"],
            ),
            ToolRisk::ReadOnly,
        )
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let input: ReadArguments = parse(arguments)?;
        let path = resolve_existing(&context.workspace, &input.path, false).await?;
        let bytes = fs::read(&path).await?;
        let text =
            String::from_utf8(bytes).map_err(|_| ToolError::Execution("file is not UTF-8".to_owned()))?;
        let total_lines = text.lines().count();
        let start = input.start_line.max(1);
        let take = input.max_lines.clamp(1, 500);
        let selected: Vec<&str> = text.lines().skip(start.saturating_sub(1)).take(take).collect();
        let end_line = start + selected.len();
        let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
        let bounded = truncate_output(
            context,
            "workspace-read",
            &selected.join("\n"),
            TruncationKeep::Head,
        )
        .await?;
        let mut value = json!({
            "path": input.path,
            "startLine": start,
            "endLine": end_line,
            "totalLines": total_lines,
            "content": bounded.content,
            "sha256": digest,
            "truncated": bounded.truncated,
            "artifactRef": bounded.artifact_ref,
        });
        let mut note = None;
        if end_line < total_lines {
            note = Some(format!(
                "Showing lines {start}-{end_line} of {total_lines}; continue reading with startLine={}.",
                end_line + 1
            ));
        }
        if bounded.truncated {
            let artifact_note = format!(
                "Output exceeded the {} line / {} byte budget; the full range is stored as artifact {}.",
                MAX_OUTPUT_LINES,
                context.max_output_bytes,
                bounded.artifact_ref.as_deref().unwrap_or_default()
            );
            note = Some(match note {
                Some(existing) => format!("{existing} {artifact_note}"),
                None => artifact_note,
            });
        }
        if let Some(note) = note {
            value["note"] = json!(note);
        }
        Ok(ToolOutput {
            summary: format!("Read {} lines {}-{}", input.path, start, end_line),
            value,
            evidence: Some(ToolEvidence {
                kind: "file_snapshot".to_owned(),
                summary: format!("Observed {} at SHA-256 {}", input.path, digest),
                artifact_ref: bounded.artifact_ref,
                content_sha256: Some(digest),
            }),
            increments_workspace: false,
        })
    }
}

struct WorkspaceSearch;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchArguments {
    query: String,
    #[serde(default = "dot")]
    path: String,
    #[serde(default)]
    regex: bool,
    #[serde(default = "default_matches")]
    max_matches: usize,
}

const fn default_matches() -> usize {
    100
}

#[async_trait]
impl Tool for WorkspaceSearch {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor(
            "workspace.search",
            "Search UTF-8 workspace files using a literal string or Rust regular expression.",
            schema(
                json!({
                    "query": {"type": "string", "minLength": 1},
                    "path": {"type": "string", "default": "."},
                    "regex": {"type": "boolean", "default": false},
                    "maxMatches": {"type": "integer", "minimum": 1, "maximum": 200}
                }),
                &["query"],
            ),
            ToolRisk::ReadOnly,
        )
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let input: SearchArguments = parse(arguments)?;
        if input.query.is_empty() {
            return Err(ToolError::InvalidArguments("query is required".to_owned()));
        }
        let matcher = input
            .regex
            .then(|| Regex::new(&input.query))
            .transpose()
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        let start = resolve_existing(&context.workspace, &input.path, true).await?;
        let canonical_workspace = fs::canonicalize(&context.workspace).await?;
        let limit = input.max_matches.clamp(1, 200);
        let mut queue = VecDeque::from([start]);
        let mut matches = Vec::new();
        while let Some(path) = queue.pop_front() {
            let metadata = fs::symlink_metadata(&path).await?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let mut reader = fs::read_dir(path).await?;
                while let Some(entry) = reader.next_entry().await? {
                    queue.push_back(entry.path());
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if metadata.len() > context.max_output_bytes as u64 {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path).await else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                let found = matcher
                    .as_ref()
                    .map_or_else(|| line.contains(&input.query), |regex| regex.is_match(line));
                if found {
                    matches.push(json!({
                        "path": path.strip_prefix(&canonical_workspace).unwrap_or(&path).to_string_lossy().replace('\\', "/"),
                        "line": index + 1,
                        "preview": line.chars().take(500).collect::<String>()
                    }));
                    if matches.len() >= limit {
                        break;
                    }
                }
            }
            if matches.len() >= limit {
                break;
            }
        }
        let summary = format!("Found {} matches", matches.len());
        let value = json!({"matches": matches, "truncated": matches.len() == limit});
        Ok(ToolOutput {
            summary: summary.clone(),
            evidence: Some(ToolEvidence {
                kind: "search_result".to_owned(),
                summary,
                artifact_ref: None,
                content_sha256: Some(format!("{:x}", Sha256::digest(value.to_string().as_bytes()))),
            }),
            value,
            increments_workspace: false,
        })
    }
}

struct WorkspaceCreate;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateArguments {
    path: String,
    content: String,
    #[serde(default)]
    create_parents: bool,
}

#[async_trait]
impl Tool for WorkspaceCreate {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor(
            "workspace.create",
            "Atomically create a new UTF-8 file without overwriting an existing path.",
            schema(
                json!({
                    "path": {"type": "string"},
                    "content": {"type": "string"},
                    "createParents": {"type": "boolean", "default": false}
                }),
                &["path", "content"],
            ),
            ToolRisk::WorkspaceWrite,
        )
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let input: CreateArguments = parse(arguments)?;
        let normalized_path = normalized_workspace_write_path(&input.path)?;
        ensure_workspace_capacity(context, input.content.len() as u64).await?;
        let path = resolve_new(&context.workspace, &input.path).await?;
        let parent = path
            .parent()
            .ok_or_else(|| ToolError::PathViolation(input.path.clone()))?;
        if input.create_parents {
            fs::create_dir_all(parent).await?;
        }
        atomic_publish(path, input.content.clone(), context.call_id.clone(), true).await?;
        let digest = format!("{:x}", Sha256::digest(input.content.as_bytes()));
        Ok(ToolOutput {
            summary: format!("Created {normalized_path}"),
            value: json!({"path": normalized_path, "sha256": digest, "bytes": input.content.len()}),
            evidence: Some(ToolEvidence {
                kind: "file_mutation".to_owned(),
                summary: format!("Created {normalized_path} at SHA-256 {digest}"),
                artifact_ref: None,
                content_sha256: Some(digest),
            }),
            increments_workspace: true,
        })
    }
}

struct WorkspaceReplace;

#[async_trait]
impl Tool for WorkspaceReplace {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor(
            "workspace.replace",
            "Atomically replace a file using an exact old-content or SHA-256 precondition.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}, "oldContent": {"type": "string"},
                    "newContent": {"type": "string"}, "expectedSha256": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path"], "additionalProperties": false,
                "oneOf": [
                    {"required": ["oldContent", "newContent"]},
                    {"required": ["expectedSha256", "content"]}
                ]
            }),
            ToolRisk::WorkspaceWrite,
        )
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let path_text = required_string(&arguments, "path")?;
        let normalized_path = normalized_workspace_write_path(&path_text)?;
        let path = resolve_existing(&context.workspace, &path_text, false).await?;
        let current = fs::read_to_string(&path).await?;
        let current_hash = format!("{:x}", Sha256::digest(current.as_bytes()));
        let replacement = replacement_content(&arguments, &current, &current_hash)?;
        ensure_workspace_capacity(context, replacement.len().saturating_sub(current.len()) as u64).await?;
        atomic_publish(path, replacement.clone(), context.call_id.clone(), false).await?;
        let digest = format!("{:x}", Sha256::digest(replacement.as_bytes()));
        Ok(ToolOutput {
            summary: format!("Replaced {normalized_path}"),
            value: json!({"path": normalized_path, "previousSha256": current_hash, "sha256": digest, "bytes": replacement.len()}),
            evidence: Some(ToolEvidence {
                kind: "file_mutation".to_owned(),
                summary: format!("Replaced {normalized_path} at SHA-256 {digest}"),
                artifact_ref: None,
                content_sha256: Some(digest),
            }),
            increments_workspace: true,
        })
    }
}

pub(crate) async fn prepare_workspace_write(
    workspace: &Path,
    name: &str,
    arguments: &Value,
) -> Result<Option<WorkspaceWriteRecovery>, ToolError> {
    match name {
        "workspace.create" => {
            let input: CreateArguments = parse(arguments.clone())?;
            let normalized_path = normalized_workspace_write_path(&input.path)?;
            let path = resolve_new(workspace, &input.path).await?;
            if fs::try_exists(&path).await? {
                return Err(ToolError::Execution(format!("{} already exists", input.path)));
            }
            let parent = path
                .parent()
                .ok_or_else(|| ToolError::PathViolation(input.path.clone()))?;
            if !input.create_parents && !fs::try_exists(parent).await? {
                return Err(ToolError::Execution(format!(
                    "parent directory for {} does not exist",
                    input.path
                )));
            }
            let digest = format!("{:x}", Sha256::digest(input.content.as_bytes()));
            Ok(Some(WorkspaceWriteRecovery {
                operation: "create".to_owned(),
                path: normalized_path,
                previous_sha256: None,
                expected_sha256: digest,
                bytes: input.content.len(),
            }))
        }
        "workspace.replace" => {
            let path_text = required_string(arguments, "path")?;
            let normalized_path = normalized_workspace_write_path(&path_text)?;
            let path = resolve_existing(workspace, &path_text, false).await?;
            let current = fs::read_to_string(&path).await?;
            let previous_sha256 = format!("{:x}", Sha256::digest(current.as_bytes()));
            let replacement = replacement_content(arguments, &current, &previous_sha256)?;
            let expected_sha256 = format!("{:x}", Sha256::digest(replacement.as_bytes()));
            Ok(Some(WorkspaceWriteRecovery {
                operation: "replace".to_owned(),
                path: normalized_path,
                previous_sha256: Some(previous_sha256),
                expected_sha256,
                bytes: replacement.len(),
            }))
        }
        _ => Ok(None),
    }
}

pub(crate) fn normalized_workspace_write_path(relative: &str) -> Result<String, ToolError> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty() {
        return Err(ToolError::PathViolation(relative.to_owned()));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ToolError::PathViolation(relative.to_owned()));
            }
        }
    }
    if parts.is_empty() {
        return Err(ToolError::PathViolation(relative.to_owned()));
    }
    Ok(parts.join("/"))
}

fn replacement_content(arguments: &Value, current: &str, current_hash: &str) -> Result<String, ToolError> {
    match (
        arguments.get("oldContent").and_then(Value::as_str),
        arguments.get("newContent").and_then(Value::as_str),
        arguments.get("expectedSha256").and_then(Value::as_str),
        arguments.get("content").and_then(Value::as_str),
    ) {
        (Some(old), Some(new), None, None) => {
            let count = current.matches(old).count();
            if count != 1 {
                return Err(ToolError::Execution(format!(
                    "oldContent matched {count} times; expected exactly once"
                )));
            }
            Ok(current.replacen(old, new, 1))
        }
        (None, None, Some(expected), Some(content)) if expected == current_hash => Ok(content.to_owned()),
        (None, None, Some(_), Some(_)) => Err(ToolError::Execution("SHA-256 precondition failed".to_owned())),
        _ => Err(ToolError::InvalidArguments(
            "provide exactly one replacement mode".to_owned(),
        )),
    }
}

pub(crate) fn workspace_temp_prefix(call_id: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(call_id.as_bytes()));
    format!(".soloops-write-{}-", &digest[..16])
}

async fn atomic_publish(
    path: PathBuf,
    content: String,
    call_id: String,
    no_clobber: bool,
) -> Result<(), ToolError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;

        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("workspace target has no parent"))?;
        let prefix = workspace_temp_prefix(&call_id);
        let mut temporary = tempfile::Builder::new().prefix(&prefix).tempfile_in(parent)?;
        temporary.write_all(content.as_bytes())?;
        temporary.as_file().sync_all()?;
        if no_clobber {
            temporary.persist_noclobber(&path).map_err(|error| error.error)?;
        } else {
            temporary.persist(&path).map_err(|error| error.error)?;
        }
        Ok::<(), std::io::Error>(())
    })
    .await
    .map_err(|error| ToolError::Execution(format!("workspace write task failed: {error}")))??;
    Ok(())
}

struct ProcessExec {
    executor: Arc<dyn HostExecutor>,
}

struct SandboxExec {
    executor: Arc<dyn HostExecutor>,
}

#[derive(Clone, Copy)]
enum ManagedDeployKind {
    Plan,
    Status,
    Apply,
    Rollback,
}

struct ManagedDeployTool {
    executor: Arc<dyn HostExecutor>,
    kind: ManagedDeployKind,
}

impl ManagedDeployTool {
    fn plan(executor: Arc<dyn HostExecutor>) -> Self {
        Self {
            executor,
            kind: ManagedDeployKind::Plan,
        }
    }

    fn status(executor: Arc<dyn HostExecutor>) -> Self {
        Self {
            executor,
            kind: ManagedDeployKind::Status,
        }
    }

    fn apply(executor: Arc<dyn HostExecutor>) -> Self {
        Self {
            executor,
            kind: ManagedDeployKind::Apply,
        }
    }

    fn rollback(executor: Arc<dyn HostExecutor>) -> Self {
        Self {
            executor,
            kind: ManagedDeployKind::Rollback,
        }
    }
}

#[async_trait]
impl Tool for ManagedDeployTool {
    fn descriptor(&self) -> ToolDescriptor {
        match self.kind {
            ManagedDeployKind::Plan => descriptor(
                "managed.deploy.plan",
                "Validate Workspace Compose and Caddy files and create an immutable deployment proposal.",
                schema(
                    json!({
                        "projectId": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{0,62}$"},
                        "composePath": {"type": "string"},
                        "caddyFragmentPath": {"type": "string"},
                        "healthPath": {"type": "string", "default": "/"}
                    }),
                    &["projectId", "composePath", "caddyFragmentPath"],
                ),
                ToolRisk::ReadOnly,
            ),
            ManagedDeployKind::Status => descriptor(
                "managed.deploy.status",
                "Read the current durable managed deployment status for one project.",
                schema(json!({"projectId": {"type": "string"}}), &["projectId"]),
                ToolRisk::ReadOnly,
            ),
            ManagedDeployKind::Apply => descriptor_with_timeout(
                "managed.deploy.apply",
                "Apply an immutable deployment proposal and automatically roll back on failed validation.",
                schema(
                    json!({
                        "proposalId": {"type": "string"},
                        "proposalSha256": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
                    }),
                    &["proposalId", "proposalSha256"],
                ),
                ToolRisk::Privileged,
                Duration::from_secs(600),
            ),
            ManagedDeployKind::Rollback => descriptor_with_timeout(
                "managed.deploy.rollback",
                "Roll back from an expected current revision to an explicitly selected verified revision.",
                schema(
                    json!({
                        "projectId": {"type": "string"},
                        "expectedCurrentRevisionId": {"type": "string"},
                        "targetRevisionId": {"type": "string"}
                    }),
                    &["projectId", "expectedCurrentRevisionId", "targetRevisionId"],
                ),
                ToolRisk::Privileged,
                Duration::from_secs(600),
            ),
        }
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let output = match self.kind {
            ManagedDeployKind::Plan => self.executor.managed_deploy_plan(context, arguments).await?,
            ManagedDeployKind::Status => self.executor.managed_deploy_status(context, arguments).await?,
            ManagedDeployKind::Apply => self.executor.managed_deploy_apply(context, arguments).await?,
            ManagedDeployKind::Rollback => self.executor.managed_deploy_rollback(context, arguments).await?,
        };
        let evidence_kind = match self.kind {
            ManagedDeployKind::Plan => "deployment_proposal",
            ManagedDeployKind::Status => "deployment_status",
            ManagedDeployKind::Apply | ManagedDeployKind::Rollback => "managed_deployment",
        };
        Ok(managed_deploy_tool_output(output, evidence_kind))
    }
}

pub(crate) fn managed_deploy_tool_output(
    output: crate::HostManagedDeployOutput,
    evidence_kind: &str,
) -> ToolOutput {
    let value = json!({
        "action": output.action,
        "projectId": output.project_id,
        "status": output.status,
        "revisionId": output.revision_id,
        "previousRevisionId": output.previous_revision_id,
        "proposalSha256": output.proposal_sha256,
        "preview": output.preview,
    });
    let summary = format!(
        "Managed deployment {} for {} is {}",
        output.action, output.project_id, output.status
    );
    ToolOutput {
        value,
        summary: summary.clone(),
        evidence: Some(ToolEvidence {
            kind: evidence_kind.into(),
            summary,
            artifact_ref: None,
            content_sha256: output.proposal_sha256,
        }),
        increments_workspace: false,
    }
}

#[async_trait]
impl Tool for SandboxExec {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor(
            "sandbox.exec",
            "Run one allowlisted executable inside an ephemeral, offline Docker sandbox without a shell. \
             A non-zero exit code is a normal result: the exit code plus captured stdout and stderr are \
             returned so failures can be diagnosed.",
            schema(
                json!({
                    "program": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}, "maxItems": 128},
                    "cwd": {"type": "string", "default": "."}, "env": {"type": "object", "additionalProperties": {"type": "string"}},
                    "timeoutMs": {"type": "integer", "minimum": 1, "maximum": 60000}
                }),
                &["program"],
            ),
            ToolRisk::Process,
        )
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let program = arguments
            .get("program")
            .and_then(Value::as_str)
            .unwrap_or("sandbox process")
            .to_owned();
        let output = self.executor.execute_sandbox(context, arguments).await?;
        let full = format!("stdout:\n{}\nstderr:\n{}", output.stdout, output.stderr);
        let bounded = truncate_output(context, "sandbox-output", &full, TruncationKeep::Tail).await?;
        let exit = output.exit_code;
        let summary = format!(
            "{} in {} exited with {}",
            program,
            output.image,
            exit.map_or_else(|| "signal".to_owned(), |value| value.to_string())
        );
        Ok(command_tool_output(
            &summary,
            exit,
            Some(output.image),
            format!("{:x}", Sha256::digest(full.as_bytes())),
            true,
            bounded,
            context,
        ))
    }
}

#[async_trait]
impl Tool for ProcessExec {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor(
            "process.exec",
            "Run one explicitly allowlisted executable without a shell. A non-zero exit \
             code is a normal result: the exit code plus captured stdout and stderr are \
             returned so failures can be diagnosed.",
            schema(
                json!({
                    "program": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}, "maxItems": 128},
                    "cwd": {"type": "string", "default": "."}, "env": {"type": "object", "additionalProperties": {"type": "string"}},
                    "timeoutMs": {"type": "integer", "minimum": 1, "maximum": 60000}
                }),
                &["program"],
            ),
            ToolRisk::Process,
        )
    }

    async fn execute(&self, context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        let program = arguments
            .get("program")
            .and_then(Value::as_str)
            .unwrap_or("process")
            .to_owned();
        let output = self.executor.execute_process(context, arguments).await?;
        let full = format!("stdout:\n{}\nstderr:\n{}", output.stdout, output.stderr);
        let bounded = truncate_output(context, "process-output", &full, TruncationKeep::Tail).await?;
        let exit = output.exit_code.map(i64::from);
        let summary = format!(
            "{} exited with {}",
            program,
            exit.map_or_else(|| "signal".to_owned(), |value| value.to_string())
        );
        Ok(command_tool_output(
            &summary,
            exit,
            None,
            format!("{:x}", Sha256::digest(full.as_bytes())),
            false,
            bounded,
            context,
        ))
    }
}

/// Shared projection for exec-style tools: a non-zero exit code is a business result
/// handed back to the model, not a tool failure. Infrastructure failures (spawn,
/// socket, timeout) still travel through `ToolError`.
fn command_tool_output(
    summary: &str,
    exit_code: Option<i64>,
    image: Option<String>,
    content_sha256: String,
    increments_workspace: bool,
    bounded: BoundedOutput,
    context: &ToolContext,
) -> ToolOutput {
    let mut value = json!({
        "exitCode": exit_code,
        "output": bounded.content,
        "truncated": bounded.truncated,
        "artifactRef": bounded.artifact_ref,
        "totalLines": bounded.total_lines,
        "outputLines": bounded.output_lines,
    });
    if let Some(image) = image {
        value["image"] = json!(image);
    }
    if bounded.truncated {
        value["note"] = json!(format!(
            "Output exceeded the {} line / {} byte budget; the tail is shown and the full output is stored as artifact {}.",
            MAX_OUTPUT_LINES,
            context.max_output_bytes,
            bounded.artifact_ref.as_deref().unwrap_or_default()
        ));
    }
    ToolOutput {
        value,
        summary: summary.to_owned(),
        evidence: Some(ToolEvidence {
            kind: "command_result".to_owned(),
            summary: summary.to_owned(),
            artifact_ref: bounded.artifact_ref,
            content_sha256: Some(content_sha256),
        }),
        increments_workspace,
    }
}

struct ControlTool {
    descriptor: ToolDescriptor,
}

impl ControlTool {
    fn plan() -> Self {
        Self {
            descriptor: descriptor(
                "plan.update",
                "Replace the persisted execution plan.",
                schema(
                    json!({
                        "summary": {"type": "string"},
                        "steps": {"type": "array", "items": {"type": "object", "properties": {
                            "id": {"type": "string"}, "title": {"type": "string"},
                            "status": {"type": "string", "enum": ["pending", "in_progress", "completed", "blocked"]},
                            "required": {"type": "boolean", "default": true}
                        }, "required": ["id", "title", "status"], "additionalProperties": false}}
                    }),
                    &["summary", "steps"],
                ),
                ToolRisk::ReadOnly,
            ),
        }
    }

    fn finish() -> Self {
        Self {
            descriptor: descriptor(
                "run.finish",
                "Request verified completion and cite evidence from this run.",
                schema(
                    json!({
                    "outcome": {"type": "string", "enum": ["succeeded"]}, "summary": {"type": "string"},
                            "completed": {"type": "array", "items": {"type": "string"}},
                            "incomplete": {"type": "array", "items": {"type": "string"}},
                            "risks": {"type": "array", "items": {"type": "string"}},
                            "evidenceIds": {"type": "array", "items": {"type": "string"}},
                            "rollback": {"type": ["string", "null"]}
                        }),
                    &[
                        "outcome",
                        "summary",
                        "completed",
                        "incomplete",
                        "risks",
                        "evidenceIds",
                    ],
                ),
                ToolRisk::ReadOnly,
            ),
        }
    }
}

#[async_trait]
impl Tool for ControlTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }
    async fn execute(&self, _context: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            value: arguments,
            summary: "Runtime control item accepted".to_owned(),
            evidence: None,
            increments_workspace: false,
        })
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, ToolError> {
    serde_json::from_value(arguments).map_err(|error| ToolError::InvalidArguments(error.to_string()))
}

fn required_string(arguments: &Value, key: &str) -> Result<String, ToolError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ToolError::InvalidArguments(format!("{key} is required")))
}

async fn resolve_existing(root: &Path, relative: &str, directory: bool) -> Result<PathBuf, ToolError> {
    let candidate = checked_join(root, relative).await?;
    let canonical_root = fs::canonicalize(root).await?;
    let canonical = fs::canonicalize(&candidate).await?;
    if !canonical.starts_with(&canonical_root) {
        return Err(ToolError::PathViolation(relative.to_owned()));
    }
    let metadata = fs::symlink_metadata(&candidate).await?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(ToolError::PathViolation(relative.to_owned()));
    }
    Ok(canonical)
}

async fn resolve_new(root: &Path, relative: &str) -> Result<PathBuf, ToolError> {
    checked_join(root, relative).await
}

async fn checked_join(root: &Path, relative: &str) -> Result<PathBuf, ToolError> {
    let relative_path = Path::new(relative);
    if relative_path.as_os_str().is_empty()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ToolError::PathViolation(relative.to_owned()));
    }
    let mut cursor = root.to_path_buf();
    for component in relative_path.components() {
        if matches!(component, Component::CurDir) {
            continue;
        }
        cursor.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&cursor).await
            && metadata.file_type().is_symlink()
        {
            return Err(ToolError::PathViolation(relative.to_owned()));
        }
    }
    Ok(cursor)
}

async fn workspace_size(root: &Path) -> Result<u64, ToolError> {
    let mut size = 0u64;
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(path) = queue.pop_front() {
        let metadata = fs::symlink_metadata(&path).await?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file() {
            size = size.saturating_add(metadata.len());
            continue;
        }
        if metadata.is_dir() {
            let mut reader = fs::read_dir(path).await?;
            while let Some(entry) = reader.next_entry().await? {
                queue.push_back(entry.path());
            }
        }
    }
    Ok(size)
}

async fn ensure_workspace_capacity(context: &ToolContext, additional: u64) -> Result<(), ToolError> {
    let size = workspace_size(&context.workspace).await?;
    if size.saturating_add(additional) > context.max_workspace_bytes {
        return Err(ToolError::PolicyDenied(
            "workspace size budget would be exceeded".to_owned(),
        ));
    }
    Ok(())
}

/// Line budget applied to every tool output before it reaches the model context.
pub(crate) const MAX_OUTPUT_LINES: usize = 2000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TruncationKeep {
    /// Keep the beginning of the output (paged reads resume forward).
    Head,
    /// Keep the end of the output (command failures usually surface at the tail).
    Tail,
}

pub(crate) struct BoundedOutput {
    pub content: String,
    pub artifact_ref: Option<String>,
    pub truncated: bool,
    pub total_lines: usize,
    pub output_lines: usize,
}

/// Bound `text` for the model context with a dual line/byte budget, never cutting a
/// line in half. When either budget trips, the full text is persisted as an artifact
/// for the Owner and the returned content advertises the artifact reference.
pub(crate) async fn truncate_output(
    context: &ToolContext,
    prefix: &str,
    text: &str,
    keep: TruncationKeep,
) -> Result<BoundedOutput, ToolError> {
    let total_lines = text.lines().count();
    if text.len() <= context.max_output_bytes && total_lines <= MAX_OUTPUT_LINES {
        return Ok(BoundedOutput {
            content: text.to_owned(),
            artifact_ref: None,
            truncated: false,
            total_lines,
            output_lines: total_lines,
        });
    }
    fs::create_dir_all(&context.artifacts).await?;
    let artifact_ref = format!("{prefix}-{}.txt", Uuid::new_v4());
    fs::write(context.artifacts.join(&artifact_ref), text.as_bytes()).await?;

    let mut lines: VecDeque<&str> = text.lines().collect();
    while lines.len() > MAX_OUTPUT_LINES {
        match keep {
            TruncationKeep::Head => {
                lines.pop_back();
            }
            TruncationKeep::Tail => {
                lines.pop_front();
            }
        }
    }
    let mut selected: Vec<&str> = lines.into();
    let mut size = selected
        .iter()
        .map(|line| line.len().saturating_add(1))
        .sum::<usize>();
    while selected.len() > 1 && size > context.max_output_bytes {
        let dropped = match keep {
            TruncationKeep::Head => selected.pop(),
            TruncationKeep::Tail => Some(selected.remove(0)),
        };
        match dropped {
            Some(line) => size = size.saturating_sub(line.len().saturating_add(1)),
            None => break,
        }
    }
    let mut content = selected.join("\n");
    if content.len() > context.max_output_bytes {
        // A single line can still exceed the byte budget; cut it at a character boundary.
        content = char_bounded_slice(&content, context.max_output_bytes, keep);
    }
    let output_lines = content.lines().count();
    Ok(BoundedOutput {
        content,
        artifact_ref: Some(artifact_ref),
        truncated: true,
        total_lines,
        output_lines,
    })
}

fn char_bounded_slice(text: &str, limit: usize, keep: TruncationKeep) -> String {
    let limit = limit.min(text.len());
    match keep {
        TruncationKeep::Head => {
            let mut boundary = limit;
            while boundary > 0 && !text.is_char_boundary(boundary) {
                boundary -= 1;
            }
            text[..boundary].to_owned()
        }
        TruncationKeep::Tail => {
            let mut start = text.len() - limit;
            while start < text.len() && !text.is_char_boundary(start) {
                start += 1;
            }
            text[start..].to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HostProcessOutput, HostSandboxOutput};

    struct NoopHostExecutor;

    struct NonzeroSandboxExecutor;

    struct FailingProcessExecutor {
        stdout_bytes: usize,
    }

    #[async_trait]
    impl HostExecutor for NoopHostExecutor {
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
    }

    #[async_trait]
    impl HostExecutor for NonzeroSandboxExecutor {
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
            Ok(HostSandboxOutput {
                exit_code: Some(7),
                stdout: "captured stdout".into(),
                stderr: "captured stderr".into(),
                image: "sandbox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            })
        }
    }

    #[async_trait]
    impl HostExecutor for FailingProcessExecutor {
        async fn execute_process(
            &self,
            _context: &ToolContext,
            _arguments: Value,
        ) -> Result<HostProcessOutput, ToolError> {
            Ok(HostProcessOutput {
                exit_code: Some(1),
                stdout: "compile started\n".repeat(self.stdout_bytes),
                stderr: "error[E0308]: mismatched types while compiling demo\n".repeat(20),
            })
        }

        async fn execute_sandbox(
            &self,
            _context: &ToolContext,
            _arguments: Value,
        ) -> Result<HostSandboxOutput, ToolError> {
            unreachable!()
        }
    }

    #[test]
    fn privileged_tool_registration_is_explicit() {
        let host = Arc::new(NoopHostExecutor) as Arc<dyn HostExecutor>;
        let host_only = ToolRegistry::standard(Some(host.clone()), false, false).unwrap();
        assert!(host_only.get("process.exec").is_some());
        assert!(host_only.get("sandbox.exec").is_none());
        assert!(host_only.get("managed.deploy.apply").is_none());

        let sandbox = ToolRegistry::standard(Some(host), true, false).unwrap();
        assert!(sandbox.get("process.exec").is_some());
        assert!(sandbox.get("sandbox.exec").is_some());

        let host = Arc::new(NoopHostExecutor) as Arc<dyn HostExecutor>;
        let managed = ToolRegistry::standard(Some(host), false, true).unwrap();
        assert!(managed.get("managed.deploy.plan").is_some());
        assert!(managed.get("managed.deploy.status").is_some());
        assert_eq!(
            managed.get("managed.deploy.apply").unwrap().descriptor().risk,
            ToolRisk::Privileged
        );
        assert_eq!(
            managed.get("managed.deploy.apply").unwrap().descriptor().timeout,
            Duration::from_secs(600)
        );
        assert_eq!(
            managed
                .get("managed.deploy.rollback")
                .unwrap()
                .descriptor()
                .timeout,
            Duration::from_secs(600)
        );
        assert_eq!(
            managed.get("managed.deploy.plan").unwrap().descriptor().timeout,
            Duration::from_secs(60)
        );
        assert_eq!(
            managed.get("process.exec").unwrap().descriptor().timeout,
            Duration::from_secs(60)
        );
    }

    #[tokio::test]
    async fn sandbox_exec_returns_nonzero_exit_output_as_a_model_visible_result() {
        let temp = tempfile::tempdir().unwrap();
        let context = ToolContext {
            run_id: "run".into(),
            call_id: "call-sandbox".into(),
            workspace: temp.path().join("workspace"),
            artifacts: temp.path().join("artifacts"),
            max_output_bytes: 1024,
            max_workspace_bytes: 1024 * 1024,
        };
        let tool = SandboxExec {
            executor: Arc::new(NonzeroSandboxExecutor),
        };

        let output = tool
            .execute(&context, json!({"program": "cargo", "args": ["test"]}))
            .await
            .unwrap();
        assert_eq!(output.value["exitCode"], json!(7));
        let text = output.value["output"].as_str().unwrap();
        assert!(text.contains("captured stdout"));
        assert!(text.contains("captured stderr"));
        assert_eq!(output.value["truncated"], json!(false));
        assert_eq!(
            output.summary,
            "cargo in sandbox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa exited with 7"
        );
        assert!(output.increments_workspace);
    }

    #[tokio::test]
    async fn process_exec_returns_nonzero_exit_with_stdout_and_stderr() {
        let temp = tempfile::tempdir().unwrap();
        let context = ToolContext {
            run_id: "run".into(),
            call_id: "call-process".into(),
            workspace: temp.path().join("workspace"),
            artifacts: temp.path().join("artifacts"),
            max_output_bytes: 64 * 1024,
            max_workspace_bytes: 1024 * 1024,
        };
        let tool = ProcessExec {
            executor: Arc::new(FailingProcessExecutor { stdout_bytes: 4 }),
        };

        let output = tool
            .execute(&context, json!({"program": "cargo", "args": ["build"]}))
            .await
            .unwrap();
        assert_eq!(output.value["exitCode"], json!(1));
        assert!(output.value["output"].as_str().unwrap().contains("error[E0308]"));
        assert_eq!(output.summary, "cargo exited with 1");
        assert!(!output.increments_workspace);
        let evidence = output.evidence.unwrap();
        assert_eq!(evidence.kind, "command_result");
        assert!(
            evidence
                .content_sha256
                .as_deref()
                .is_some_and(|digest| digest.len() == 64)
        );
    }

    #[tokio::test]
    async fn process_exec_truncates_huge_output_to_the_tail_and_persists_an_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let context = ToolContext {
            run_id: "run".into(),
            call_id: "call-process".into(),
            workspace: temp.path().join("workspace"),
            artifacts: temp.path().join("artifacts"),
            max_output_bytes: 1024,
            max_workspace_bytes: 1024 * 1024,
        };
        let tool = ProcessExec {
            executor: Arc::new(FailingProcessExecutor { stdout_bytes: 5000 }),
        };

        let output = tool
            .execute(&context, json!({"program": "cargo", "args": ["build"]}))
            .await
            .unwrap();
        assert_eq!(output.value["truncated"], json!(true));
        let text = output.value["output"].as_str().unwrap();
        // The tail is preserved: compiler errors surface at the end of the output.
        assert!(text.contains("error[E0308]"));
        assert!(!text.contains("compile started"));
        assert!(
            text.len() <= 2048,
            "content should respect the byte budget, got {}",
            text.len()
        );
        let artifact_ref = output.value["artifactRef"].as_str().unwrap().to_owned();
        assert!(artifact_ref.starts_with("process-output-"));
        let persisted = fs::read_to_string(context.artifacts.join(&artifact_ref))
            .await
            .unwrap();
        assert!(persisted.contains("compile started"));
        assert!(persisted.len() > context.max_output_bytes);
        let evidence = output.evidence.unwrap();
        assert_eq!(evidence.artifact_ref.as_deref(), Some(artifact_ref.as_str()));
        let note = output.value["note"].as_str().unwrap();
        assert!(note.contains(artifact_ref.as_str()));
    }

    #[tokio::test]
    async fn truncate_output_never_cuts_a_line_in_half() {
        let temp = tempfile::tempdir().unwrap();
        let context = ToolContext {
            run_id: "run".into(),
            call_id: "call-truncate".into(),
            workspace: temp.path().join("workspace"),
            artifacts: temp.path().join("artifacts"),
            max_output_bytes: 300,
            max_workspace_bytes: 1024 * 1024,
        };
        let text = (0..2000)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");

        let tail = truncate_output(&context, "tail", &text, TruncationKeep::Tail)
            .await
            .unwrap();
        assert!(tail.truncated);
        assert_eq!(tail.total_lines, 2000);
        assert!(
            tail.content.ends_with("line-1999"),
            "tail should keep the last lines"
        );
        assert!(
            tail.content.starts_with("line-19"),
            "only the final lines survive the byte budget"
        );
        assert_eq!(tail.output_lines, tail.content.lines().count());
        assert!(tail.content.len() <= context.max_output_bytes + 10);
        let head = truncate_output(&context, "head", &text, TruncationKeep::Head)
            .await
            .unwrap();
        assert!(head.content.starts_with("line-0"));
        assert!(head.content.lines().all(|line| line.starts_with("line-")));

        let unbounded = truncate_output(
            &ToolContext {
                run_id: context.run_id.clone(),
                call_id: context.call_id.clone(),
                workspace: context.workspace.clone(),
                artifacts: context.artifacts.clone(),
                max_output_bytes: 1_000_000,
                max_workspace_bytes: context.max_workspace_bytes,
            },
            "none",
            "small\noutput",
            TruncationKeep::Tail,
        )
        .await
        .unwrap();
        assert!(!unbounded.truncated);
        assert_eq!(unbounded.content, "small\noutput");
        assert!(unbounded.artifact_ref.is_none());
    }

    #[tokio::test]
    async fn truncate_output_handles_multibyte_lines_at_the_byte_budget() {
        let temp = tempfile::tempdir().unwrap();
        let context = ToolContext {
            run_id: "run".into(),
            call_id: "call-truncate".into(),
            workspace: temp.path().join("workspace"),
            artifacts: temp.path().join("artifacts"),
            max_output_bytes: 64,
            max_workspace_bytes: 1024 * 1024,
        };
        // One 3-byte-per-character line that alone exceeds the budget.
        let text = "中文输出".repeat(50);

        let bounded = truncate_output(&context, "mbcs", &text, TruncationKeep::Tail)
            .await
            .unwrap();
        assert!(bounded.truncated);
        assert!(bounded.content.len() <= 64);
        assert_eq!(bounded.output_lines, 1);
        let persisted = fs::read_to_string(context.artifacts.join(bounded.artifact_ref.as_deref().unwrap()))
            .await
            .unwrap();
        assert_eq!(persisted, text);
    }

    #[tokio::test]
    async fn workspace_tools_reject_escape_and_replace_with_precondition() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let artifacts = temp.path().join("artifacts");
        fs::create_dir_all(&workspace).await.unwrap();
        let context = ToolContext {
            run_id: "run".into(),
            call_id: "call-workspace".into(),
            workspace: workspace.clone(),
            artifacts,
            max_output_bytes: 1024,
            max_workspace_bytes: 1024 * 1024,
        };
        let create = WorkspaceCreate;
        assert!(
            create
                .execute(&context, json!({"path": "../escape", "content": "x"}))
                .await
                .is_err()
        );
        create
            .execute(&context, json!({"path": "demo.txt", "content": "hello"}))
            .await
            .unwrap();
        assert!(
            create
                .execute(&context, json!({"path": "demo.txt", "content": "clobbered"}))
                .await
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).await.unwrap(),
            "hello"
        );
        let replace = WorkspaceReplace;
        replace
            .execute(
                &context,
                json!({"path": "demo.txt", "oldContent": "hello", "newContent": "world"}),
            )
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).await.unwrap(),
            "world"
        );
        let current_hash = format!("{:x}", Sha256::digest(b"world"));
        replace
            .execute(
                &context,
                json!({"path": "demo.txt", "expectedSha256": current_hash, "content": "again"}),
            )
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).await.unwrap(),
            "again"
        );

        let mut entries = fs::read_dir(&workspace).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            assert!(!entry.file_name().to_string_lossy().starts_with(".soloops-write-"));
        }
    }

    #[tokio::test]
    async fn workspace_search_skips_files_larger_than_the_output_budget() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let artifacts = temp.path().join("artifacts");
        fs::create_dir_all(&workspace).await.unwrap();
        fs::write(workspace.join("large.txt"), "needle".repeat(100))
            .await
            .unwrap();
        fs::write(workspace.join("small.txt"), "needle\n").await.unwrap();
        let context = ToolContext {
            run_id: "run".into(),
            call_id: "call-search".into(),
            workspace,
            artifacts,
            max_output_bytes: 64,
            max_workspace_bytes: 1024 * 1024,
        };

        let output = WorkspaceSearch
            .execute(&context, json!({"path": ".", "query": "needle"}))
            .await
            .unwrap();
        let matches = output.value["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["path"], "small.txt");
    }
}
