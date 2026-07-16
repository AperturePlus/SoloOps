import { z } from "zod";

export const runStatuses = [
  "draft",
  "queued",
  "leased",
  "planning",
  "running",
  "waiting_for_approval",
  "paused",
  "verifying",
  "reporting",
  "retry_scheduled",
  "blocked",
  "needs_recovery",
  "succeeded",
  "failed",
  "cancelled"
] as const;

export const RunStatusSchema = z.enum(runStatuses);
export type RunStatus = z.infer<typeof RunStatusSchema>;

const transitions: Readonly<Record<RunStatus, readonly RunStatus[]>> = {
  draft: ["queued", "cancelled"],
  queued: ["leased", "cancelled"],
  leased: ["planning", "needs_recovery", "cancelled"],
  planning: ["running", "blocked", "failed", "cancelled"],
  running: ["waiting_for_approval", "paused", "verifying", "blocked", "failed", "cancelled"],
  waiting_for_approval: ["running", "blocked", "cancelled"],
  paused: ["running", "cancelled"],
  verifying: ["reporting", "running", "blocked", "failed", "cancelled"],
  reporting: ["succeeded", "failed", "blocked"],
  retry_scheduled: ["queued", "cancelled"],
  needs_recovery: ["queued", "blocked", "failed", "cancelled"],
  blocked: [],
  succeeded: [],
  failed: [],
  cancelled: []
};

export function canTransitionRun(from: RunStatus, to: RunStatus): boolean {
  return transitions[from].includes(to);
}

export const LoginRequestSchema = z.object({
  username: z.string().trim().min(1).max(64),
  password: z.string().min(1).max(1024)
});
export type LoginRequest = z.infer<typeof LoginRequestSchema>;

export const OwnerSchema = z.object({
  id: z.string(),
  username: z.string()
});
export type Owner = z.infer<typeof OwnerSchema>;

export const SessionResponseSchema = z.object({ owner: OwnerSchema });
export type SessionResponse = z.infer<typeof SessionResponseSchema>;

export const CreateTaskRequestSchema = z.object({
  title: z.string().trim().min(1).max(160),
  goal: z.string().trim().min(1).max(20_000)
});
export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>;

export const TaskSummarySchema = z.object({
  id: z.string(),
  title: z.string(),
  goal: z.string(),
  status: RunStatusSchema,
  latestRunId: z.string(),
  createdAt: z.number().int()
});
export type TaskSummary = z.infer<typeof TaskSummarySchema>;

export const RunDetailSchema = z.object({
  id: z.string(),
  taskId: z.string(),
  status: RunStatusSchema,
  statusReason: z.string().nullable(),
  createdAt: z.number().int(),
  startedAt: z.number().int().nullable(),
  finishedAt: z.number().int().nullable()
});
export type RunDetail = z.infer<typeof RunDetailSchema>;

export const EventTypeSchema = z.enum([
  "run.created",
  "run.status_changed"
]);
export type EventType = z.infer<typeof EventTypeSchema>;

export const EventEnvelopeSchema = z.object({
  sequence: z.number().int().nonnegative(),
  id: z.string(),
  runId: z.string().nullable(),
  type: EventTypeSchema,
  payload: z.record(z.string(), z.unknown()),
  createdAt: z.number().int()
});
export type EventEnvelope = z.infer<typeof EventEnvelopeSchema>;

export const EventQuerySchema = z.object({
  after: z.coerce.number().int().nonnegative().default(0),
  runId: z.string().optional()
});

export const ApiErrorSchema = z.object({
  error: z.object({
    code: z.string(),
    message: z.string(),
    details: z.unknown().optional()
  })
});
export type ApiError = z.infer<typeof ApiErrorSchema>;

