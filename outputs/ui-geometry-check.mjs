// Geometric assertions for the composite layouts (round 2).
// Confirms each page actually uses multiple planes instead of a narrow stack.
import { chromium } from "@playwright/test";

const BASE = "http://127.0.0.1:5199";
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
    goal: "Bring up the staging stack, verify health, write a report.",
    status: "succeeded",
    latestRunId: RUN_ID,
    createdAt: now - 3600_000 * 26
  },
  {
    id: "task_2",
    title: "Extract contact data",
    goal: "Parse CSV and dedupe.",
    status: "running",
    latestRunId: "run_d8c2e11a",
    createdAt: now - 3600_000 * 1
  }
];

const RUNTIME = {
  runId: RUN_ID,
  checkpoint: "executing_tools",
  plan: {
    summary: "Deploy, verify, report.",
    steps: [
      { id: "s1", title: "Inspect workspace", status: "completed", required: true },
      { id: "s2", title: "Apply change", status: "in_progress", required: true },
      { id: "s3", title: "Verify result", status: "pending", required: true }
    ]
  },
  budget: { maxModelTurns: 8, maxToolCalls: 6, maxInputTokens: 4000, maxOutputTokens: 2000, maxDurationMs: 60000, maxToolDurationMs: 10000, maxToolOutputBytes: 10000, maxWorkspaceBytes: 100000 },
  usage: { modelTurns: 3, toolCalls: 2, inputTokens: 1840, outputTokens: 720, cachedInputTokens: 0, cacheWriteInputTokens: 0, elapsedMs: 18500 },
  workspaceRevision: 4,
  toolCalls: [
    { callId: "call_1", name: "workspace.read", argumentsSha256: "a".repeat(64), approvalPreview: null, risk: "read_only", policy: "allow", status: "completed", resultSummary: "Read compose file.", errorCategory: null, startedAt: now - 16000, completedAt: now - 12000 },
    { callId: "call_2", name: "workspace.replace", argumentsSha256: "b".repeat(64), approvalPreview: { action: "replace", path: "result.txt", summary: "Replace artifact" }, risk: "workspace_write", policy: "require_approval", status: "waiting_for_approval", resultSummary: null, errorCategory: null, startedAt: now - 9000, completedAt: null }
  ],
  evidence: [
    { id: "ev_11", toolCallId: "call_1", kind: "file.read", summary: "Compose present.", artifactRef: "workspace/compose.yml", contentSha256: "c".repeat(64), workspaceRevision: 1, createdAt: now - 12000 }
  ],
  report: null
};

const EVENTS = [];
for (let i = 0; i < 40; i++) {
  EVENTS.push(
    evt(i % 3 === 0 ? "agent.message" : i % 3 === 1 ? "tool.call_started" : "tool.call_completed", {
      preview: `Agent working through step ${i}`,
      callId: `call_${i}`,
      name: "workspace.read",
      resultSummary: `Read file ${i}.`
    })
  );
}

const results = [];
const expect = (name, cond, detail = "") => {
  results.push({ name, pass: !!cond, detail });
  console.log(`${cond ? "PASS" : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
};

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });

await page.route("**/api/**", (route) => {
  const u = new URL(route.request().url());
  const p = u.pathname;
  if (p === "/api/auth/session") return route.fulfill({ json: { owner: { id: "owner", username: "owner" } } });
  if (p === "/api/tasks") return route.fulfill({ json: { items: TASKS } });
  if (p === `/api/runs/${RUN_ID}`) return route.fulfill({ json: { id: RUN_ID, taskId: TASK_ID, status: "waiting_for_approval", statusReason: null, createdAt: now - 26000, startedAt: now - 25000, finishedAt: null } });
  if (p === `/api/runs/${RUN_ID}/runtime`) return route.fulfill({ json: RUNTIME });
  if (p.startsWith("/api/events")) {
    const after = Number(u.searchParams.get("after") ?? "0");
    return route.fulfill({ json: { items: EVENTS.filter((e) => e.sequence > after) } });
  }
  if (p === "/api/settings/ip-notifications") {
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
  // ============ 1. Home overview ============
  await page.goto(`${BASE}/`);
  await page.waitForSelector("text=Queue glance");
  const homeBody = await page.evaluate(() => {
    const width = document.documentElement.clientWidth;
    const hero = document.querySelector("main");
    const capGrid = hero?.querySelector(".sm\\:grid-cols-2");
    const aside = hero?.querySelector("aside");
    return {
      width,
      heroLeft: hero?.children[0]?.getBoundingClientRect().x ?? 0,
      heroRight: aside?.getBoundingClientRect().right ?? 0,
      capGridCols: capGrid ? getComputedStyle(capGrid).gridTemplateColumns : "n/a"
    };
  });
  expect("home: hero uses full width (no 3xl shrink)", homeBody.heroRight - homeBody.heroLeft > 1200, `span=${Math.round(homeBody.heroRight - homeBody.heroLeft)}px`);
  expect("home: capability grid is multi-column", homeBody.capGridCols.split(" ").length >= 2, homeBody.capGridCols);

  // ============ 2. Login split ============
  await page.goto(`${BASE}/login`);
  await page.waitForSelector("text=Owner sign in");
  const login = await page.evaluate(() => {
    const brand = document.querySelector("section");
    const form = document.querySelector("form")?.getBoundingClientRect();
    return {
      brandVisible:
        !!brand &&
        getComputedStyle(brand).display !== "none" &&
        brand.getBoundingClientRect().width > 300,
      formX: Math.round(form?.x ?? -1),
      winWidth: document.documentElement.clientWidth
    };
  });
  expect("login: left brand pane rendered", login.brandVisible);
  expect(
    "login: split layout (brand left, form right)",
    login.formX > login.winWidth * 0.4,
    `form.x=${login.formX} win=${login.winWidth}`
  );

  // ============ 3. Tasks workspace ============
  await page.goto(`${BASE}/tasks`);
  await page.waitForSelector("text=Deploy the staging compose stack");
  const tasks = await page.evaluate(() => {
    const metric = [...document.querySelectorAll(".metric-chip")];
    const detail = document.querySelector(".metric-chip")?.closest("div")?.parentElement;
    return {
      metricCount: metric.length,
      detailWidth: document.querySelector("main, .p-5")?.getBoundingClientRect().width ?? 0,
      winWidth: document.documentElement.clientWidth
    };
  });
  expect("tasks: command strip has 4 metric chips", tasks.metricCount === 4, `count=${tasks.metricCount}`);
  expect("tasks: detail zone spans full remaining width", tasks.detailWidth > 900, `${Math.round(tasks.detailWidth)}px`);

  // Collapse check
  await page.click('button[aria-label="Collapse task list"]');
  await page.waitForTimeout(350);
  const collapsed = await page.evaluate(() => {
    const aside = document.querySelector("aside");
    return aside ? aside.getBoundingClientRect().width : -1;
  });
  expect("tasks: list collapses to rail", collapsed <= 60, `${Math.round(collapsed)}px`);

  // ============ 4. Run detail 3-column ============
  await page.goto(`${BASE}/runs/${RUN_ID}`);
  await page.waitForSelector("text=Tool calls");
  const run = await page.evaluate(() => {
    const cols = [...document.querySelectorAll(".grid.grid-cols-12 > div")];
    return cols.map((c) => ({ w: Math.round(c.getBoundingClientRect().width), x: Math.round(c.getBoundingClientRect().x) }));
  });
  expect("run: 3 grid columns rendered", run.length === 3, `cols=${run.length}`);
  if (run.length === 3) {
    expect("run: activity column is middle (not squeezed)", run[1]?.w > 500, `w=${run[1]?.w}`);
  }
  const activityScroll = await page.evaluate(() => {
    // The activity scroller is the overflow-y-auto element inside the Activity panel.
    const scroller = [...document.querySelectorAll(".overflow-y-auto")].find(
      (d) => d.scrollHeight > d.clientHeight && d.clientHeight > 250
    );
    return scroller ? { h: scroller.clientHeight, sh: scroller.scrollHeight } : null;
  });
  expect("run: activity panel has internal scroll", !!activityScroll, activityScroll ? `client=${activityScroll.h} scroll=${activityScroll.sh}` : "no scroller");

  // ============ 5. Settings composite ============
  await page.goto(`${BASE}/settings/notifications`);
  await page.waitForSelector("text=Public IP notifications");
  await page.waitForSelector("text=Current IPv4", { timeout: 10000 });
  const settings = await page.evaluate(() => {
    // Use the rail inside the settings route (it has the "Settings" label), not the global 56px nav.
    const rail = [...document.querySelectorAll("aside")].find(
      (a) => a.getBoundingClientRect().width > 120
    );
    const strip = document.querySelector(".command-strip");
    const content = document.querySelector(".p-6");
    return {
      railVisible: !!rail,
      railW: rail?.getBoundingClientRect().width ?? 0,
      stripRows: strip ? strip.children.length : 0,
      contentX: Math.round(content?.getBoundingClientRect().x ?? -1)
    };
  });
  expect("settings: left rail exists", settings.railVisible, `w=${Math.round(settings.railW)}px`);
  expect("settings: status strip has 3 rows", settings.stripRows === 3, `rows=${settings.stripRows}`);
  expect("settings: content starts right of rail", settings.contentX > settings.railW, `x=${settings.contentX}`);

  console.log("FAILURES:", results.filter((r) => !r.pass).length);
  process.exitCode = results.some((r) => !r.pass) ? 1 : 0;
} catch (cause) {
  console.error("CRASHED:", cause);
  process.exitCode = 1;
} finally {
  await browser.close();
}