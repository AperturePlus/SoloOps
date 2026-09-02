import type {
  TaskSummary,
  RunDetail,
  RuntimeSnapshot,
  EventEnvelope,
  ToolCallSummary,
  RunStatus
} from "$lib/contracts";
import type { IpNotificationSettings } from "$lib/contracts";
import type { SshAccessReport } from "$lib/contracts";

export interface ScenarioHandle {
  runId: string;
  step: number;
  timer: ReturnType<typeof setTimeout> | undefined;
  finished: boolean;
}

export interface MockState {
  owner: { id: string; username: string } | null;
  tasks: Map<string, TaskSummary>;
  runs: Map<string, RunDetail>;
  runtimes: Map<string, RuntimeSnapshot>;
  events: EventEnvelope[];
  scenarios: Map<string, ScenarioHandle>;
  ipNotifications: IpNotificationSettings;
  sshAccess: SshAccessReport;
}

function randomId(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 10)}`;
}

function nowMs(): number {
  return Date.now();
}

function defaultBudget() {
  return {
    maxModelTurns: 8,
    maxToolCalls: 6,
    maxInputTokens: 4000,
    maxOutputTokens: 2000,
    maxDurationMs: 60000,
    maxToolDurationMs: 10000,
    maxToolOutputBytes: 10000,
    maxWorkspaceBytes: 100000
  };
}

function emptyUsage() {
  return {
    modelTurns: 0,
    toolCalls: 0,
    inputTokens: 0,
    outputTokens: 0,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    elapsedMs: 0
  };
}

function mockSshAccess(): SshAccessReport {
  return {
    scannedAt: Date.now(),
    user: "owner",
    homeDir: "/home/owner",
    platform: "linux",
    totalKeys: 4,
    invalidLines: 1,
    machines: [
      { name: "buildbox", keyCount: 1 },
      { name: "thinkpad", keyCount: 2 },
      { name: null, keyCount: 1 }
    ],
    files: [
      {
        path: "/home/owner/.ssh/authorized_keys",
        role: "user",
        exists: true,
        error: null,
        entries: [
          {
            line: 1,
            keyType: "ssh-ed25519",
            keyBits: 256,
            fingerprint: "SHA256:tR4mP0rtA1Xi9q2Ld8vN3wY5uI7oP1aSdF6gH8jK5cE",
            comment: "owner@thinkpad",
            machine: "thinkpad",
            options: [],
            fromPatterns: null,
            forcedCommand: null,
            valid: true,
            error: null
          },
          {
            line: 2,
            keyType: "ssh-rsa",
            keyBits: 3072,
            fingerprint: "SHA256:Q2b9ZkW4eV7nR1mYcU3xD8fG6hJ0lK5aSdF2gH8jK5cE",
            comment: "owner@thinkpad",
            machine: "thinkpad",
            options: [],
            fromPatterns: null,
            forcedCommand: null,
            valid: true,
            error: null
          },
          {
            line: 4,
            keyType: "ssh-ed25519",
            keyBits: 256,
            fingerprint: "SHA256:9Xk7Lm2Np8Qw4Er6Ty1Uz3Xc5Vb7Nh9Jk2Lm4Pq6St8Uv",
            comment: "ci@buildbox",
            machine: "buildbox",
            options: ['from="10.0.0.*"', 'command="/usr/local/bin/nightly-backup"', "no-pty"],
            fromPatterns: ["10.0.0.*"],
            forcedCommand: "/usr/local/bin/nightly-backup",
            valid: true,
            error: null
          },
          {
            line: 6,
            keyType: "ssh-ed25519",
            keyBits: 256,
            fingerprint: "SHA256:VbN8Jk2Lm4Pq6St8Uv0Wx2Yz4A6B8C0D2E4F6G8H0J2K4L",
            comment: "recovery key 2024",
            machine: null,
            options: [],
            fromPatterns: null,
            forcedCommand: null,
            valid: true,
            error: null
          },
          {
            line: 8,
            keyType: null,
            keyBits: null,
            fingerprint: null,
            comment: null,
            machine: null,
            options: [],
            fromPatterns: null,
            forcedCommand: null,
            valid: false,
            error: "line does not contain a recognizable public key type"
          }
        ]
      }
    ]
  };
}

export function createState(): MockState {
  return {
    owner: null,
    tasks: new Map(),
    runs: new Map(),
    runtimes: new Map(),
    events: [],
    scenarios: new Map(),
    ipNotifications: {
      enabled: false,
      smtpConfigured: true,
      currentIpv4: "203.0.113.10",
      lastCheckedAt: Date.now(),
      lastChangedAt: Date.now(),
      recipients: []
    },
    sshAccess: mockSshAccess()
  };
}

export function createTask(state: MockState, title: string, goal: string): TaskSummary {
  const runId = randomId("run");
  const taskId = randomId("task");
  const createdAt = nowMs();

  const run: RunDetail = {
    id: runId,
    taskId,
    status: "queued",
    statusReason: null,
    createdAt,
    startedAt: null,
    finishedAt: null
  };
  state.runs.set(runId, run);

  const runtime: RuntimeSnapshot = {
    runId,
    checkpoint: "preparing",
    plan: { summary: "", steps: [] },
    budget: defaultBudget(),
    usage: emptyUsage(),
    workspaceRevision: 0,
    toolCalls: [],
    evidence: [],
    report: null
  };
  state.runtimes.set(runId, runtime);

  const task: TaskSummary = {
    id: taskId,
    title,
    goal,
    status: "queued",
    latestRunId: runId,
    createdAt
  };
  state.tasks.set(taskId, task);

  appendEvent(state, {
    id: randomId("evt"),
    runId,
    type: "run.created",
    payload: { taskId, runId }
  });

  state.scenarios.set(runId, { runId, step: 0, timer: undefined, finished: false });
  return task;
}

export function getTask(state: MockState, id: string): TaskSummary | undefined {
  return state.tasks.get(id);
}

export function listTasks(state: MockState): TaskSummary[] {
  return [...state.tasks.values()].sort((a, b) => b.createdAt - a.createdAt);
}

export function getRun(state: MockState, id: string): RunDetail | undefined {
  return state.runs.get(id);
}

export function getRuntime(state: MockState, id: string): RuntimeSnapshot | undefined {
  return state.runtimes.get(id);
}

export function setRunStatus(
  state: MockState,
  runId: string,
  status: RunStatus,
  reason?: string
): RunDetail {
  const run = state.runs.get(runId);
  if (!run) throw new Error(`run not found: ${runId}`);
  run.status = status;
  if (reason !== undefined) run.statusReason = reason;
  if (status === "queued" || status === "planning" || status === "running" || status === "leased") {
    if (run.startedAt === null) run.startedAt = nowMs();
  }
  if (["succeeded", "failed", "cancelled"].includes(status)) {
    run.finishedAt = nowMs();
  }
  const task = [...state.tasks.values()].find((t) => t.latestRunId === runId);
  if (task) task.status = status;
  return run;
}

export function appendEvent(
  state: MockState,
  event: Omit<EventEnvelope, "sequence" | "createdAt" | "id"> & { id: string }
): EventEnvelope {
  const envelope: EventEnvelope = {
    sequence: state.events.length === 0 ? 1 : state.events[state.events.length - 1].sequence + 1,
    id: event.id,
    runId: event.runId,
    type: event.type,
    payload: event.payload,
    createdAt: nowMs()
  };
  state.events.push(envelope);
  return envelope;
}

export function listEvents(state: MockState, after: number, runId?: string): EventEnvelope[] {
  return state.events.filter(
    (e) => e.sequence > after && (runId === undefined || e.runId === runId)
  );
}

export function updateToolCall(
  state: MockState,
  runId: string,
  callId: string,
  patch: Partial<ToolCallSummary>
): ToolCallSummary {
  const runtime = state.runtimes.get(runId);
  if (!runtime) throw new Error(`runtime not found: ${runId}`);
  const idx = runtime.toolCalls.findIndex((c) => c.callId === callId);
  if (idx === -1) throw new Error(`tool call not found: ${callId}`);
  runtime.toolCalls[idx] = { ...runtime.toolCalls[idx], ...patch };
  return runtime.toolCalls[idx];
}
