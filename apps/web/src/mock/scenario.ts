import type { RuntimeSnapshot, FinalReport, ToolCallSummary } from "$lib/contracts";
import type { MockState, ScenarioHandle } from "./state.ts";
import { setRunStatus, appendEvent, updateToolCall } from "./state.ts";
import type { MockClients } from "./clients.ts";

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
  if (envelope.runId) deps.clients.broadcast(envelope.runId, JSON.stringify(envelope));
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

function makeToolCall(): ToolCallSummary {
  return {
    callId: rid("call"),
    name: "workspace.replace",
    argumentsSha256: "0".repeat(64),
    approvalPreview: {
      action: "replace",
      path: "result.txt",
      summary: "Replace one workspace artifact"
    },
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
      inputTokens: 1560,
      outputTokens: 640,
      cachedInputTokens: 400,
      cacheWriteInputTokens: 0,
      elapsedMs: 6000
    },
    markdown: "# Mock Report\n\nAll steps completed successfully."
  };
}

function setPlanStep(
  deps: ScenarioDeps,
  runId: string,
  stepId: string,
  status: "pending" | "in_progress" | "completed"
): void {
  const runtime = deps.state.runtimes.get(runId);
  const step = runtime?.plan.steps.find((item) => item.id === stepId);
  if (step) step.status = status;
}

function addUsage(
  deps: ScenarioDeps,
  runId: string,
  patch: Partial<RuntimeSnapshot["usage"]>
): void {
  const runtime = deps.state.runtimes.get(runId);
  if (runtime) runtime.usage = { ...runtime.usage, ...patch };
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
      addUsage(deps, runId, {
        modelTurns: 1,
        inputTokens: 800,
        outputTokens: 400,
        elapsedMs: 1500
      });
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
      const call = makeToolCall();
      runtime.toolCalls = [call];
      setRunStatus(deps.state, runId, "running");
      setPlanStep(deps, runId, "s1", "completed");
      setPlanStep(deps, runId, "s2", "in_progress");
      addUsage(deps, runId, {
        modelTurns: 2,
        inputTokens: 1200,
        outputTokens: 500,
        elapsedMs: 3000
      });
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
      setPlanStep(deps, runId, "s3", "in_progress");
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
      setPlanStep(deps, runId, "s3", "completed");
      addUsage(deps, runId, {
        modelTurns: 3,
        inputTokens: 1560,
        outputTokens: 640,
        cachedInputTokens: 400,
        elapsedMs: 6000
      });
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
  setPlanStep(deps, runId, "s2", "completed");
  const runtime = deps.state.runtimes.get(runId)!;
  runtime.workspaceRevision = 1;
  addUsage(deps, runId, { toolCalls: 1, elapsedMs: 4500 });
  broadcastEvent(
    deps,
    appendEvent(deps.state, {
      id: rid("evt"),
      runId,
      type: "tool.call_completed",
      payload: { callId, resultSummary: "Applied the change to the workspace." }
    })
  );
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
