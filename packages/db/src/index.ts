import { readFileSync, mkdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, resolve } from "node:path";
import { Database } from "bun:sqlite";
import type {
  CreateTaskRequest,
  EventEnvelope,
  EventType,
  Owner,
  RunDetail,
  RunStatus,
  TaskSummary
} from "@soloops/contracts";
import { canTransitionRun, EventEnvelopeSchema, RunStatusSchema } from "@soloops/contracts";
import { count, eq } from "drizzle-orm";
import { drizzle, type BunSQLiteDatabase } from "drizzle-orm/bun-sqlite";
import { migrate } from "drizzle-orm/bun-sqlite/migrator";
import * as schema from "./schema.ts";

export * from "./schema.ts";

export type SoloOpsDatabase = {
  sqlite: Database;
  orm: BunSQLiteDatabase<typeof schema>;
};

export const defaultDatabasePath = resolve(process.cwd(), "var/db/soloops.db");

export function openDatabase(path = defaultDatabasePath): SoloOpsDatabase {
  if (path !== ":memory:") mkdirSync(dirname(path), { recursive: true });
  const sqlite = new Database(path, { create: true, strict: true });
  sqlite.exec("PRAGMA foreign_keys = ON;");
  sqlite.exec("PRAGMA busy_timeout = 5000;");
  if (path !== ":memory:") sqlite.exec("PRAGMA journal_mode = WAL;");
  sqlite.exec("PRAGMA synchronous = NORMAL;");
  return { sqlite, orm: drizzle(sqlite, { schema }) };
}

export function closeDatabase(database: SoloOpsDatabase): void {
  database.sqlite.close();
}

export function runMigrations(database: SoloOpsDatabase): void {
  const migrationDir = resolve(import.meta.dir, "../migrations");
  const hasLegacyTable = database.sqlite
    .query<{ found: number }, []>(
      "SELECT 1 AS found FROM sqlite_master WHERE type = 'table' AND name = '_soloops_migrations'"
    )
    .get();
  const hasDrizzleTable = database.sqlite
    .query<{ found: number }, []>(
      "SELECT 1 AS found FROM sqlite_master WHERE type = 'table' AND name = '__drizzle_migrations'"
    )
    .get();
  if (hasLegacyTable && !hasDrizzleTable) {
    const migration = readFileSync(resolve(migrationDir, "0001_phase_zero.sql"), "utf8");
    const hash = createHash("sha256").update(migration).digest("hex");
    const adopt = database.sqlite.transaction(() => {
      database.sqlite.exec(`CREATE TABLE __drizzle_migrations (
        id SERIAL PRIMARY KEY,
        hash TEXT NOT NULL,
        created_at NUMERIC
      )`);
      database.sqlite
        .query("INSERT INTO __drizzle_migrations (hash, created_at) VALUES ($hash, $createdAt)")
        .run({ hash, createdAt: 1784044800000 });
    });
    adopt.immediate();
  }
  migrate(database.orm, { migrationsFolder: migrationDir });
}

type UserRow = { id: string; username: string; password_hash: string; created_at: number };
type SessionUserRow = UserRow & { session_id: string; expires_at: number; revoked_at: number | null };

export function countUsers(database: SoloOpsDatabase): number {
  return database.orm.select({ value: count() }).from(schema.users).get()?.value ?? 0;
}

export function createOwner(database: SoloOpsDatabase, username: string, passwordHash: string): Owner {
  const normalized = username.trim().toLowerCase();
  if (!normalized) throw new Error("Owner username is required");
  if (countUsers(database) > 0) throw new Error("Owner already exists");
  const owner = { id: crypto.randomUUID(), username: normalized };
  database.orm.insert(schema.users).values({
    id: owner.id,
    username: owner.username,
    passwordHash,
    createdAt: Date.now()
  }).run();
  return owner;
}

export function findUserByUsername(database: SoloOpsDatabase, username: string): UserRow | null {
  const row = database.orm
    .select()
    .from(schema.users)
    .where(eq(schema.users.username, username.trim().toLowerCase()))
    .get();
  return row
    ? { id: row.id, username: row.username, password_hash: row.passwordHash, created_at: row.createdAt }
    : null;
}

export function createSession(
  database: SoloOpsDatabase,
  input: { userId: string; tokenHash: string; expiresAt: number }
): string {
  const id = crypto.randomUUID();
  const now = Date.now();
  database.orm.insert(schema.sessions).values({
    id,
    tokenHash: input.tokenHash,
    userId: input.userId,
    expiresAt: input.expiresAt,
    revokedAt: null,
    createdAt: now,
    lastSeenAt: now
  }).run();
  return id;
}

export function findSessionOwner(database: SoloOpsDatabase, tokenHash: string, now = Date.now()): (Owner & { sessionId: string }) | null {
  const row = database.sqlite
    .query<SessionUserRow, { tokenHash: string; now: number }>(`
      SELECT users.*, sessions.id AS session_id, sessions.expires_at, sessions.revoked_at
      FROM sessions JOIN users ON users.id = sessions.user_id
      WHERE sessions.token_hash = $tokenHash
        AND sessions.revoked_at IS NULL
        AND sessions.expires_at > $now
    `)
    .get({ tokenHash, now });
  if (!row) return null;
  database.sqlite
    .query("UPDATE sessions SET last_seen_at = $now WHERE id = $id")
    .run({ now, id: row.session_id });
  return { id: row.id, username: row.username, sessionId: row.session_id };
}

export function revokeSession(database: SoloOpsDatabase, sessionId: string, now = Date.now()): void {
  database.orm
    .update(schema.sessions)
    .set({ revokedAt: now })
    .where(eq(schema.sessions.id, sessionId))
    .run();
}

function insertEvent(
  database: SoloOpsDatabase,
  runId: string,
  type: EventType,
  payload: Record<string, unknown>,
  now: number
): void {
  database.sqlite
    .query("INSERT INTO events (id, run_id, type, payload, created_at) VALUES ($id, $runId, $type, $payload, $createdAt)")
    .run({
      id: crypto.randomUUID(),
      runId,
      type,
      payload: JSON.stringify(payload),
      createdAt: now
    });
}

export function createTaskWithRun(database: SoloOpsDatabase, ownerId: string, input: CreateTaskRequest): TaskSummary {
  const taskId = crypto.randomUUID();
  const runId = crypto.randomUUID();
  const now = Date.now();
  const create = database.sqlite.transaction(() => {
    database.sqlite
      .query(`INSERT INTO tasks (id, title, goal, created_by, created_at, updated_at)
              VALUES ($id, $title, $goal, $createdBy, $createdAt, $updatedAt)`)
      .run({ id: taskId, title: input.title, goal: input.goal, createdBy: ownerId, createdAt: now, updatedAt: now });
    database.sqlite
      .query(`INSERT INTO runs (id, task_id, status, status_reason, lease_owner, lease_expires_at, created_at, started_at, finished_at)
              VALUES ($id, $taskId, 'queued', NULL, NULL, NULL, $createdAt, NULL, NULL)`)
      .run({ id: runId, taskId, createdAt: now });
    insertEvent(database, runId, "run.created", { taskId, status: "queued" }, now);
  });
  create.immediate();
  return { id: taskId, title: input.title, goal: input.goal, status: "queued", latestRunId: runId, createdAt: now };
}

type TaskRow = {
  id: string;
  title: string;
  goal: string;
  status: string;
  latest_run_id: string;
  created_at: number;
};

const latestTaskSql = `
  SELECT tasks.id, tasks.title, tasks.goal, runs.status, runs.id AS latest_run_id, tasks.created_at
  FROM tasks
  JOIN runs ON runs.id = (
    SELECT r.id FROM runs r WHERE r.task_id = tasks.id ORDER BY r.created_at DESC, r.id DESC LIMIT 1
  )
`;

function taskFromRow(row: TaskRow): TaskSummary {
  return {
    id: row.id,
    title: row.title,
    goal: row.goal,
    status: RunStatusSchema.parse(row.status),
    latestRunId: row.latest_run_id,
    createdAt: row.created_at
  };
}

export function listTasks(database: SoloOpsDatabase): TaskSummary[] {
  return database.sqlite.query<TaskRow, []>(`${latestTaskSql} ORDER BY tasks.created_at DESC`).all().map(taskFromRow);
}

export function getTask(database: SoloOpsDatabase, taskId: string): TaskSummary | null {
  const row = database.sqlite
    .query<TaskRow, { taskId: string }>(`${latestTaskSql} WHERE tasks.id = $taskId`)
    .get({ taskId });
  return row ? taskFromRow(row) : null;
}

type RunRow = {
  id: string;
  task_id: string;
  status: string;
  status_reason: string | null;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
};

function runFromRow(row: RunRow): RunDetail {
  return {
    id: row.id,
    taskId: row.task_id,
    status: RunStatusSchema.parse(row.status),
    statusReason: row.status_reason,
    createdAt: row.created_at,
    startedAt: row.started_at,
    finishedAt: row.finished_at
  };
}

export function getRun(database: SoloOpsDatabase, runId: string): RunDetail | null {
  const row = database.sqlite.query<RunRow, { runId: string }>("SELECT * FROM runs WHERE id = $runId").get({ runId });
  return row ? runFromRow(row) : null;
}

export function claimNextRun(database: SoloOpsDatabase, workerId: string, leaseMs = 30_000): RunDetail | null {
  const claim = database.sqlite.transaction(() => {
    const row = database.sqlite
      .query<RunRow, []>("SELECT * FROM runs WHERE status = 'queued' ORDER BY created_at ASC LIMIT 1")
      .get();
    if (!row) return null;
    const now = Date.now();
    database.sqlite
      .query(`UPDATE runs SET status = 'leased', lease_owner = $workerId, lease_expires_at = $leaseExpiresAt,
              started_at = COALESCE(started_at, $now)
              WHERE id = $runId AND status = 'queued'`)
      .run({ workerId, leaseExpiresAt: now + leaseMs, now, runId: row.id });
    insertEvent(database, row.id, "run.status_changed", { from: "queued", to: "leased", workerId }, now);
    return getRun(database, row.id);
  });
  return claim.immediate();
}

export function transitionRun(database: SoloOpsDatabase, runId: string, to: RunStatus, reason: string | null = null): RunDetail {
  const change = database.sqlite.transaction(() => {
    const current = getRun(database, runId);
    if (!current) throw new Error(`Run not found: ${runId}`);
    if (!canTransitionRun(current.status, to)) throw new Error(`Invalid run transition: ${current.status} -> ${to}`);
    const now = Date.now();
    const finishedAt = ["blocked", "succeeded", "failed", "cancelled"].includes(to) ? now : null;
    database.sqlite
      .query(`UPDATE runs SET status = $status, status_reason = $reason,
              lease_owner = CASE WHEN $finishedAt IS NULL THEN lease_owner ELSE NULL END,
              lease_expires_at = CASE WHEN $finishedAt IS NULL THEN lease_expires_at ELSE NULL END,
              finished_at = $finishedAt WHERE id = $runId`)
      .run({ status: to, reason, finishedAt, runId });
    insertEvent(database, runId, "run.status_changed", { from: current.status, to, reason }, now);
    const updated = getRun(database, runId);
    if (!updated) throw new Error(`Run disappeared during transition: ${runId}`);
    return updated;
  });
  return change.immediate();
}

type EventRow = {
  sequence: number;
  id: string;
  run_id: string | null;
  type: string;
  payload: string;
  created_at: number;
};

export function listEvents(database: SoloOpsDatabase, after = 0, runId?: string, limit = 200): EventEnvelope[] {
  const boundedLimit = Math.max(1, Math.min(limit, 500));
  const rows = runId
    ? database.sqlite
        .query<EventRow, { after: number; runId: string; limit: number }>(
          "SELECT * FROM events WHERE sequence > $after AND run_id = $runId ORDER BY sequence ASC LIMIT $limit"
        )
        .all({ after, runId, limit: boundedLimit })
    : database.sqlite
        .query<EventRow, { after: number; limit: number }>(
          "SELECT * FROM events WHERE sequence > $after ORDER BY sequence ASC LIMIT $limit"
        )
        .all({ after, limit: boundedLimit });
  return rows.map((row) => EventEnvelopeSchema.parse({
    sequence: row.sequence,
    id: row.id,
    runId: row.run_id,
    type: row.type,
    payload: JSON.parse(row.payload),
    createdAt: row.created_at
  }));
}

export function writeAudit(
  database: SoloOpsDatabase,
  entry: {
    actorType: "owner" | "anonymous" | "system";
    actorId?: string;
    action: string;
    objectType?: string;
    objectId?: string;
    outcome: "success" | "failure";
    context?: Record<string, unknown>;
  }
): void {
  database.sqlite
    .query(`INSERT INTO audit_logs
      (id, actor_type, actor_id, action, object_type, object_id, outcome, context, created_at)
      VALUES ($id, $actorType, $actorId, $action, $objectType, $objectId, $outcome, $context, $createdAt)`)
    .run({
      id: crypto.randomUUID(),
      actorType: entry.actorType,
      actorId: entry.actorId ?? null,
      action: entry.action,
      objectType: entry.objectType ?? null,
      objectId: entry.objectId ?? null,
      outcome: entry.outcome,
      context: JSON.stringify(entry.context ?? {}),
      createdAt: Date.now()
    });
}
