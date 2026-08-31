import { test, expect } from "bun:test";
import { createState, createTask, getRuntime, getRun } from "./state";
import { createClients } from "./clients";
import { startScenario, approveToolCall, denyToolCall, cancelScenario } from "./scenario";

function fakeTimers(): {
  schedule: (fn: () => void, ms: number) => any;
  flush: () => void;
} {
  const pending: Array<{ fn: () => void; ms: number }> = [];
  return {
    schedule: (fn, ms) => {
      pending.push({ fn, ms });
      return { fn, ms, fired: false } as any;
    },
    flush: () => {
      const item = pending.shift();
      if (item) item.fn();
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
