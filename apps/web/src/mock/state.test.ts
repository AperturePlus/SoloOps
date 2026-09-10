import { test, expect } from "bun:test";
import {
  createState,
  createTask,
  getRuntime,
  setRunStatus,
  appendEvent,
  listEvents
} from "./state.ts";

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

test("ssh access fixture is internally consistent", () => {
  const state = createState();
  const entries = state.sshAccess.files.flatMap((file) => file.entries);
  expect(entries.filter((entry) => entry.valid).length).toBe(state.sshAccess.totalKeys);
  expect(entries.filter((entry) => !entry.valid).length).toBe(state.sshAccess.invalidLines);
  expect(state.sshAccess.machines.reduce((sum, machine) => sum + machine.keyCount, 0)).toBe(
    state.sshAccess.totalKeys
  );
});
