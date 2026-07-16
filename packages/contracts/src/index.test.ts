import { describe, expect, test } from "bun:test";
import { CreateTaskRequestSchema, canTransitionRun } from "./index.ts";

describe("run state transitions", () => {
  test("allows the Phase 0 worker path", () => {
    expect(canTransitionRun("queued", "leased")).toBe(true);
    expect(canTransitionRun("leased", "planning")).toBe(true);
    expect(canTransitionRun("planning", "blocked")).toBe(true);
  });

  test("does not leave a terminal state", () => {
    expect(canTransitionRun("blocked", "running")).toBe(false);
    expect(canTransitionRun("succeeded", "queued")).toBe(false);
  });
});

test("task input is normalized and bounded", () => {
  expect(CreateTaskRequestSchema.parse({ title: " Demo ", goal: " Test " })).toEqual({
    title: "Demo",
    goal: "Test"
  });
  expect(() => CreateTaskRequestSchema.parse({ title: "", goal: "x" })).toThrow();
});

