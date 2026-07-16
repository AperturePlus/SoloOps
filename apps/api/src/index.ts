import { closeDatabase, openDatabase } from "@soloops/db";
import { loadConfig } from "@soloops/server-shared";
import { buildApp } from "./app.ts";

const config = loadConfig();
const database = openDatabase(config.databasePath);
const app = await buildApp({ database, config });

async function shutdown(signal: string) {
  app.log.info({ signal }, "Shutting down API");
  await app.close();
  closeDatabase(database);
  process.exit(0);
}

process.on("SIGINT", () => void shutdown("SIGINT"));
process.on("SIGTERM", () => void shutdown("SIGTERM"));

try {
  await app.listen({ host: config.host, port: config.apiPort });
} catch (error) {
  app.log.error(error);
  closeDatabase(database);
  process.exit(1);
}

