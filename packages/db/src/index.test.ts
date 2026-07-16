import { afterEach, describe, expect, test } from "bun:test";
import {
  claimNextRun,
  closeDatabase,
  createOwner,
  createTaskWithRun,
  getRun,
  listEvents,
  openDatabase,
  runMigrations,
  transitionRun,
  type SoloOpsDatabase
} from "./index.ts";

const databases: SoloOpsDatabase[] = [];

function temporaryDatabase(): SoloOpsDatabase {
  const database = openDatabase(":memory:");
  databases.push(database);
  runMigrations(database);
  return database;
}

afterEach(() => {
  while (databases.length) closeDatabase(databases.pop()!);
});

describe("Phase 0 repository", () => {
  test("creates a task and an ordered event stream", () => {
    const database = temporaryDatabase();
    const owner = createOwner(database, "owner", "hash");
    const task = createTaskWithRun(database, owner.id, { title: "Demo", goal: "Exercise the worker" });
    expect(task.status).toBe("queued");

    const leased = claimNextRun(database, "worker-a");
    expect(leased?.id).toBe(task.latestRunId);
    transitionRun(database, task.latestRunId, "planning");
    transitionRun(database, task.latestRunId, "blocked", "Executor is not configured");

    const run = getRun(database, task.latestRunId);
    expect(run?.status).toBe("blocked");
    const events = listEvents(database, 0, task.latestRunId);
    expect(events.map((event) => event.sequence)).toEqual([...events.map((event) => event.sequence)].sort((a, b) => a - b));
    expect(listEvents(database, events[1]!.sequence, task.latestRunId).length).toBe(2);
  });

  test("leases a queued run only once", () => {
    const database = temporaryDatabase();
    const owner = createOwner(database, "owner", "hash");
    createTaskWithRun(database, owner.id, { title: "Only once", goal: "No duplicate work" });
    expect(claimNextRun(database, "worker-a")).not.toBeNull();
    expect(claimNextRun(database, "worker-b")).toBeNull();
  });
});
