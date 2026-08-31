# WebUI Mock 测试替身 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `apps/web` 内置一个 Vite 插件,用 `VITE_MOCK=1` 开关启动一个无需真实后端的 mock,自动驱动单条快路 Run 生命周期,供 WebUI 人工功能测试。

**Architecture:** 一个 Vite 插件挂载在 dev server 上,在 `configureServer` 中拦截 `/api/*` REST 与 `/api/events` 的 WebSocket 升级。内存态由 `state.ts` 维护,`scenario.ts` 用定时器序列推进 Run 状态并经 `clients.ts` 向所有 WS 连接广播事件。REST 路由由 `router.ts` 分发。所有类型复用 `src/lib/contracts.ts`。

**Tech Stack:** TypeScript、Vite plugin API(`configureServer`)、`ws`(WebSocket server)、Bun test runner(`bun test`)。

## Global Constraints

- Bun 1.3.x;WebUI 为 SvelteKit + `adapter-static`,生产构建不需要 Bun/Node 运行时。
- mock 仅 dev 生效:插件只在 `command === "serve"` 时加入 `plugins`,且 `ws` 仅 dev server 进程使用,绝不进入浏览器 bundle。
- 类型严格对齐 `apps/web/src/lib/contracts.ts` 与 `docs/api/openapi.yaml` 的字段名与枚举值,不得改名。
- 现有 `apps/web/vite.config.ts` 的 `:3001` 代理配置保留不动;mock 开启时短路 `/api`,关闭时不影响代理。
- `ws` 与 `@types/ws` 加入 `apps/web/package.json` 的 `devDependencies`。
- 不实现多剧本、预算超限、needs_recovery、corrupt stream、动态模型循环。
- Run 状态枚举完整 15 值见 contracts.ts;mock 只用到其中子集,但 ToolCallStatus 等枚举值必须与契约字符串完全一致。

---

## File Structure

新增文件:

- `apps/web/src/mock/state.ts` — 内存状态:Owner/Task/Run/RuntimeSnapshot 的创建、查询、转换、事件存储。纯数据与逻辑,无 Vite/ws 依赖。
- `apps/web/src/mock/clients.ts` — 活跃 WebSocket 连接集合,按 runId 广播事件。依赖 `ws`。
- `apps/web/src/mock/scenario.ts` — 快路剧本:定时器序列推进状态 + 派发事件。依赖 state、clients。
- `apps/web/src/mock/router.ts` — REST 路由分发。依赖 state(写)、scenario(触发剧本)。
- `apps/web/src/mock/index.ts` — Vite 插件入口:开关、注册 router、挂 `upgrade` 监听。依赖上述全部。
- `apps/web/src/mock/state.test.ts` — state 单元测试。
- `apps/web/src/mock/scenario.test.ts` — scenario 单元测试。

修改文件:

- `apps/web/vite.config.ts` — 引入 mock 插件。
- `apps/web/package.json` — 加 `ws`、`@types/ws` devDeps。

依赖顺序:`state.ts` → `clients.ts` → `scenario.ts` → `router.ts` → `index.ts` → 接线 + 手测。

---

## Task 1: 内存状态模块 state.ts(TDD)

**Files:**

- Create: `apps/web/src/mock/state.ts`
- Test: `apps/web/src/mock/state.test.ts`

**Interfaces:**

- Consumes: `apps/web/src/lib/contracts.ts` 的类型(`TaskSummary`、`RunDetail`、`RuntimeSnapshot`、`EventEnvelope`、`ToolCallSummary`、`AgentPlan`、`BudgetSnapshot`、`UsageSnapshot`、`FinalReport` 等)。
- Produces(后续任务依赖的精确签名):

```ts
export interface MockState {
  owner: { id: string; username: string } | null;
  tasks: Map<string, TaskSummary>;
  runs: Map<string, RunDetail>;
  runtimes: Map<string, RuntimeSnapshot>;
  events: EventEnvelope[]; // 全局事件,sequence 单调递增
  scenarios: Map<string, ScenarioHandle>; // runId -> 剧本控制句柄
}

export interface ScenarioHandle {
  runId: string;
  step: number;
  timer: ReturnType<typeof setTimeout> | undefined;
  finished: boolean;
}

export function createState(): MockState;
export function createTask(state: MockState, title: string, goal: string): TaskSummary;
export function getTask(state: MockState, id: string): TaskSummary | undefined;
export function listTasks(state: MockState): TaskSummary[];
export function getRun(state: MockState, id: string): RunDetail | undefined;
export function getRuntime(state: MockState, id: string): RuntimeSnapshot | undefined;
export function setRunStatus(
  state: MockState,
  runId: string,
  status: RunStatus,
  reason?: string
): RunDetail;
export function appendEvent(
  state: MockState,
  event: Omit<EventEnvelope, "sequence" | "createdAt"> & { id: string }
): EventEnvelope;
export function listEvents(state: MockState, after: number, runId?: string): EventEnvelope[];
export function updateToolCall(
  state: MockState,
  runId: string,
  callId: string,
  patch: Partial<ToolCallSummary>
): ToolCallSummary;
```

- [ ] **Step 1: Write the failing test**

Create `apps/web/src/mock/state.test.ts`:

```ts
import { test, expect } from "bun:test";
import {
  createState,
  createTask,
  getRuntime,
  setRunStatus,
  appendEvent,
  listEvents
} from "./state";

test("createTask creates task, run, and runtime snapshot", () => {
  const state = createState();
  const task = createTask(state, "Demo", "Do something");
  expect(task.title).toBe("Demo");
  expect(task.status).toBe("queued");
  expect(task.latestRunId).toBeTruthy();

  const runtime = getRuntime(state, task.latestRunId);
  expect(runtime).toBeDefined();
  expect(runtime!.checkpoint).toBe("preparing");
  expect(runtime!.plan.steps).toEqual([]);
  expect(runtime!.toolCalls).toEqual([]);
});

test("setRunStatus transitions run status", () => {
  const state = createState();
  const task = createTask(state, "T", "G");
  const run = setRunStatus(state, task.latestRunId, "running", "started");
  expect(run.status).toBe("running");
  expect(run.statusReason).toBe("started");
});

test("appendEvent assigns monotonic sequence and returns envelope", () => {
  const state = createState();
  const e1 = appendEvent(state, { id: "e1", runId: "r1", type: "run.created", payload: {} });
  const e2 = appendEvent(state, { id: "e2", runId: "r1", type: "run.status_changed", payload: {} });
  expect(e1.sequence).toBe(1);
  expect(e2.sequence).toBe(2);
  expect(e1.createdAt).toBeGreaterThan(0);
});

test("listEvents filters by after cursor and runId", () => {
  const state = createState();
  appendEvent(state, { id: "a", runId: "r1", type: "run.created", payload: {} });
  appendEvent(state, { id: "b", runId: "r2", type: "run.created", payload: {} });
  appendEvent(state, { id: "c", runId: "r1", type: "run.status_changed", payload: {} });
  // sequence: a=1, b=2, c=3
  expect(listEvents(state, 0).map((e) => e.id)).toEqual(["a", "b", "c"]);
  expect(listEvents(state, 0, "r1").map((e) => e.id)).toEqual(["a", "c"]);
  expect(listEvents(state, 1, "r1").map((e) => e.id)).toEqual(["c"]);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test --cwd apps/web src/mock/state.test.ts`
Expected: FAIL — `Cannot find module "./state"` (file does not exist).

- [ ] **Step 3: Write minimal implementation**

Create `apps/web/src/mock/state.ts`:

```ts
import type {
  TaskSummary,
  RunDetail,
  RuntimeSnapshot,
  EventEnvelope,
  ToolCallSummary,
  RunStatus
} from "$lib/contracts";

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

export function createState(): MockState {
  return {
    owner: null,
    tasks: new Map(),
    runs: new Map(),
    runtimes: new Map(),
    events: [],
    scenarios: new Map()
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test --cwd apps/web src/mock/state.test.ts`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add apps/web/src/mock/state.ts apps/web/src/mock/state.test.ts
git commit -m "feat(mock): add in-memory state module for webui mock"
```

---

## Task 2: WebSocket 客户端集合 clients.ts

**Files:**

- Create: `apps/web/src/mock/clients.ts`

**Interfaces:**

- Consumes: `ws` 库的 `WebSocket` 类型。
- Produces:

```ts
export interface MockClients {
  add(runId: string, socket: WebSocket): void;
  remove(socket: WebSocket): void;
  broadcast(runId: string, message: string): void;
  size(): number;
}
export function createClients(): MockClients;
```

> 注:本任务无独立单测(行为依赖真实 ws 连接,在 Task 6 手测覆盖)。实现极简,保持聚焦。

- [ ] **Step 1: Write minimal implementation**

Create `apps/web/src/mock/clients.ts`:

```ts
import type WebSocket from "ws";

export interface MockClients {
  add(runId: string, socket: WebSocket): void;
  remove(socket: WebSocket): void;
  broadcast(runId: string, message: string): void;
  size(): number;
}

export function createClients(): MockClients {
  const byRun = new Map<string, Set<WebSocket>>();

  return {
    add(runId, socket) {
      let set = byRun.get(runId);
      if (!set) {
        set = new Set();
        byRun.set(runId, set);
      }
      set.add(socket);
    },
    remove(socket) {
      for (const set of byRun.values()) set.delete(socket);
    },
    broadcast(runId, message) {
      const set = byRun.get(runId);
      if (!set) return;
      for (const socket of set) {
        if (socket.readyState === socket.OPEN) socket.send(message);
      }
    },
    size() {
      let total = 0;
      for (const set of byRun.values()) total += set.size;
      return total;
    }
  };
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/web/src/mock/clients.ts
git commit -m "feat(mock): add websocket client registry"
```

---

## Task 3: 快路剧本 scenario.ts(TDD)

**Files:**

- Create: `apps/web/src/mock/scenario.ts`
- Test: `apps/web/src/mock/scenario.test.ts`

**Interfaces:**

- Consumes: Task 1 的 `MockState`、`setRunStatus`、`appendEvent`、`updateToolCall`、`ScenarioHandle`;Task 2 的 `MockClients`。
- Produces:

```ts
export interface ScenarioDeps {
  state: MockState;
  clients: MockClients;
  stepDelayMs?: number; // 默认 1500
  now?: () => number; // 注入时钟,便于测试;默认 Date.now
  schedule?: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>; // 注入调度,便于测试
}
export function startScenario(deps: ScenarioDeps, runId: string): void;
export function approveToolCall(deps: ScenarioDeps, runId: string, callId: string): void;
export function denyToolCall(deps: ScenarioDeps, runId: string, callId: string): void;
export function cancelScenario(deps: ScenarioDeps, runId: string): void;
```

剧本步骤(确定性序列,每步派发事件并更新 runtime,再广播给 WS):

```
step 0: queued → planning        setRunStatus planning; plan.summary + steps;
        checkpoint=calling_model; appendEvent agent.plan_updated
step 1: planning → running       tool.call_started(workspace_write tool);
        toolCall.status=running; appendEvent tool.call_started
step 2: running → waiting_for_approval  run status; toolCall.status=waiting_for_approval;
        appendEvent run.status_changed (to=waiting_for_approval); 停住等用户
[approve]: waiting → running    tool.call_completed; toolCall.status=completed;
        appendEvent tool.call_completed
step 3: running → verifying     setRunStatus verifying; appendEvent run.status_changed
step 4: verifying → reporting   checkpoint=reporting; 构造 FinalReport; appendEvent run.reported
step 5: reporting → succeeded   setRunStatus succeeded; appendEvent run.status_changed(to=succeeded); finished=true
```

- [ ] **Step 1: Write the failing test**

Create `apps/web/src/mock/scenario.test.ts`:

```ts
import { test, expect } from "bun:test";
import { createState, createTask, getRuntime, getRun } from "./state";
import { createClients } from "./clients";
import { startScenario, approveToolCall, denyToolCall, cancelScenario } from "./scenario";

interface FakeTimers {
  pending: Array<{ fn: () => void; ms: number }>;
  install(): { schedule: (fn: () => void, ms: number) => any; flush: () => void };
}

function fakeTimers(): FakeTimers["install"] {
  const pending: Array<{ fn: () => void; ms: number }> = [];
  return {
    schedule: (fn, ms) => {
      const handle = { fn, ms, fired: false } as any;
      pending.push({ fn, ms });
      return handle;
    },
    flush: () => {
      while (pending.length) {
        const item = pending.shift()!;
        item.fn();
      }
    }
  };
}

function deps() {
  const state = createState();
  const clients = createClients();
  const timers = fakeTimers();
  return {
    state,
    clients,
    stepDelayMs: 0,
    schedule: timers.schedule,
    flush: timers.flush
  };
}

test("scenario advances queued→planning on first step", () => {
  const d = deps();
  const task = createTask(d.state, "T", "G");
  startScenario(d, task.latestRunId);
  d.flush(); // step 0
  expect(getRun(d.state, task.latestRunId)!.status).toBe("planning");
  const runtime = getRuntime(d.state, task.latestRunId)!;
  expect(runtime.plan.summary.length).toBeGreaterThan(0);
  expect(runtime.checkpoint).toBe("calling_model");
});

test("scenario reaches waiting_for_approval after two steps and stops", () => {
  const d = deps();
  const task = createTask(d.state, "T", "G");
  startScenario(d, task.latestRunId);
  d.flush(); // step 0 planning
  d.flush(); // step 1 tool.call_started (running)
  d.flush(); // step 2 waiting_for_approval
  expect(getRun(d.state, task.latestRunId)!.status).toBe("waiting_for_approval");
  const runtime = getRuntime(d.state, task.latestRunId)!;
  expect(runtime.toolCalls[0].status).toBe("waiting_for_approval");
  // nothing more scheduled until approve
  d.flush();
  expect(getRun(d.state, task.latestRunId)!.status).toBe("waiting_for_approval");
});

test("approve advances to succeeded with report", () => {
  const d = deps();
  const task = createTask(d.state, "T", "G");
  startScenario(d, task.latestRunId);
  d.flush();
  d.flush();
  d.flush(); // -> waiting_for_approval
  const callId = getRuntime(d.state, task.latestRunId)!.toolCalls[0].callId;
  approveToolCall(d, task.latestRunId, callId);
  d.flush(); // step 3 verifying
  d.flush(); // step 4 reporting
  d.flush(); // step 5 succeeded
  expect(getRun(d.state, task.latestRunId)!.status).toBe("succeeded");
  const runtime = getRuntime(d.state, task.latestRunId)!;
  expect(runtime.report).not.toBeNull();
  expect(runtime.report!.outcome).toBe("succeeded");
});

test("deny sends run to failed", () => {
  const d = deps();
  const task = createTask(d.state, "T", "G");
  startScenario(d, task.latestRunId);
  d.flush();
  d.flush();
  d.flush();
  const callId = getRuntime(d.state, task.latestRunId)!.toolCalls[0].callId;
  denyToolCall(d, task.latestRunId, callId);
  expect(getRun(d.state, task.latestRunId)!.status).toBe("failed");
});

test("cancel stops scenario and sets cancelled", () => {
  const d = deps();
  const task = createTask(d.state, "T", "G");
  startScenario(d, task.latestRunId);
  d.flush();
  cancelScenario(d, task.latestRunId);
  expect(getRun(d.state, task.latestRunId)!.status).toBe("cancelled");
  d.flush(); // no further progression
  expect(getRun(d.state, task.latestRunId)!.status).toBe("cancelled");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test --cwd apps/web src/mock/scenario.test.ts`
Expected: FAIL — `Cannot find module "./scenario"`.

- [ ] **Step 3: Write minimal implementation**

Create `apps/web/src/mock/scenario.ts`:

```ts
import type { RuntimeSnapshot, FinalReport, ToolCallSummary } from "$lib/contracts";
import type { MockState, ScenarioHandle } from "./state";
import { setRunStatus, appendEvent, updateToolCall } from "./state";
import type { MockClients } from "./clients";

export interface ScenarioDeps {
  state: MockState;
  clients: MockClients;
  stepDelayMs?: number;
  schedule?: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>;
}

const DEFAULT_STEP_MS = 1500;

function rid(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 10)}`;
}

function broadcastEvent(deps: ScenarioDeps, envelope: ReturnType<typeof appendEvent>): void {
  deps.clients.broadcast(envelope.runId, JSON.stringify(envelope));
}

function next(deps: ScenarioDeps, runId: string, fn: () => void): void {
  const schedule = deps.schedule ?? ((f, ms) => setTimeout(f, ms));
  schedule(fn, deps.stepDelayMs ?? DEFAULT_STEP_MS);
}

function ensureHandle(deps: ScenarioDeps, runId: string): ScenarioHandle {
  const handle = deps.state.scenarios.get(runId);
  if (!handle) throw new Error(`scenario not found: ${runId}`);
  return handle;
}

function makeToolCall(runId: string): ToolCallSummary {
  return {
    callId: rid("call"),
    name: "workspace.replace",
    risk: "workspace_write",
    policy: "require_approval",
    status: "running",
    resultSummary: null,
    errorCategory: null,
    startedAt: Date.now(),
    completedAt: null
  };
}

function makeReport(): FinalReport {
  return {
    outcome: "succeeded",
    summary: "Mock task completed all required steps.",
    completed: ["Planned the work", "Applied the change", "Verified the result"],
    incomplete: [],
    risks: [],
    evidenceIds: [],
    rollback: null,
    usage: {
      modelTurns: 3,
      toolCalls: 1,
      inputTokens: 1200,
      outputTokens: 600,
      cachedInputTokens: 0,
      cacheWriteInputTokens: 0,
      elapsedMs: 4500
    },
    markdown: "# Mock Report\n\nAll steps completed successfully."
  };
}

function setCheckpoint(
  deps: ScenarioDeps,
  runId: string,
  checkpoint: RuntimeSnapshot["checkpoint"]
): void {
  const runtime = deps.state.runtimes.get(runId);
  if (runtime) runtime.checkpoint = checkpoint;
}

function advance(deps: ScenarioDeps, runId: string): void {
  const handle = ensureHandle(deps, runId);
  if (handle.finished) return;

  switch (handle.step) {
    case 0: {
      setRunStatus(deps.state, runId, "planning");
      setCheckpoint(deps, runId, "calling_model");
      const runtime = deps.state.runtimes.get(runId)!;
      runtime.plan = {
        summary: "Inspect workspace, apply the change, then verify.",
        steps: [
          { id: "s1", title: "Inspect workspace", status: "pending", required: true },
          { id: "s2", title: "Apply change", status: "pending", required: true },
          { id: "s3", title: "Verify result", status: "pending", required: true }
        ]
      };
      broadcastEvent(
        deps,
        appendEvent(deps.state, {
          id: rid("evt"),
          runId,
          type: "agent.plan_updated",
          payload: { plan: runtime.plan }
        })
      );
      handle.step = 1;
      next(deps, runId, () => advance(deps, runId));
      break;
    }
    case 1: {
      const runtime = deps.state.runtimes.get(runId)!;
      const call = makeToolCall(runId);
      runtime.toolCalls = [call];
      setRunStatus(deps.state, runId, "running");
      broadcastEvent(
        deps,
        appendEvent(deps.state, {
          id: rid("evt"),
          runId,
          type: "tool.call_started",
          payload: { callId: call.callId, name: call.name }
        })
      );
      handle.step = 2;
      next(deps, runId, () => advance(deps, runId));
      break;
    }
    case 2: {
      const runtime = deps.state.runtimes.get(runId)!;
      const call = runtime.toolCalls[0];
      updateToolCall(deps.state, runId, call.callId, { status: "waiting_for_approval" });
      setRunStatus(deps.state, runId, "waiting_for_approval");
      broadcastEvent(
        deps,
        appendEvent(deps.state, {
          id: rid("evt"),
          runId,
          type: "run.status_changed",
          payload: { from: "running", to: "waiting_for_approval" }
        })
      );
      handle.step = 3;
      // 停住,等 approveToolCall 推进
      break;
    }
    case 3: {
      setRunStatus(deps.state, runId, "verifying");
      setCheckpoint(deps, runId, "validating_completion");
      broadcastEvent(
        deps,
        appendEvent(deps.state, {
          id: rid("evt"),
          runId,
          type: "run.status_changed",
          payload: { from: "running", to: "verifying" }
        })
      );
      handle.step = 4;
      next(deps, runId, () => advance(deps, runId));
      break;
    }
    case 4: {
      const runtime = deps.state.runtimes.get(runId)!;
      runtime.checkpoint = "reporting";
      runtime.report = makeReport();
      broadcastEvent(
        deps,
        appendEvent(deps.state, {
          id: rid("evt"),
          runId,
          type: "run.reported",
          payload: { report: runtime.report }
        })
      );
      handle.step = 5;
      next(deps, runId, () => advance(deps, runId));
      break;
    }
    case 5: {
      setRunStatus(deps.state, runId, "succeeded");
      broadcastEvent(
        deps,
        appendEvent(deps.state, {
          id: rid("evt"),
          runId,
          type: "run.status_changed",
          payload: { from: "reporting", to: "succeeded" }
        })
      );
      handle.finished = true;
      break;
    }
  }
}

export function startScenario(deps: ScenarioDeps, runId: string): void {
  const handle = ensureHandle(deps, runId);
  handle.step = 0;
  next(deps, runId, () => advance(deps, runId));
}

export function approveToolCall(deps: ScenarioDeps, runId: string, callId: string): void {
  const handle = ensureHandle(deps, runId);
  if (handle.finished) return;
  updateToolCall(deps.state, runId, callId, {
    status: "completed",
    resultSummary: "Applied the change to the workspace.",
    completedAt: Date.now()
  });
  setRunStatus(deps.state, runId, "running");
  const runtime = deps.state.runtimes.get(runId)!;
  broadcastEvent(
    deps,
    appendEvent(deps.state, {
      id: rid("evt"),
      runId,
      type: "tool.call_completed",
      payload: { callId, resultSummary: "Applied the change to the workspace." }
    })
  );
  void runtime;
  handle.step = 3;
  next(deps, runId, () => advance(deps, runId));
}

export function denyToolCall(deps: ScenarioDeps, runId: string, callId: string): void {
  const handle = ensureHandle(deps, runId);
  if (handle.finished) return;
  updateToolCall(deps.state, runId, callId, { status: "denied" });
  setRunStatus(deps.state, runId, "failed", "Tool call denied");
  if (handle.timer) clearTimeout(handle.timer);
  handle.finished = true;
  broadcastEvent(
    deps,
    appendEvent(deps.state, {
      id: rid("evt"),
      runId,
      type: "run.status_changed",
      payload: { from: "waiting_for_approval", to: "failed", reason: "Tool call denied" }
    })
  );
}

export function cancelScenario(deps: ScenarioDeps, runId: string): void {
  const handle = ensureHandle(deps, runId);
  if (handle.timer) clearTimeout(handle.timer);
  handle.finished = true;
  setRunStatus(deps.state, runId, "cancelled", "Cancelled by owner");
  broadcastEvent(
    deps,
    appendEvent(deps.state, {
      id: rid("evt"),
      runId,
      type: "run.status_changed",
      payload: { from: "planning", to: "cancelled", reason: "Cancelled by owner" }
    })
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test --cwd apps/web src/mock/scenario.test.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add apps/web/src/mock/scenario.ts apps/web/src/mock/scenario.test.ts
git commit -m "feat(mock): add happy-path run lifecycle scenario"
```

---

## Task 4: REST 路由分发 router.ts

**Files:**

- Create: `apps/web/src/mock/router.ts`

**Interfaces:**

- Consumes: Task 1 `state`(`createState` 及查询/写函数)、Task 3 `scenario`(start/approve/deny/cancel)。Vite `Connect`/`http` 风格中间件签名。
- Produces:

```ts
export interface MockRouterDeps {
  state: MockState;
  scenario: ScenarioDeps;
}
export function createMockRouter(
  deps: MockRouterDeps
): (req: IncomingMessage, res: ServerResponse) => Promise<boolean>;
// 返回 true 表示已处理(命中 /api),false 表示未命中交还 Vite
```

**契约对齐**(`docs/api/openapi.yaml`):

- `POST /api/auth/login` → 200 `{ owner }` + `Set-Cookie: soloops_session=mock-<rand>; HttpOnly; Path=/`
- `GET /api/auth/session` → 有 cookie 200 `{ owner }`,否则 401 `{ error: { code: "unauthorized", message } }`
- `POST /api/auth/logout` → 204,清除会话
- `GET /api/tasks` → 200 `{ items }`
- `POST /api/tasks` → 201 `TaskSummary`(触发 `startScenario`)
- `GET /api/tasks/:id` → 200 或 404
- `GET /api/runs/:id` → 200 或 404
- `GET /api/runs/:id/runtime` → 200 或 404
- `POST /api/runs/:id/cancel` → 200 `RunDetail`(调用 `cancelScenario`)
- `POST /api/runs/:id/tool-calls/:callId/decision` → 200 `ToolCallSummary`(approve→`approveToolCall`,deny→`denyToolCall`)
- `GET /api/events?after=&runId=` → 200 `{ items }`
- `GET /healthz|/livez|/readyz` → 200 最小桩;`GET /metrics` → 200 text 桩
- 404 `{ error: { code: "not_found", message } }`

- [ ] **Step 1: Write minimal implementation**

Create `apps/web/src/mock/router.ts`:

```ts
import type { IncomingMessage, ServerResponse } from "node:http";
import type { MockState } from "./state";
import type { ScenarioDeps } from "./scenario";
import {
  createTask,
  getTask,
  listTasks,
  getRun,
  getRuntime,
  setRunStatus,
  listEvents,
  updateToolCall
} from "./state";
import { startScenario, approveToolCall, denyToolCall, cancelScenario } from "./scenario";
import type { TaskSummary, RunDetail, ToolCallSummary, EventEnvelope } from "$lib/contracts";

export interface MockRouterDeps {
  state: MockState;
  scenario: ScenarioDeps;
}

function rid(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 10)}`;
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.statusCode = status;
  res.setHeader("content-type", "application/json");
  res.setHeader("content-length", Buffer.byteLength(payload));
  res.end(payload);
}

function sendError(res: ServerResponse, status: number, code: string, message: string): void {
  sendJson(res, status, { error: { code, message } });
}

function send204(res: ServerResponse): void {
  res.statusCode = 204;
  res.end();
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve) => {
    let data = "";
    req.on("data", (chunk) => (data += chunk));
    req.on("end", () => resolve(data));
  });
}

function hasSession(req: IncomingMessage): boolean {
  const cookie = req.headers.cookie ?? "";
  return /soloops_session=mock-/.test(cookie);
}

function splitPath(url: string): string[] {
  const path = url.split("?")[0];
  return path.split("/").filter(Boolean);
}

export function createMockRouter(deps: MockRouterDeps) {
  return async function route(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    const segments = splitPath(req.url ?? "");
    if (
      segments[0] !== "api" &&
      !["healthz", "livez", "readyz", "metrics"].includes(segments[0] ?? "")
    ) {
      return false;
    }

    const method = req.method ?? "GET";

    // health
    if (segments[0] === "healthz" || segments[0] === "livez") {
      sendJson(res, 200, { status: "ok", service: "soloops-mock" });
      return true;
    }
    if (segments[0] === "readyz") {
      sendJson(res, 200, { status: "ready" });
      return true;
    }
    if (segments[0] === "metrics") {
      res.statusCode = 200;
      res.setHeader("content-type", "text/plain");
      res.end("# soloops mock metrics\n");
      return true;
    }

    // /api/auth/*
    if (segments[1] === "auth") {
      if (segments[2] === "login" && method === "POST") {
        deps.state.owner = { id: rid("owner"), username: "owner" };
        res.setHeader("Set-Cookie", `soloops_session=mock-${rid("sess")}; HttpOnly; Path=/`);
        sendJson(res, 200, { owner: deps.state.owner });
        return true;
      }
      if (segments[2] === "session" && method === "GET") {
        if (!hasSession(req))
          return (sendError(res, 401, "unauthorized", "Session required"), true);
        sendJson(res, 200, { owner: deps.state.owner ?? { id: "owner", username: "owner" } });
        return true;
      }
      if (segments[2] === "logout" && method === "POST") {
        deps.state.owner = null;
        send204(res);
        return true;
      }
    }

    if (!hasSession(req)) return (sendError(res, 401, "unauthorized", "Session required"), true);

    // /api/tasks
    if (segments[1] === "tasks") {
      if (method === "GET" && segments.length === 2) {
        sendJson(res, 200, { items: listTasks(deps.state) });
        return true;
      }
      if (method === "POST" && segments.length === 2) {
        const body = JSON.parse((await readBody(req)) || "{}");
        const task: TaskSummary = createTask(
          deps.state,
          String(body.title ?? ""),
          String(body.goal ?? "")
        );
        startScenario(deps.scenario, task.latestRunId);
        res.statusCode = 201;
        sendJson(res, 201, task);
        return true;
      }
      if (method === "GET" && segments.length === 3) {
        const task = getTask(deps.state, segments[2]);
        if (!task) return (sendError(res, 404, "not_found", "Task not found"), true);
        sendJson(res, 200, task);
        return true;
      }
    }

    // /api/runs/*
    if (segments[1] === "runs" && segments.length >= 3) {
      const runId = segments[2];
      if (method === "GET" && segments.length === 3) {
        const run = getRun(deps.state, runId);
        if (!run) return (sendError(res, 404, "not_found", "Run not found"), true);
        sendJson(res, 200, run);
        return true;
      }
      if (method === "GET" && segments.length === 4 && segments[3] === "runtime") {
        const runtime = getRuntime(deps.state, runId);
        if (!runtime) return (sendError(res, 404, "not_found", "Runtime not found"), true);
        sendJson(res, 200, runtime);
        return true;
      }
      if (method === "POST" && segments.length === 4 && segments[3] === "cancel") {
        cancelScenario(deps.scenario, runId);
        const run = getRun(deps.state, runId);
        if (!run) return (sendError(res, 404, "not_found", "Run not found"), true);
        sendJson(res, 200, run);
        return true;
      }
      // POST /api/runs/:runId/tool-calls/:callId/decision
      // 路径段 ["api","runs",runId,"tool-calls",callId,"decision"] 共 6 段
      if (
        method === "POST" &&
        segments.length === 6 &&
        segments[3] === "tool-calls" &&
        segments[5] === "decision"
      ) {
        const callId = decodeURIComponent(segments[4]);
        const body = JSON.parse((await readBody(req)) || "{}");
        const decision = String(body.decision ?? "");
        if (decision === "approve") approveToolCall(deps.scenario, runId, callId);
        else if (decision === "deny") denyToolCall(deps.scenario, runId, callId);
        else
          return (sendError(res, 400, "invalid_request", "decision must be approve or deny"), true);
        const runtime = getRuntime(deps.state, runId);
        const call = runtime?.toolCalls.find((c) => c.callId === callId);
        if (!call) return (sendError(res, 404, "not_found", "Tool call not found"), true);
        sendJson(res, 200, call);
        return true;
      }
    }

    // /api/events
    if (segments[1] === "events" && method === "GET") {
      const url = new URL(req.url ?? "", "http://mock");
      const after = Number(url.searchParams.get("after") ?? "0");
      const runId = url.searchParams.get("runId") ?? undefined;
      sendJson(res, 200, { items: listEvents(deps.state, after, runId) });
      return true;
    }

    return (sendError(res, 404, "not_found", "Not found"), true);
  };
}
```

> ⚠️ 上面 decision 分支写得绕了,实现时请用下面的**修正版**替换整个 decision 分支。修正版(实现时采用):

```ts
// POST /api/runs/:runId/tool-calls/:callId/decision
if (
  method === "POST" &&
  segments.length === 6 &&
  segments[3] === "tool-calls" &&
  segments[5] === "decision"
) {
  const callId = decodeURIComponent(segments[4]);
  const body = JSON.parse((await readBody(req)) || "{}");
  const decision = String(body.decision ?? "");
  if (decision === "approve") approveToolCall(deps.scenario, runId, callId);
  else if (decision === "deny") denyToolCall(deps.scenario, runId, callId);
  else return (sendError(res, 400, "invalid_request", "decision must be approve or deny"), true);
  const runtime = getRuntime(deps.state, runId);
  const call = runtime?.toolCalls.find((c) => c.callId === callId);
  if (!call) return (sendError(res, 404, "not_found", "Tool call not found"), true);
  sendJson(res, 200, call);
  return true;
}
```

> 实现说明:把上面 router 中那段有缺陷的 decision 处理整段删除,用此修正版替换。路径段 `segments` 对 `/api/runs/:runId/tool-calls/:callId/decision` 共 6 段:`["api","runs",runId,"tool-calls",callId,"decision"]`。

- [ ] **Step 2: Commit**

```bash
git add apps/web/src/mock/router.ts
git commit -m "feat(mock): add REST router aligned to openapi contract"
```

---

## Task 5: Vite 插件入口 index.ts + WebSocket 升级

**Files:**

- Create: `apps/web/src/mock/index.ts`

**Interfaces:**

- Consumes: Task 1-4。Vite `Plugin` 类型。
- Produces: `export function mockPlugin(): Plugin`。

职责:

- 仅 `command === "serve"` 生效。
- `configureServer(server)`:创建 state/clients/scenario/router;注册中间件拦截 `/api` REST(命中则 `res.end()` 不调 `next`);挂 `server.httpServer.on("upgrade")` 处理 `/api/events` 的 WS 升级。
- WS 升级:用 `ws` 的 `WebSocketServer` + `noServer:true`,手动 `handleUpgrade`;解析 query 的 `runId` 与 `after`。
- WS 连接建立后:先补拉 `after` 之后的存量事件(`listEvents`)逐条 send,再 `clients.add(runId, socket)`;`on('close')` 时 `clients.remove`。

- [ ] **Step 1: Write minimal implementation**

Create `apps/web/src/mock/index.ts`:

```ts
import type { Plugin } from "vite";
import { WebSocketServer, type WebSocket } from "ws";
import type { IncomingMessage } from "node:http";
import { createState, listEvents } from "./state";
import { createClients } from "./clients";
import type { ScenarioDeps } from "./scenario";
import { createMockRouter } from "./router";

export function mockPlugin(): Plugin {
  return {
    name: "soloops-mock",
    apply: "serve",
    configureServer(server) {
      if (process.env.VITE_MOCK !== "1") return;

      const state = createState();
      const clients = createClients();
      const scenario: ScenarioDeps = { state, clients };
      const route = createMockRouter({ state, scenario });

      // REST 中间件
      server.middlewares.use(async (req, res, next) => {
        const url = req.url ?? "";
        if (
          !url.startsWith("/api/") &&
          !["/healthz", "/livez", "/readyz", "/metrics"].includes(url.split("?")[0])
        ) {
          return next();
        }
        const handled = await route(req as IncomingMessage, res);
        if (!handled) next();
      });

      // WebSocket 升级
      const wss = new WebSocketServer({ noServer: true });
      server.httpServer.on("upgrade", (req, socket, head) => {
        const url = req.url ?? "";
        if (!url.startsWith("/api/events")) {
          return; // 交还默认行为
        }
        const parsed = new URL(url, "http://mock");
        const runId = parsed.searchParams.get("runId") ?? undefined;
        const after = Number(parsed.searchParams.get("after") ?? "0");
        wss.handleUpgrade(req, socket, head, (ws) => {
          const conn = ws as WebSocket;
          // 先补拉存量事件
          for (const evt of listEvents(state, after, runId)) {
            if (conn.readyState === conn.OPEN) conn.send(JSON.stringify(evt));
          }
          if (runId) clients.add(runId, conn);
          conn.on("close", () => clients.remove(conn));
        });
      });
    }
  };
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/web/src/mock/index.ts
git commit -m "feat(mock): add vite plugin with rest and websocket handling"
```

---

## Task 6: 接线 vite.config.ts + 依赖 + 手测验收

**Files:**

- Modify: `apps/web/vite.config.ts`
- Modify: `apps/web/package.json`

**Interfaces:**

- Consumes: Task 5 `mockPlugin`。

- [ ] **Step 1: Add dependencies**

Run:

```bash
cd apps/web && bun add -d ws @types/ws
```

Expected: `ws` 与 `@types/ws` 出现在 `apps/web/package.json` 的 `devDependencies`。

- [ ] **Step 2: Wire plugin into vite.config.ts**

Modify `apps/web/vite.config.ts` to:

```ts
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import { mockPlugin } from "./src/mock";

export default defineConfig({
  plugins: [tailwindcss(), sveltekit(), mockPlugin()],
  server: {
    host: "127.0.0.1",
    proxy: {
      "/api": {
        target: "http://127.0.0.1:3001",
        ws: true
      }
    }
  }
});
```

> 说明:`mockPlugin` 内部 `apply: "serve"` 且仅 `VITE_MOCK=1` 时生效;关闭时代理照常工作。开启时,mock 中间件命中 `/api` 后响应并不调用 `next`,从而短路代理。

- [ ] **Step 3: Verify checks pass**

Run:

```bash
bun run --cwd apps/web check
bun run lint
```

Expected: svelte-check 与 eslint 通过,无类型错误。`bun test --cwd apps/web` 全部通过。

- [ ] **Step 4: Manual smoke test (happy path)**

Run(分一个终端):

```bash
$env:VITE_MOCK=1; bun run dev:web
```

在浏览器 `http://127.0.0.1:5173`:

1. 登录页,用户名任意、密码任意,点 Sign in → 跳转 `/tasks`。
2. 点 New task → 填 title/goal → Create task → 跳转 Run timeline。
3. 观察:Plan 出现 summary 与 3 个 step;Budget 显示 usage/上限;Tool call 出现且状态从 running → waiting_for_approval,出现 Approve/Deny 按钮。
4. 点 Approve → Tool call completed(带 resultSummary)→ 状态 → verifying → reporting → succeeded;Final report 区域显示 markdown。
5. 再建一个任务,到 waiting_for_approval 后点 Deny → run 状态 failed,显示 statusReason。
6. 再建一个任务,运行中点 Cancel → run 状态 cancelled。
7. 期间刷新页面:事件 timeline 不丢(WS 重连 + HTTP 补拉恢复增量)。

Expected: 全部 UI 状态如预期呈现;无控制台报错。

- [ ] **Step 5: Verify mock off does not break dev**

关闭 `VITE_MOCK`(不设或设为 0),`bun run dev:web` 正常启动,`/api` 仍尝试代理到 `:3001`(无后端时返回 502/连接失败属正常,证明代理未被短路)。

- [ ] **Step 6: Commit**

```bash
git add apps/web/vite.config.ts apps/web/package.json
git commit -m "feat(mock): wire mock plugin into vite dev server"
```

---

## Self-Review

**1. Spec coverage:**

- Vite 插件形态 + `VITE_MOCK` 开关 → Task 5 + Task 6 ✓
- REST 拦截阻止代理 → Task 5 中间件 + Task 6 注释 ✓
- WebSocket 升级拦截 → Task 5 ✓
- 单条快路剧本状态序列 → Task 3 ✓
- Deny → failed、Cancel → cancelled → Task 3 ✓
- 事件 sequence 单调、WS/HTTP 共用源 → Task 1 `appendEvent`/`listEvents` + Task 5 补拉 ✓
- 接口契约对齐(login/session/logout/tasks/runs/runtime/decision/cancel/events/health) → Task 4 ✓
- 404/401 ApiError → Task 4 ✓
- 组件拆分 5 文件 → Task 1-5 ✓
- 构建 boundary(ws 仅 dev)→ Task 5 `apply: "serve"` + Task 6 ✓
- 单元测试(state/scenario) → Task 1/3 ✓
- 手测验收路径 → Task 6 Step 4 ✓

**2. Placeholder scan:** Task 4 决策分支已给出修正版替换说明,无 TBD/TODO。其余步骤均含完整代码。已用具体代码替换占位。

**3. Type consistency:** `createState`/`createTask`/`getRun`/`getRuntime`/`setRunStatus`/`appendEvent`/`listEvents`/`updateToolCall`(Task 1)在 Task 3/4 中调用名一致;`MockClients` 的 `add/remove/broadcast`(Task 2)在 Task 3/5 调用一致;`ScenarioDeps`(Task 3)在 Task 4/5 引用一致;`createMockRouter`(Task 4)与 `mockPlugin`(Task 5)签名一致。`ScenarioHandle.step` 在 Task 3 自增语义贯穿。
