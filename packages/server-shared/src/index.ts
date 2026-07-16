import { resolve } from "node:path";
import { z } from "zod";

const EnvironmentSchema = z.object({
  NODE_ENV: z.enum(["development", "test", "production"]).default("development"),
  SOLOOPS_HOST: z.string().default("127.0.0.1"),
  SOLOOPS_API_PORT: z.coerce.number().int().min(1).max(65_535).default(3001),
  SOLOOPS_WEB_ORIGIN: z.string().url().default("http://127.0.0.1:5173"),
  SOLOOPS_DATABASE_PATH: z.string().min(1).default("var/db/soloops.db"),
  SOLOOPS_SESSION_TTL_HOURS: z.coerce.number().positive().max(24 * 30).default(24),
  SOLOOPS_EVENT_POLL_MS: z.coerce.number().int().min(100).max(10_000).default(500),
  SOLOOPS_WORKER_POLL_MS: z.coerce.number().int().min(100).max(60_000).default(1000),
  SOLOOPS_LOG_LEVEL: z.enum(["debug", "info", "warn", "error"]).default("info")
});

export type AppConfig = ReturnType<typeof loadConfig>;

export function loadConfig(environment: Record<string, string | undefined> = process.env) {
  const parsed = EnvironmentSchema.parse(environment);
  return {
    environment: parsed.NODE_ENV,
    host: parsed.SOLOOPS_HOST,
    apiPort: parsed.SOLOOPS_API_PORT,
    webOrigin: parsed.SOLOOPS_WEB_ORIGIN,
    databasePath: resolve(process.cwd(), parsed.SOLOOPS_DATABASE_PATH),
    sessionTtlMs: parsed.SOLOOPS_SESSION_TTL_HOURS * 60 * 60 * 1000,
    eventPollMs: parsed.SOLOOPS_EVENT_POLL_MS,
    workerPollMs: parsed.SOLOOPS_WORKER_POLL_MS,
    logLevel: parsed.SOLOOPS_LOG_LEVEL,
    secureCookies: parsed.NODE_ENV === "production"
  } as const;
}

const levels = { debug: 10, info: 20, warn: 30, error: 40 } as const;
export type LogLevel = keyof typeof levels;

export function createLogger(service: string, threshold: LogLevel = "info") {
  function write(level: LogLevel, message: string, context: Record<string, unknown> = {}) {
    if (levels[level] < levels[threshold]) return;
    const line = JSON.stringify({ timestamp: new Date().toISOString(), level, service, message, ...context });
    if (level === "error") console.error(line);
    else if (level === "warn") console.warn(line);
    else console.log(line);
  }
  return {
    debug: (message: string, context?: Record<string, unknown>) => write("debug", message, context),
    info: (message: string, context?: Record<string, unknown>) => write("info", message, context),
    warn: (message: string, context?: Record<string, unknown>) => write("warn", message, context),
    error: (message: string, context?: Record<string, unknown>) => write("error", message, context)
  };
}

export function apiError(code: string, message: string, details?: unknown) {
  return { error: { code, message, ...(details === undefined ? {} : { details }) } };
}

