import { afterEach, expect, test } from "bun:test";
import {
  closeDatabase,
  createOwner,
  createTaskWithRun,
  getRun,
  openDatabase,
  runMigrations,
  type SoloOpsDatabase
} from "@soloops/db";
import { runOnce, STUB_BLOCK_REASON } from "./worker.ts";

let database: SoloOpsDatabase | undefined;

afterEach(() => {
  if (database) closeDatabase(database);
  database = undefined;
});

test("worker proves the pipeline without claiming task success", () => {
  database = openDatabase(":memory:");
  runMigrations(database);
  const owner = createOwner(database, "owner", "hash");
  const task = createTaskWithRun(database, owner.id, { title: "Stub", goal: "Do not fake success" });
  expect(runOnce(database, "worker-test")).toBe(task.latestRunId);
  expect(getRun(database, task.latestRunId)).toMatchObject({ status: "blocked", statusReason: STUB_BLOCK_REASON });
});
