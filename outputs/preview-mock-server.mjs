// SoloOps static-preview mock server.
// Serves apps/web/build (the static SPA) and fakes every /api route with a
// living agent-run scenario, including WebSocket event streaming, so the
// Codex-style composite layouts can be experienced end-to-end without the
// Rust backend.
//
// Usage:  bun outputs/preview-mock-server.mjs [port]
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import wsPkg from "../apps/web/node_modules/ws/index.js";
const { WebSocketServer } = wsPkg;

const PORT = Number(process.argv[2] || 5199);
const ROOT = join(process.cwd(), "apps", "web", "build");

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2"
};

// ---------------------------------------------------------------------------
// Fake state + scenario
// ---------------------------------------------------------------------------
const now = () => Date.now();
let seq = 0;
const rid = (p) => `${p}_${Math.random().toString(36).slice(2, 10)}`;

const state = {
  owner: null,
  tasks: [],
  events: [],
  runtimes: new Map(),
  scenarios: new Map()
};

const emit = (type, payload, runId) => {
  const envelope = { sequence: ++seq, id: rid("evt"), runId, type, payload, createdAt: now() };
  state.events.push(envelope);
  return envelope;
};

function makeTask(title, goal, status, minutesAgo) {
  const runId = rid("run");
  const taskId = rid("task");
  const createdAt = now() - minutesAgo * 60_000;
  const task = { id: taskId, title, goal, status, latestRunId: runId, createdAt };
  state.tasks.push(task);
  state.runtimes.set(runId, {
    runId,
    checkpoint: "preparing",
    plan: { summary: "", steps: [] },
    budget: {
      maxModelTurns: 8,
      maxToolCalls: 6,
      maxInputTokens: 4000,
      maxOutputTokens: 2000,
      maxDurationMs: 60000,
      maxToolDurationMs: 10000,
      maxToolOutputBytes: 10000,
      maxWorkspaceBytes: 100000
    },
    usage: {
      modelTurns: 0,
      toolCalls: 0,
      inputTokens: 0,
      outputTokens: 0,
      cachedInputTokens: 0,
      cacheWriteInputTokens: 0,
      elapsedMs: 0
    },
    workspaceRevision: 0,
    toolCalls: [],
    evidence: [],
    report: null
  });
  state.scenarios.set(runId, { step: 0, timer: null, finished: false });
  return { task, runId };
}

// Seed a couple of historic tasks so the workspace list has texture.
makeTask(
  "Nightly backup verification",
  "Run the backup verification job and report any corrupted archives.",
  "failed",
  320
);
makeTask(
  "Extract contact data for the newsletter list",
  "Parse the exported CSV, dedupe by email, and produce a clean CSV with headers.",
  "succeeded",
  160
);

function runDetail(runId) {
  return {
    id: runId,
    taskId: "task",
    status: state.scenarios.get(runId)?.status ?? "queued",
    statusReason: state.scenarios.get(runId)?.statusReason ?? null,
    createdAt: now() - 40_000,
    startedAt: now() - 38_000,
    finishedAt: ["succeeded", "failed", "cancelled"].includes(
      state.scenarios.get(runId)?.status ?? ""
    )
      ? now()
      : null
  };
}

function schedule(runId, fn, ms) {
  const handle = state.scenarios.get(runId);
  if (handle?.finished) return;
  handle.timer = setTimeout(() => {
    handle.timer = null;
    if (!handle.finished) fn();
  }, ms);
}

function setStatus(runId, status, reason) {
  const handle = state.scenarios.get(runId);
  handle.status = status;
  if (reason !== undefined) handle.statusReason = reason;
  const task = state.tasks.find((t) => t.latestRunId === runId);
  if (task) task.status = status;
  emit("run.status_changed", { from: handle.prevStatus ?? "queued", to: status, reason }, runId);
  handle.prevStatus = status;
  if (["succeeded", "failed", "cancelled"].includes(status)) handle.finished = true;
}

function runScenario(runId) {
  const runtime = state.runtimes.get(runId);
  const handle = state.scenarios.get(runId);

  const step0 = () => {
    if (handle.finished) return;
    setStatus(runId, "planning");
    runtime.checkpoint = "calling_model";
    runtime.plan = {
      summary: "Inspect the compose configuration, apply the change, then verify health.",
      steps: [
        { id: "s1", title: "Inspect workspace", status: "completed", required: true },
        { id: "s2", title: "Apply change", status: "in_progress", required: true },
        { id: "s3", title: "Verify result", status: "pending", required: true },
        { id: "s4", title: "Write final report", status: "pending", required: false }
      ]
    };
    runtime.usage.modelTurns = 1;
    runtime.usage.inputTokens = 640;
    runtime.usage.outputTokens = 180;
    emit("agent.plan_updated", { plan: runtime.plan }, runId);
    emit("agent.message", { preview: "Proposing a 4-step plan, health checks gated on evidence." }, runId);
    schedule(runId, step1, 1600);
  };

  const step1 = () => {
    if (handle.finished) return;
    const call = {
      callId: rid("call"),
      name: "workspace.read",
      argumentsSha256: "0".repeat(64),
      approvalPreview: null,
      risk: "read_only",
      policy: "allow",
      status: "completed",
      resultSummary: "Read docker-compose.staging.yml (2.4 kb) — 3 services defined.",
      errorCategory: null,
      startedAt: now() - 900,
      completedAt: now() - 100
    };
    runtime.toolCalls.push(call);
    setStatus(runId, "running");
    runtime.checkpoint = "executing_tools";
    runtime.usage.toolCalls = 1;
    runtime.usage.outputTokens = 340;
    runtime.workspaceRevision = 1;
    emit("tool.call_started", { callId: call.callId, name: call.name }, runId);
    emit("tool.call_completed", { callId: call.callId, resultSummary: call.resultSummary }, runId);
    runtime.evidence.push({
      id: rid("ev"),
      toolCallId: call.callId,
      kind: "file.read",
      summary: "Compose config present with 3 services (web, api, worker).",
      artifactRef: "workspace/docker-compose.staging.yml",
      contentSha256: "0".repeat(64),
      workspaceRevision: 1,
      createdAt: now() - 100
    });
    runtime.evidence.push({
      id: rid("ev"),
      toolCallId: call.callId,
      kind: "file.stat",
      summary: "Workspace 12.4 kb across 5 files, nothing checked out of bounds.",
      artifactRef: null,
      contentSha256: null,
      workspaceRevision: 1,
      createdAt: now() - 80
    });
    schedule(runId, step2, 1800);
  };

  const step2 = () => {
    if (handle.finished) return;
    const call = {
      callId: rid("call"),
      name: "workspace.replace",
      argumentsSha256: "1".repeat(64),
      approvalPreview: {
        action: "replace",
        path: "result.txt",
        summary: "Replace one workspace artifact with the verification output"
      },
      risk: "workspace_write",
      policy: "require_approval",
      status: "waiting_for_approval",
      resultSummary: null,
      errorCategory: null,
      startedAt: now() - 200,
      completedAt: null
    };
    runtime.toolCalls.push(call);
    setStatus(runId, "waiting_for_approval");
    runtime.checkpoint = "waiting_for_approval";
    emit("tool.call_started", { callId: call.callId, name: call.name }, runId);
    // Pause here until the owner decides.
  };

  const step3 = () => {
    if (handle.finished) return;
    setStatus(runId, "verifying");
    runtime.checkpoint = "validating_completion";
    emit("agent.message", { preview: "Health checks green: web 1/1, api 1/1, worker 1/1." }, runId);
    schedule(runId, step4, 1600);
  };

  const step4 = () => {
    if (handle.finished) return;
    runtime.checkpoint = "reporting";
    runtime.report = {
      outcome: "succeeded",
      summary: "Staging stack deployed and verified. All three services report healthy; rollback not needed.",
      completed: ["Inspected compose configuration", "Applied the workspace change", "Verified service health"],
      incomplete: [],
      risks: [],
      evidenceIds: [],
      rollback: "docker compose -f docker-compose.staging.yml down && git checkout docker-compose.staging.yml",
      usage: { ...runtime.usage, modelTurns: 4, toolCalls: 2, inputTokens: 2180, outputTokens: 920, elapsedMs: 14200 },
      markdown:
        "# Deployment Report\n\n**Outcome:** succeeded\n\nApplied the staging compose stack and verified health of web/api/worker. No rollback required."
    };
    emit("run.reported", { summary: runtime.report.summary }, runId);
    schedule(runId, step5, 900);
  };

  const step5 = () => {
    if (handle.finished) return;
    setStatus(runId, "succeeded");
  };

  schedule(runId, step0, 600);
}

function approveCall(runId, callId) {
  const runtime = state.runtimes.get(runId);
  const call = runtime.toolCalls.find((c) => c.callId === callId);
  if (!call || call.status !== "waiting_for_approval") return null;
  call.status = "completed";
  call.resultSummary = "Applied the change to the workspace (rev 4).";
  call.completedAt = now();
  runtime.workspaceRevision = 4;
  setStatus(runId, "running");
  emit("tool.call_completed", { callId, resultSummary: call.resultSummary }, runId);
  // advance to verifying
  runScenarioNext(runId);
  return call;
}

function denyCall(runId, callId) {
  const runtime = state.runtimes.get(runId);
  const call = runtime.toolCalls.find((c) => c.callId === callId);
  if (!call) return null;
  call.status = "denied";
  setStatus(runId, "failed", "Tool call denied");
  return call;
}

function runScenarioNext(runId) {
  const handle = state.scenarios.get(runId);
  if (!handle || handle.finished) return;
  // step 2 approved -> verifying (step3)
  if (handle.prevStatus === "running") {
    schedule(runId, () => {
      if (handle.finished) return;
      setStatus(runId, "verifying");
      const runtime = state.runtimes.get(runId);
      runtime.checkpoint = "validating_completion";
      emit("agent.message", { preview: "Health checks green: web 1/1, api 1/1, worker 1/1." }, runId);
      schedule(runId, step4Next, 1400);
    }, 500);
  }
  let step4Next;
  step4Next = () => {
    if (handle.finished) return;
    const runtime = state.runtimes.get(runId);
    runtime.checkpoint = "reporting";
    runtime.report = {
      outcome: "succeeded",
      summary: "Staging stack deployed and verified. All three services report healthy; rollback not needed.",
      completed: ["Inspected compose configuration", "Applied the workspace change", "Verified service health"],
      incomplete: [],
      risks: [],
      evidenceIds: [],
      rollback: "docker compose -f docker-compose.staging.yml down && git checkout docker-compose.staging.yml",
      usage: { ...runtime.usage, modelTurns: 4, toolCalls: 2, elapsedMs: 14200 },
      markdown: "# Deployment Report\n\n**Outcome:** succeeded\n\nApplied the staging compose stack and verified health. No rollback required."
    };
    emit("run.reported", { summary: runtime.report.summary }, runId);
    schedule(runId, () => !handle.finished && setStatus(runId, "succeeded"), 900);
  };
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------
const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://preview");
  const path = url.pathname;
  const method = req.method ?? "GET";

  const sendJson = (status, body) => {
    const payload = JSON.stringify(body);
    res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(payload) });
    res.end(payload);
  };

  const bodyText = () =>
    new Promise((resolve) => {
      let data = "";
      req.on("data", (c) => (data += c));
      req.on("end", () => resolve(data));
      req.on("error", () => resolve(""));
    });

  const authed = () => (req.headers.cookie ?? "").includes("soloops_session=");

  // health
  if (["/healthz", "/livez"].includes(path)) return sendJson(200, { status: "ok", service: "soloops-preview-mock" });
  if (path === "/readyz") return sendJson(200, { status: "ready" });
  if (path === "/metrics") {
    res.writeHead(200, { "content-type": "text/plain" });
    return res.end("# soloops preview mock metrics\n");
  }

  if (path.startsWith("/api/")) {
    const seg = path.split("/").filter(Boolean); // ["api", ...]

    if (seg[1] === "auth") {
      if (seg[2] === "login" && method === "POST") {
        state.owner = { id: rid("owner"), username: "owner" };
        res.writeHead(200, { "content-type": "application/json", "set-cookie": `soloops_session=${rid("sess")}; Path=/; HttpOnly` });
        return res.end(JSON.stringify({ owner: state.owner }));
      }
      if (seg[2] === "session" && method === "GET") {
        if (!authed()) return sendJson(401, { error: { code: "unauthorized", message: "Session required" } });
        return sendJson(200, { owner: state.owner ?? { id: "owner", username: "owner" } });
      }
      if (seg[2] === "logout" && method === "POST") {
        state.owner = null;
        res.writeHead(204); return res.end();
      }
    }

    if (!authed()) return sendJson(401, { error: { code: "unauthorized", message: "Session required" } });

    if (seg[1] === "tasks") {
      if (method === "GET" && seg.length === 2) return sendJson(200, { items: state.tasks });
      if (method === "POST" && seg.length === 2) {
        const body = JSON.parse((await bodyText()) || "{}");
        const { task, runId } = makeTask(String(body.title ?? "Untitled"), String(body.goal ?? ""), "queued", 0);
        emit("run.created", { taskId: task.id, runId }, runId);
        runScenario(runId);
        return sendJson(201, task);
      }
    }

    if (seg[1] === "runs" && seg.length >= 3) {
      const runId = seg[2];
      if (method === "GET" && seg.length === 3) {
        if (!state.runtimes.has(runId)) return sendJson(404, { error: { code: "not_found", message: "Run not found" } });
        return sendJson(200, runDetail(runId));
      }
      if (method === "GET" && seg.length === 4 && seg[3] === "runtime") {
        const runtime = state.runtimes.get(runId);
        if (!runtime) return sendJson(404, { error: { code: "not_found", message: "Runtime not found" } });
        return sendJson(200, runtime);
      }
      if (method === "POST" && seg.length === 4 && seg[3] === "cancel") {
        const handle = state.scenarios.get(runId);
        if (handle) {
          if (handle.timer) clearTimeout(handle.timer);
          handle.timer = null;
          setStatus(runId, "cancelled", "Cancelled by owner");
        }
        return sendJson(200, runDetail(runId));
      }
      if (seg.length === 6 && seg[3] === "tool-calls" && seg[5] === "decision") {
        const callId = decodeURIComponent(seg[4]);
        const body = JSON.parse((await bodyText()) || "{}");
        const call =
          body.decision === "approve" ? approveCall(runId, callId) : denyCall(runId, callId);
        if (!call) return sendJson(404, { error: { code: "not_found", message: "Tool call not found" } });
        return sendJson(200, call);
      }
    }

    if (seg[1] === "events" && method === "GET") {
      const after = Number(url.searchParams.get("after") ?? "0");
      const runId = url.searchParams.get("runId") ?? undefined;
      return sendJson(200, {
        items: state.events.filter((e) => e.sequence > after && (runId === undefined || e.runId === runId))
      });
    }

    return sendJson(404, { error: { code: "not_found", message: "Not found" } });
  }

  // static SPA
  let filePath = normalize(join(ROOT, path === "/" ? "index.html" : path));
  if (!filePath.startsWith(ROOT)) {
    res.writeHead(403); return res.end("Forbidden");
  }
  try {
    const info = await stat(filePath);
    if (info.isDirectory()) filePath = join(filePath, "index.html");
  } catch {
    filePath = join(ROOT, "index.html"); // SPA fallback
  }
  try {
    const data = await readFile(filePath);
    const type = MIME[extname(filePath)] ?? "application/octet-stream";
    res.writeHead(200, { "content-type": type });
    res.end(data);
  } catch {
    res.writeHead(404); res.end("Not found");
  }
});

// ---------------------------------------------------------------------------
// WebSocket events
// ---------------------------------------------------------------------------
const wss = new WebSocketServer({ noServer: true });
const conns = new Set();

server.on("upgrade", (req, socket, head) => {
  const url = new URL(req.url, "http://preview");
  if (!url.pathname.startsWith("/api/events")) return;
  wss.handleUpgrade(req, socket, head, (ws) => {
    conns.add(ws);
    // replay backlog
    const after = Number(url.searchParams.get("after") ?? "0");
    const runId = url.searchParams.get("runId") ?? undefined;
    for (const evt of state.events.filter((e) => e.sequence > after && (runId === undefined || e.runId === runId))) {
      ws.send(JSON.stringify(evt));
    }
    ws.on("close", () => conns.delete(ws));
  });
});

// broadcast new events to subscribers
let lastSeq = 0;
setInterval(() => {
  if (conns.size === 0) return;
  for (const evt of state.events) {
    if (evt.sequence <= lastSeq) continue;
    const payload = JSON.stringify(evt);
    for (const ws of conns) if (ws.readyState === 1) ws.send(payload);
    lastSeq = evt.sequence;
  }
}, 120);

server.listen(PORT, "127.0.0.1", () => {
  console.log(`SoloOps preview (mock API) → http://127.0.0.1:${PORT}`);
});