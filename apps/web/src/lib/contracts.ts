// Browser contract mirror for the Rust API. Keep field names aligned with
// crates/soloops-domain and docs/api/openapi.yaml.
export type RunStatus =
  | "draft"
  | "queued"
  | "leased"
  | "planning"
  | "running"
  | "waiting_for_approval"
  | "paused"
  | "verifying"
  | "reporting"
  | "retry_scheduled"
  | "blocked"
  | "needs_recovery"
  | "succeeded"
  | "failed"
  | "cancelled";

export type Owner = {
  id: string;
  username: string;
};

export type IpNotificationRecipientStatus = {
  email: string;
  lastNotifiedIpv4: string | null;
  lastNotifiedAt: number | null;
  lastAttemptAt: number | null;
  lastError: string | null;
};

export type IpNotificationSettings = {
  enabled: boolean;
  smtpConfigured: boolean;
  currentIpv4: string | null;
  lastCheckedAt: number | null;
  lastChangedAt: number | null;
  recipients: IpNotificationRecipientStatus[];
};

export type TestIpNotificationResponse = {
  sentCount: number;
  failedRecipients: string[];
};

export type SmtpSettings = {
  configured: boolean;
  source: "database" | "environment" | null;
  host: string | null;
  port: number | null;
  security: "tls" | "starttls" | null;
  from: string | null;
  username: string | null;
  passwordConfigured: boolean;
};

export type UpdateSmtpSettingsRequest = {
  host: string;
  port: number;
  security: "tls" | "starttls";
  from: string;
  username?: string | null;
  /** Absent keeps the stored credential, an empty string clears it. */
  password?: string | null;
};

export type TestSmtpDeliveryResponse = {
  delivered: boolean;
  error: string | null;
};

export type ModelSettings = {
  configured: boolean;
  source: "database" | "environment" | null;
  baseUrl: string | null;
  modelName: string | null;
  apiKeyConfigured: boolean;
};

export type UpdateModelSettingsRequest = {
  baseUrl: string;
  modelName: string;
  /** Absent keeps the stored credential, an empty string clears it. */
  apiKey?: string | null;
};

export type TestModelSettingsResponse = {
  responded: boolean;
  error: string | null;
};

export type SshKeyFileRole = "user" | "administrators";

export type SshAuthorizedKeyEntry = {
  line: number;
  keyType: string | null;
  keyBits: number | null;
  fingerprint: string | null;
  comment: string | null;
  machine: string | null;
  options: string[];
  fromPatterns: string[] | null;
  forcedCommand: string | null;
  valid: boolean;
  error: string | null;
};

export type SshAuthorizedKeysFile = {
  path: string;
  role: SshKeyFileRole;
  exists: boolean;
  entries: SshAuthorizedKeyEntry[];
  error: string | null;
};

export type SshMachineSummary = {
  name: string | null;
  keyCount: number;
};

export type SshAccessReport = {
  scannedAt: number;
  user: string | null;
  homeDir: string | null;
  platform: string;
  totalKeys: number;
  invalidLines: number;
  machines: SshMachineSummary[];
  files: SshAuthorizedKeysFile[];
};

export type TaskSummary = {
  id: string;
  title: string;
  goal: string;
  status: RunStatus;
  latestRunId: string;
  createdAt: number;
};

export type RunDetail = {
  id: string;
  taskId: string;
  status: RunStatus;
  statusReason: string | null;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
};

export type EventType =
  | "run.created"
  | "run.status_changed"
  | "agent.plan_updated"
  | "agent.message"
  | "tool.call_started"
  | "tool.call_completed"
  | "tool.call_failed"
  | "run.reported";

export type EventEnvelope = {
  sequence: number;
  id: string;
  runId: string | null;
  type: EventType;
  payload: Record<string, unknown>;
  createdAt: number;
};

export type ApiError = {
  error: {
    code: string;
    message: string;
    details?: unknown;
  };
};

export type RuntimeCheckpoint =
  | "preparing"
  | "calling_model"
  | "executing_tools"
  | "validating_completion"
  | "reporting"
  | "waiting_for_approval"
  | "retry_scheduled"
  | "recovering";

export type PlanStepStatus = "pending" | "in_progress" | "completed" | "blocked";

export type AgentPlan = {
  summary: string;
  steps: { id: string; title: string; status: PlanStepStatus; required: boolean }[];
};

export type BudgetSnapshot = {
  maxModelTurns: number;
  maxToolCalls: number;
  maxInputTokens: number;
  maxOutputTokens: number;
  maxDurationMs: number;
  maxToolDurationMs: number;
  maxToolOutputBytes: number;
  maxWorkspaceBytes: number;
};

export type UsageSnapshot = {
  modelTurns: number;
  toolCalls: number;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  cacheWriteInputTokens: number;
  elapsedMs: number;
};

export type ToolRisk = "read_only" | "workspace_write" | "process" | "network" | "privileged";
export type PolicyDecision = "allow" | "deny" | "require_approval";
export type ToolCallStatus =
  "pending" | "waiting_for_approval" | "running" | "completed" | "failed" | "denied" | "unknown";

export type ToolCallSummary = {
  callId: string;
  name: string;
  argumentsSha256: string;
  approvalPreview: Record<string, unknown> | null;
  risk: ToolRisk;
  policy: PolicyDecision;
  status: ToolCallStatus;
  resultSummary: string | null;
  errorCategory: string | null;
  startedAt: number | null;
  completedAt: number | null;
};

export type EvidenceSummary = {
  id: string;
  toolCallId: string;
  kind: string;
  summary: string;
  artifactRef: string | null;
  contentSha256: string | null;
  workspaceRevision: number;
  createdAt: number;
};

export type FinalReport = {
  outcome: string;
  summary: string;
  completed: string[];
  incomplete: string[];
  risks: string[];
  evidenceIds: string[];
  rollback: string | null;
  usage: UsageSnapshot;
  markdown: string;
};

export type RuntimeSnapshot = {
  runId: string;
  checkpoint: RuntimeCheckpoint;
  plan: AgentPlan;
  budget: BudgetSnapshot;
  usage: UsageSnapshot;
  workspaceRevision: number;
  toolCalls: ToolCallSummary[];
  evidence: EvidenceSummary[];
  report: FinalReport | null;
};
