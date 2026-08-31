// Visual check harness for the Codex-style composite rebuild (round 2).
// Runs against the static preview build; all /api calls are faked in-process
// with realistic agent-run data so we can screenshot every composite layout.
import { chromium } from "@playwright/test";

const BASE = "http://127.0.0.1:5199";
const OUT_DIR = "D:/tsprj/soloOps/outputs/ui-shots";

const shot = async (page, name) => {
  await page.waitForTimeout(450);
  await page.screenshot({ path: `${OUT_DIR}/${name}.png`, fullPage: true });
  console.log("saved", name);
};

// ---- Fake backend ---------------------------------------------------------
const RUN_ID = "run_9f3a2c81e7";
const TASK_ID = "task_7b1d40aa";
const now = Date.now();
const seq = { n: 10 };

const evt = (type, payload, runId = RUN_ID) => ({
  sequence: seq.n++,
  id: `evt_${seq.n}`,
  runId,
  type,
  payload,
  createdAt: now - (20 - seq.n) * 800
});

const TASKS = [
  {
    id: TASK_ID,
    title: "Deploy the staging compose stack",
    goal:
      "Bring up the staging stack from docker-compose.staging.yml, verify all three services report healthy, and write a short report. Roll back automatically if health checks fail.",
    status: "succeeded",
    latestRunId: RUN_ID,
    createdAt: now - 3600_000 * 26
  },
  {
    id: "task_2",
    title: "Extract contact data for the newsletter list",
    goal: "Parse the exported CSV, dedupe by email, and produce a clean CSV with headers.",
    status: "failed",
    latestRunId: "run_d8c2e11a",
    createdAt: now - 3600_000 * 5
  },
  {
    id: "task_3",
    title: "Nightly backup verification",
    goal: "Run the backup verification job and report any corrupted archives.",
    status: "running",
    latestRunId: "run_a01f88c3",
    createdAt: now - 3600_000 * 1
  }
];

const RUNTIME = {
  runId: RUN_ID,
  checkpoint: "executing_tools",
  plan: {
    summary: "Inspect compose configuration, deploy the stack, then verify health.",
    steps: [
      { id: "s1", title: "Inspect workspace", status: "completed", required: true },
      { id: "s2", title: "Apply change", status: "in_progress", required: true },
      { id: "s3", title: "Verify result", status: "pending", required: true },
      { id: "s4", title: "Write final report", status: "pending", required: false }
    ]
  },
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
    modelTurns: 3,
    toolCalls: 2,
    inputTokens: 1840,
    outputTokens: 720,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    elapsedMs: 18500
  },
  workspaceRevision: 4,
  toolCalls: [
    {
      callId: "call_1",
      name: "workspace.read",
      argumentsSha256: "a".repeat(64),
      approvalPreview: null,
      risk: "read_only",
      policy: "allow",
      status: "completed",
      resultSummary: "Read docker-compose.staging.yml (2.4 kb).",
      errorCategory: null,
      startedAt: now - 16000,
      completedAt: now - 12000
    },
    {
      callId: "call_2",
      name: "workspace.replace",
      argumentsSha256: "b".repeat(64),
      approvalPreview: {
        action: "replace",
        path: "result.txt",
        summary: "Replace one workspace artifact"
      },
      risk: "workspace_write",
      policy: "require_approval",
      status: "waiting_for_approval",
      resultSummary: null,
      errorCategory: null,
      startedAt: now - 9000,
      completedAt: null
    }
  ],
  evidence: [
    {
      id: "ev_11",
      toolCallId: "call_1",
      kind: "file.read",
      summary: "Compose config present with 3 services defined.",
      artifactRef: "workspace/docker-compose.staging.yml",
      contentSha256: "c".repeat(64),
      workspaceRevision: 1,
      createdAt: now - 12000
    },
    {
      id: "ev_12",
      toolCallId: "call_1",
      kind: "file.stat",
      summary: "Workspace size 12.4 kb, 5 files.",
      artifactRef: null,
      contentSha256: null,
      workspaceRevision: 1,
      createdAt: now - 11000
    }
  ],
  report: null
};

const EVENTS = [
  evt("run.created", { taskId: TASK_ID, runId: RUN_ID }),
  evt("run.status_changed", { from: "queued", to: "planning" }),
  evt("agent.plan_updated", { plan: RUNTIME.plan }),
  evt("agent.message", { preview: "Proposing a plan with 4 steps." }),
  evt("tool.call_started", { callId: "call_1", name: "workspace.read" }),
  evt("tool.call_completed", { callId: "call_1", resultSummary: "Read compose file." }),
  evt("run.status_changed", { from: "planning", to: "running" }),
  evt("tool.call_started", { callId: "call_2", name: "workspace.replace" })
];

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });

await page.route("**/api/**", async (route) => {
  const url = new URL(route.request().url());
  const method = route.request().method();
  const path = url.pathname;
  if (path === "/api/auth/session") {
    return route.fulfill({ json: { owner: { id: "owner", username: "owner" } } });
  }
  if (path === "/api/auth/login") {
    return route.fulfill({ json: { owner: { id: "owner", username: "owner" } } });
  }
  if (path === "/api/auth/logout") {
    return route.fulfill({ status: 204, body: "" });
  }
  if (path === "/api/tasks") {
    return route.fulfill({ json: { items: TASKS } });
  }
  if (path === `/api/runs/${RUN_ID}`) {
    return route.fulfill({
      json: {
        id: RUN_ID,
        taskId: TASK_ID,
        status: "waiting_for_approval",
        statusReason: null,
        createdAt: now - 26000,
        startedAt: now - 25000,
        finishedAt: null
      }
    });
  }
  if (path === `/api/runs/${RUN_ID}/runtime`) {
    return route.fulfill({ json: RUNTIME });
  }
  if (path.startsWith("/api/events")) {
    const after = Number(url.searchParams.get("after") ?? "0");
    return route.fulfill({ json: { items: EVENTS.filter((e) => e.sequence > after) } });
  }
  if (path.includes("/decision")) {
    return route.fulfill({ json: { ok: true } });
  }
  if (path === "/api/settings/ip-notifications") {
    return route.fulfill({
      json: {
        enabled: true,
        smtpConfigured: true,
        currentIpv4: "203.0.113.7",
        lastCheckedAt: now - 300_000,
        lastChangedAt: now - 3600_000,
        recipients: [
          { email: "owner@example.com", lastNotifiedIpv4: null, lastNotifiedAt: null, lastAttemptAt: null, lastError: null },
          { email: "backup@example.com", lastNotifiedIpv4: "203.0.113.7", lastNotifiedAt: now - 3600_000, lastAttemptAt: now - 3600_000, lastError: null }
        ]
      }
    });
  }
  route.fulfill({ status: 404, json: { error: { code: "not_found", message: "no mock" } } });
});

try {
  // 1. Home overview — composite landing (authed)
  await page.goto(`${BASE}/`);
  await page.waitForSelector("text=Queue glance");
  await shot(page, "01-home-overview");

  // 2. Login — split-screen composite
  await page.goto(`${BASE}/login`);
  await page.waitForSelector("text=Owner sign in");
  await shot(page, "02-login-split");

  // 3. Tasks workspace (list populated)
  await page.goto(`${BASE}/tasks`);
  await page.waitForSelector("text=Deploy the staging compose stack");
  await shot(page, "03-tasks-workspace");

  // 4. Collapse list -> detail stays, space reused
  await page.click('button[aria-label="Collapse task list"]');
  await page.waitForTimeout(400);
  await shot(page, "04-tasks-collapsed");

  // 5. Run detail — 3-column composite grid
  await page.goto(`${BASE}/runs/${RUN_ID}`);
  await page.waitForSelector("text=Tool calls");
  await shot(page, "05-run-detail");

  // 6. Settings — left rail + composite content
  await page.goto(`${BASE}/settings/notifications`);
  await page.waitForSelector("text=Public IP notifications");
  await shot(page, "06-settings");

  // 7. New task page
  await page.goto(`${BASE}/tasks/new`);
  await page.waitForSelector("text=Create a task");
  await shot(page, "07-new-task");

  console.log("DONE");
} catch (cause) {
  console.error("FAILED:", cause);
  await page.screenshot({ path: `${OUT_DIR}/error.png`, fullPage: true });
  process.exitCode = 1;
} finally {
  await browser.close();
}