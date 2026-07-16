import { closeDatabase, openDatabase, runMigrations } from "./index.ts";

const database = openDatabase(process.env.SOLOOPS_DATABASE_PATH);
try {
  runMigrations(database);
  console.log(JSON.stringify({ level: "info", message: "Database migrations applied" }));
} finally {
  closeDatabase(database);
}

