import { afterEach, describe, expect, test } from "bun:test";
import type { FastifyInstance } from "fastify";
import type { EventEnvelope, TaskSummary } from "@soloops/contracts";
import {
  closeDatabase,
  createOwner,
  getRun,
  openDatabase,
  runMigrations,
  type SoloOpsDatabase
} from "@soloops/db";
import { loadConfig } from "@soloops/server-shared";
import { runOnce } from "../../worker/src/worker.ts";
import { buildApp } from "./app.ts";

type TestContext = {
  app: FastifyInstance;
  database: SoloOpsDatabase;
  webOrigin: string;
};

const contexts: TestContext[] = [];

async function createContext(): Promise<TestContext> {
  const database = openDatabase(":memory:");
  runMigrations(database);
  const webOrigin = "http://127.0.0.1:5173";
  const config = loadConfig({
    NODE_ENV: "test",
    SOLOOPS_DATABASE_PATH: ":memory:",
    SOLOOPS_WEB_ORIGIN: webOrigin,
    SOLOOPS_EVENT_POLL_MS: "100"
  });
  const app = await buildApp({ database, config });
  await app.ready();
  const context = { app, database, webOrigin };
  contexts.push(context);
  return context;
}

async function initializeOwner(database: SoloOpsDatabase) {
  const password = "correct horse battery staple";
  const passwordHash = await Bun.password.hash(password, { algorithm: "argon2id" });
  const owner = createOwner(database, "owner", passwordHash);
  return { owner, password };
}

function cookieFrom(response: { headers: Record<string, string | string[] | number | undefined> }): string {
  const value = response.headers["set-cookie"];
  const first = Array.isArray(value) ? value[0] : value;
  if (!first) throw new Error("Session cookie was not returned");
  return String(first).split(";", 1)[0]!;
}

async function listenUrl(app: FastifyInstance): Promise<string> {
  await app.listen({ host: "127.0.0.1", port: 0 });
  const address = app.server.address();
  if (!address || typeof address === "string") throw new Error("API did not expose a TCP test address");
  return `ws://127.0.0.1:${address.port}`;
}

afterEach(async () => {
  while (contexts.length) {
    const context = contexts.pop()!;
    await context.app.close();
    closeDatabase(context.database);
  }
});

describe("Phase 0 API", () => {
  test("protects routes and supports the full stub task lifecycle", async () => {
    const { app, database, webOrigin } = await createContext();
    const { password } = await initializeOwner(database);

    expect((await app.inject({ method: "GET", url: "/api/tasks" })).statusCode).toBe(401);

    const login = await app.inject({
      method: "POST",
      url: "/api/auth/login",
      headers: { origin: webOrigin },
      payload: { username: "owner", password }
    });
    expect(login.statusCode).toBe(200);
    const cookie = cookieFrom(login);

    const created = await app.inject({
      method: "POST",
      url: "/api/tasks",
      headers: { cookie, origin: webOrigin },
      payload: { title: "Integration", goal: "Exercise the persisted pipeline" }
    });
    expect(created.statusCode).toBe(201);
    const task = created.json<TaskSummary>();
    expect(runOnce(database, "worker-integration")).toBe(task.latestRunId);
    expect(getRun(database, task.latestRunId)?.status).toBe("blocked");

    const events = await app.inject({
      method: "GET",
      url: `/api/events?after=0&runId=${task.latestRunId}`,
      headers: { cookie }
    });
    expect(events.statusCode).toBe(200);
    expect(events.json<{ items: EventEnvelope[] }>().items.map((event) => event.type)).toEqual([
      "run.created",
      "run.status_changed",
      "run.status_changed",
      "run.status_changed"
    ]);

    const socketBaseUrl = await listenUrl(app);
    const WebSocketWithHeaders = WebSocket as unknown as new (
      url: string,
      options: { headers: Record<string, string> }
    ) => WebSocket;
    const socket = new WebSocketWithHeaders(`${socketBaseUrl}/api/events?after=0&runId=${task.latestRunId}`, {
      headers: { cookie }
    });
    const message = await new Promise<string>((resolveMessage, reject) => {
      const timeout = setTimeout(() => reject(new Error("Timed out waiting for a WebSocket event")), 1000);
      socket.addEventListener("message", (event) => {
        clearTimeout(timeout);
        resolveMessage(String(event.data));
      }, { once: true });
    });
    expect(JSON.parse(message).type).toBe("run.created");
    socket.close();

    const logout = await app.inject({
      method: "POST",
      url: "/api/auth/logout",
      headers: { cookie, origin: webOrigin }
    });
    expect(logout.statusCode).toBe(204);
    expect((await app.inject({ method: "GET", url: "/api/auth/session", headers: { cookie } })).statusCode).toBe(401);

    const auditCount = database.sqlite
      .query<{ count: number }, []>("SELECT count(*) AS count FROM audit_logs")
      .get()?.count;
    expect(auditCount).toBe(3);
  });

  test("rate limits repeated login failures", async () => {
    const { app, database, webOrigin } = await createContext();
    await initializeOwner(database);
    const statuses: number[] = [];
    for (let attempt = 0; attempt < 6; attempt += 1) {
      const response = await app.inject({
        method: "POST",
        url: "/api/auth/login",
        headers: { origin: webOrigin },
        payload: { username: "owner", password: "wrong password" }
      });
      statuses.push(response.statusCode);
    }
    expect(statuses.slice(0, 5)).toEqual([401, 401, 401, 401, 401]);
    expect(statuses[5]).toBe(429);
  });

  test("rejects unauthenticated WebSocket subscriptions", async () => {
    const { app } = await createContext();
    const socketBaseUrl = await listenUrl(app);
    const socket = new WebSocket(`${socketBaseUrl}/api/events`);
    const closed = new Promise<number>((resolveClose) => {
      socket.addEventListener("close", (event) => resolveClose(event.code), { once: true });
    });
    expect(await closed).toBe(1008);
  });
});
