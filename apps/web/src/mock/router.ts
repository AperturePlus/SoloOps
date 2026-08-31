import type { IncomingMessage, ServerResponse } from "node:http";
import type { MockState } from "./state";
import type { ScenarioDeps } from "./scenario";
import { createTask, getTask, listTasks, getRun, getRuntime, listEvents } from "./state";
import { startScenario, approveToolCall, denyToolCall, cancelScenario } from "./scenario";
import type { TaskSummary } from "$lib/contracts";

export interface MockRouterDeps {
  state: MockState;
  scenario: ScenarioDeps;
}

function rid(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 10)}`;
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.statusCode = status;
  res.setHeader("content-type", "application/json");
  res.setHeader("content-length", Buffer.byteLength(payload));
  res.end(payload);
}

function sendError(res: ServerResponse, status: number, code: string, message: string): void {
  sendJson(res, status, { error: { code, message } });
}

function send204(res: ServerResponse): void {
  res.statusCode = 204;
  res.end();
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve) => {
    let data = "";
    req.on("data", (chunk) => (data += chunk));
    req.on("end", () => resolve(data));
    req.on("error", () => resolve(""));
  });
}

function hasSession(req: IncomingMessage): boolean {
  const cookie = req.headers.cookie ?? "";
  return /soloops_session=mock-/.test(cookie);
}

function splitPath(url: string): string[] {
  const path = url.split("?")[0];
  return path.split("/").filter(Boolean);
}

export function createMockRouter(deps: MockRouterDeps) {
  return async function route(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    const segments = splitPath(req.url ?? "");
    if (
      segments[0] !== "api" &&
      !["healthz", "livez", "readyz", "metrics"].includes(segments[0] ?? "")
    ) {
      return false;
    }

    const method = req.method ?? "GET";

    // health
    if (segments[0] === "healthz" || segments[0] === "livez") {
      sendJson(res, 200, { status: "ok", service: "soloops-mock" });
      return true;
    }
    if (segments[0] === "readyz") {
      sendJson(res, 200, { status: "ready" });
      return true;
    }
    if (segments[0] === "metrics") {
      res.statusCode = 200;
      res.setHeader("content-type", "text/plain");
      res.end("# soloops mock metrics\n");
      return true;
    }

    // /api/auth/*
    if (segments[1] === "auth") {
      if (segments[2] === "login" && method === "POST") {
        deps.state.owner = { id: rid("owner"), username: "owner" };
        res.setHeader("Set-Cookie", `soloops_session=mock-${rid("sess")}; HttpOnly; Path=/`);
        sendJson(res, 200, { owner: deps.state.owner });
        return true;
      }
      if (segments[2] === "session" && method === "GET") {
        if (!hasSession(req))
          return (sendError(res, 401, "unauthorized", "Session required"), true);
        sendJson(res, 200, { owner: deps.state.owner ?? { id: "owner", username: "owner" } });
        return true;
      }
      if (segments[2] === "logout" && method === "POST") {
        deps.state.owner = null;
        send204(res);
        return true;
      }
    }

    if (!hasSession(req)) return (sendError(res, 401, "unauthorized", "Session required"), true);

    // /api/settings/ip-notifications
    if (segments[1] === "settings" && segments[2] === "ip-notifications") {
      if (method === "GET" && segments.length === 3) {
        sendJson(res, 200, deps.state.ipNotifications);
        return true;
      }
      if (method === "PUT" && segments.length === 3) {
        const body = JSON.parse((await readBody(req)) || "{}");
        const rawRecipients: string[] = (Array.isArray(body.recipients) ? body.recipients : [])
          .map((value: unknown) => String(value).trim().toLowerCase())
          .filter(Boolean);
        const recipients = [...new Set<string>(rawRecipients)].sort();
        const previous = new Map(
          deps.state.ipNotifications.recipients.map((item) => [item.email, item])
        );
        deps.state.ipNotifications = {
          ...deps.state.ipNotifications,
          enabled: Boolean(body.enabled),
          recipients: recipients.map(
            (email) =>
              previous.get(email) ?? {
                email,
                lastNotifiedIpv4: null,
                lastNotifiedAt: null,
                lastAttemptAt: null,
                lastError: null
              }
          )
        };
        sendJson(res, 200, deps.state.ipNotifications);
        return true;
      }
      if (method === "POST" && segments.length === 4 && segments[3] === "test") {
        sendJson(res, 200, {
          sentCount: deps.state.ipNotifications.recipients.length,
          failedRecipients: []
        });
        return true;
      }
    }

    // /api/tasks
    if (segments[1] === "tasks") {
      if (method === "GET" && segments.length === 2) {
        sendJson(res, 200, { items: listTasks(deps.state) });
        return true;
      }
      if (method === "POST" && segments.length === 2) {
        const body = JSON.parse((await readBody(req)) || "{}");
        const task: TaskSummary = createTask(
          deps.state,
          String(body.title ?? ""),
          String(body.goal ?? "")
        );
        startScenario(deps.scenario, task.latestRunId);
        res.statusCode = 201;
        sendJson(res, 201, task);
        return true;
      }
      if (method === "GET" && segments.length === 3) {
        const task = getTask(deps.state, segments[2]);
        if (!task) return (sendError(res, 404, "not_found", "Task not found"), true);
        sendJson(res, 200, task);
        return true;
      }
    }

    // /api/runs/*
    if (segments[1] === "runs" && segments.length >= 3) {
      const runId = segments[2];
      if (method === "GET" && segments.length === 3) {
        const run = getRun(deps.state, runId);
        if (!run) return (sendError(res, 404, "not_found", "Run not found"), true);
        sendJson(res, 200, run);
        return true;
      }
      if (method === "GET" && segments.length === 4 && segments[3] === "runtime") {
        const runtime = getRuntime(deps.state, runId);
        if (!runtime) return (sendError(res, 404, "not_found", "Runtime not found"), true);
        sendJson(res, 200, runtime);
        return true;
      }
      if (method === "POST" && segments.length === 4 && segments[3] === "cancel") {
        cancelScenario(deps.scenario, runId);
        const run = getRun(deps.state, runId);
        if (!run) return (sendError(res, 404, "not_found", "Run not found"), true);
        sendJson(res, 200, run);
        return true;
      }
      // POST /api/runs/:runId/tool-calls/:callId/decision
      // 路径段 ["api","runs",runId,"tool-calls",callId,"decision"] 共 6 段
      if (
        method === "POST" &&
        segments.length === 6 &&
        segments[3] === "tool-calls" &&
        segments[5] === "decision"
      ) {
        const callId = decodeURIComponent(segments[4]);
        const body = JSON.parse((await readBody(req)) || "{}");
        const decision = String(body.decision ?? "");
        if (decision === "approve") approveToolCall(deps.scenario, runId, callId);
        else if (decision === "deny") denyToolCall(deps.scenario, runId, callId);
        else
          return (sendError(res, 400, "invalid_request", "decision must be approve or deny"), true);
        const runtime = getRuntime(deps.state, runId);
        const call = runtime?.toolCalls.find((c) => c.callId === callId);
        if (!call) return (sendError(res, 404, "not_found", "Tool call not found"), true);
        sendJson(res, 200, call);
        return true;
      }
    }

    // /api/events
    if (segments[1] === "events" && method === "GET") {
      const url = new URL(req.url ?? "", "http://mock");
      const after = Number(url.searchParams.get("after") ?? "0");
      const runId = url.searchParams.get("runId") ?? undefined;
      sendJson(res, 200, { items: listEvents(deps.state, after, runId) });
      return true;
    }

    return (sendError(res, 404, "not_found", "Not found"), true);
  };
}
