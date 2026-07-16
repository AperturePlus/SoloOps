import type { RunStatus } from "@soloops/contracts";
import { index, integer, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";

export const users = sqliteTable(
  "users",
  {
    id: text("id").primaryKey(),
    username: text("username").notNull(),
    passwordHash: text("password_hash").notNull(),
    createdAt: integer("created_at").notNull()
  },
  (table) => [uniqueIndex("users_username_idx").on(table.username)]
);

export const sessions = sqliteTable(
  "sessions",
  {
    id: text("id").primaryKey(),
    tokenHash: text("token_hash").notNull(),
    userId: text("user_id").notNull().references(() => users.id, { onDelete: "cascade" }),
    expiresAt: integer("expires_at").notNull(),
    revokedAt: integer("revoked_at"),
    createdAt: integer("created_at").notNull(),
    lastSeenAt: integer("last_seen_at").notNull()
  },
  (table) => [
    uniqueIndex("sessions_token_hash_idx").on(table.tokenHash),
    index("sessions_user_id_idx").on(table.userId)
  ]
);

export const tasks = sqliteTable("tasks", {
  id: text("id").primaryKey(),
  title: text("title").notNull(),
  goal: text("goal").notNull(),
  createdBy: text("created_by").notNull().references(() => users.id),
  createdAt: integer("created_at").notNull(),
  updatedAt: integer("updated_at").notNull()
});

export const runs = sqliteTable(
  "runs",
  {
    id: text("id").primaryKey(),
    taskId: text("task_id").notNull().references(() => tasks.id, { onDelete: "cascade" }),
    status: text("status").$type<RunStatus>().notNull(),
    statusReason: text("status_reason"),
    leaseOwner: text("lease_owner"),
    leaseExpiresAt: integer("lease_expires_at"),
    createdAt: integer("created_at").notNull(),
    startedAt: integer("started_at"),
    finishedAt: integer("finished_at")
  },
  (table) => [
    index("runs_task_id_idx").on(table.taskId),
    index("runs_status_created_at_idx").on(table.status, table.createdAt)
  ]
);

export const events = sqliteTable(
  "events",
  {
    sequence: integer("sequence").primaryKey({ autoIncrement: true }),
    id: text("id").notNull(),
    runId: text("run_id").references(() => runs.id, { onDelete: "cascade" }),
    type: text("type").notNull(),
    payload: text("payload").notNull(),
    createdAt: integer("created_at").notNull()
  },
  (table) => [
    uniqueIndex("events_id_idx").on(table.id),
    index("events_run_sequence_idx").on(table.runId, table.sequence)
  ]
);

export const auditLogs = sqliteTable(
  "audit_logs",
  {
    sequence: integer("sequence").primaryKey({ autoIncrement: true }),
    id: text("id").notNull(),
    actorType: text("actor_type").notNull(),
    actorId: text("actor_id"),
    action: text("action").notNull(),
    objectType: text("object_type"),
    objectId: text("object_id"),
    outcome: text("outcome").notNull(),
    context: text("context").notNull(),
    createdAt: integer("created_at").notNull()
  },
  (table) => [
    uniqueIndex("audit_logs_id_idx").on(table.id),
    index("audit_action_created_at_idx").on(table.action, table.createdAt)
  ]
);

