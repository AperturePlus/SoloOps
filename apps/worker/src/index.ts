import { closeDatabase, openDatabase } from "@soloops/db";
import { createLogger, loadConfig } from "@soloops/server-shared";
import { runOnce } from "./worker.ts";

const config = loadConfig();
const logger = createLogger("worker", config.logLevel);
const database = openDatabase(config.databasePath);
const workerId = `worker-${crypto.randomUUID()}`;
let stopping = false;

function stop(signal: string) {
  if (stopping) return;
  stopping = true;
  logger.info("Worker stopping", { signal, workerId });
}

process.on("SIGINT", () => stop("SIGINT"));
process.on("SIGTERM", () => stop("SIGTERM"));

logger.info("Worker started", { workerId, pollMs: config.workerPollMs });
try {
  while (!stopping) {
    const runId = runOnce(database, workerId);
    if (runId) logger.info("Stub run blocked safely", { runId });
    else await Bun.sleep(config.workerPollMs);
  }
} catch (error) {
  logger.error("Worker crashed", { error: error instanceof Error ? error.message : String(error) });
  process.exitCode = 1;
} finally {
  closeDatabase(database);
}

