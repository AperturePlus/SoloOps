import { claimNextRun, transitionRun, type SoloOpsDatabase } from "@soloops/db";

export const STUB_BLOCK_REASON = "Phase 0 executor is not configured; no task actions were performed.";

export function runOnce(database: SoloOpsDatabase, workerId: string): string | null {
  const run = claimNextRun(database, workerId);
  if (!run) return null;
  transitionRun(database, run.id, "planning");
  transitionRun(database, run.id, "blocked", STUB_BLOCK_REASON);
  return run.id;
}

