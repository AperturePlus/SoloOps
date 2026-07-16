import cookie from "@fastify/cookie";
import rateLimit from "@fastify/rate-limit";
import websocket from "@fastify/websocket";
import Fastify, { type FastifyInstance } from "fastify";
import { ZodError } from "zod";
import {
  CreateTaskRequestSchema,
  EventQuerySchema,
  LoginRequestSchema,
  type EventEnvelope,
  type Owner
} from "@soloops/contracts";
import {
  createSession,
  createTaskWithRun,
  findUserByUsername,
  getRun,
  getTask,
  listEvents,
  listTasks,
  revokeSession,
  writeAudit,
  type SoloOpsDatabase
} from "@soloops/db";
import { apiError, type AppConfig } from "@soloops/server-shared";
import {
  generateSessionToken,
  hashSessionToken,
  requireOwner,
  SESSION_COOKIE,
  sessionOwner
} from "./auth.ts";

type EventSocket = {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  on(event: "close" | "error", listener: () => void): void;
};

type EventClient = { socket: EventSocket; after: number; runId?: string };

export type AppOptions = {
  database: SoloOpsDatabase;
  config: AppConfig;
};

export async function buildApp({ database, config }: AppOptions): Promise<FastifyInstance> {
  const app = Fastify({ logger: config.environment !== "test" ? { level: config.logLevel } : false });
  const clients = new Set<EventClient>();

  await app.register(cookie);
  await app.register(rateLimit, { global: false });
  await app.register(websocket);

  app.setErrorHandler(async (error, _request, reply) => {
    if (error instanceof ZodError) {
      await reply.code(400).send(apiError("invalid_request", "Request validation failed", error.issues));
      return;
    }
    if ((error as { code?: string }).code === "SQLITE_CONSTRAINT_UNIQUE") {
      await reply.code(409).send(apiError("conflict", "The requested resource already exists"));
      return;
    }
    const statusCode = (error as { statusCode?: number }).statusCode;
    if (statusCode && statusCode >= 400 && statusCode < 500) {
      await reply
        .code(statusCode)
        .send(apiError(
          (error as { code?: string }).code ?? "request_rejected",
          error instanceof Error ? error.message : "Request was rejected"
        ));
      return;
    }
    app.log.error(error);
    await reply.code(500).send(apiError("internal_error", "Unexpected server error"));
  });

  app.addHook("onRequest", async (request, reply) => {
    if (["GET", "HEAD", "OPTIONS"].includes(request.method)) return;
    const origin = request.headers.origin;
    if (origin && origin !== config.webOrigin) {
      await reply.code(403).send(apiError("invalid_origin", "Request origin is not allowed"));
    }
  });

  app.get("/healthz", async () => ({ status: "ok", service: "api" }));

  app.post(
    "/api/auth/login",
    { config: { rateLimit: { max: 5, timeWindow: "1 minute" } } },
    async (request, reply) => {
      const input = LoginRequestSchema.parse(request.body);
      const user = findUserByUsername(database, input.username);
      const valid = user ? await Bun.password.verify(input.password, user.password_hash) : false;
      if (!user || !valid) {
        writeAudit(database, {
          actorType: "anonymous",
          action: "auth.login",
          outcome: "failure",
          context: { username: input.username.trim().toLowerCase(), remoteAddress: request.ip }
        });
        return reply.code(401).send(apiError("invalid_credentials", "Invalid username or password"));
      }

      const token = generateSessionToken();
      const expiresAt = Date.now() + config.sessionTtlMs;
      createSession(database, { userId: user.id, tokenHash: hashSessionToken(token), expiresAt });
      reply.setCookie(SESSION_COOKIE, token, {
        path: "/",
        httpOnly: true,
        sameSite: "strict",
        secure: config.secureCookies,
        expires: new Date(expiresAt)
      });
      writeAudit(database, {
        actorType: "owner",
        actorId: user.id,
        action: "auth.login",
        objectType: "session",
        outcome: "success",
        context: { remoteAddress: request.ip }
      });
      const owner: Owner = { id: user.id, username: user.username };
      return { owner };
    }
  );

  app.get("/api/auth/session", async (request, reply) => {
    const owner = await requireOwner(database, request, reply);
    if (!owner) return;
    return { owner: { id: owner.id, username: owner.username } };
  });

  app.post("/api/auth/logout", async (request, reply) => {
    const owner = await requireOwner(database, request, reply);
    if (!owner) return;
    revokeSession(database, owner.sessionId);
    reply.clearCookie(SESSION_COOKIE, { path: "/" });
    writeAudit(database, {
      actorType: "owner",
      actorId: owner.id,
      action: "auth.logout",
      objectType: "session",
      objectId: owner.sessionId,
      outcome: "success"
    });
    return reply.code(204).send();
  });

  app.post("/api/tasks", async (request, reply) => {
    const owner = await requireOwner(database, request, reply);
    if (!owner) return;
    const input = CreateTaskRequestSchema.parse(request.body);
    const task = createTaskWithRun(database, owner.id, input);
    writeAudit(database, {
      actorType: "owner",
      actorId: owner.id,
      action: "task.create",
      objectType: "task",
      objectId: task.id,
      outcome: "success",
      context: { runId: task.latestRunId }
    });
    return reply.code(201).send(task);
  });

  app.get("/api/tasks", async (request, reply) => {
    if (!(await requireOwner(database, request, reply))) return;
    return { items: listTasks(database) };
  });

  app.get<{ Params: { taskId: string } }>("/api/tasks/:taskId", async (request, reply) => {
    if (!(await requireOwner(database, request, reply))) return;
    const task = getTask(database, request.params.taskId);
    if (!task) return reply.code(404).send(apiError("not_found", "Task was not found"));
    return task;
  });

  app.get<{ Params: { runId: string } }>("/api/runs/:runId", async (request, reply) => {
    if (!(await requireOwner(database, request, reply))) return;
    const run = getRun(database, request.params.runId);
    if (!run) return reply.code(404).send(apiError("not_found", "Run was not found"));
    return run;
  });

  async function sendPending(client: EventClient): Promise<void> {
    if (client.socket.readyState !== 1) return;
    const events: EventEnvelope[] = listEvents(database, client.after, client.runId);
    for (const event of events) {
      client.socket.send(JSON.stringify(event));
      client.after = event.sequence;
    }
  }

  app.route({
    method: "GET",
    url: "/api/events",
    handler: async (request, reply) => {
      if (!(await requireOwner(database, request, reply))) return;
      const query = EventQuerySchema.parse(request.query);
      return { items: listEvents(database, query.after, query.runId) };
    },
    wsHandler: (rawSocket, request) => {
      const socket = rawSocket as unknown as EventSocket;
      if (!sessionOwner(database, request)) {
        socket.close(1008, "Owner session is required");
        return;
      }
      let query: { after: number; runId?: string };
      try {
        query = EventQuerySchema.parse(request.query);
      } catch {
        socket.close(1008, "Invalid event cursor");
        return;
      }
      const client: EventClient = { socket, after: query.after, ...(query.runId ? { runId: query.runId } : {}) };
      clients.add(client);
      void sendPending(client);
      const remove = () => clients.delete(client);
      socket.on("close", remove);
      socket.on("error", remove);
    }
  });

  const timer = setInterval(() => {
    for (const client of clients) void sendPending(client);
  }, config.eventPollMs);
  timer.unref();

  app.addHook("onClose", async () => {
    clearInterval(timer);
    for (const client of clients) client.socket.close(1001, "Server is shutting down");
    clients.clear();
  });

  return app;
}
