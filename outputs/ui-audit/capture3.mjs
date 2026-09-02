// Phase-3 capture: take the already-waiting run, expand its tool call, approve,
// then screenshot the finished report state and the populated task list.
import { chromium } from "@playwright/test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.BASE_URL ?? "http://127.0.0.1:5174";
const log = (...args) => console.log("[capture3]", ...args);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

await page.goto(`${base}/login`, { waitUntil: "domcontentloaded" });
await page.fill('input[type="password"]', "mock-password");
await page.click('form button:has-text("Sign in")');
await page.waitForURL("**/tasks", { timeout: 15000 });
log("logged in");

const runId = await page.evaluate(async () => {
  const res = await fetch("/api/tasks");
  const data = await res.json();
  return data.items?.find((t) => t.latestRunId)?.latestRunId ?? null;
});
log("run:", runId);

await page.goto(`${base}/runs/${runId}`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200);

// expand the awaiting tool call, then approve
await page.click('button[aria-expanded="false"]');
await page.waitForTimeout(400);
await page.click('button:has-text("Approve")');
log("approved");
await page.waitForTimeout(9000);

await page.screenshot({ path: join(here, "run-report.png"), fullPage: true });
log("captured run-report");

await page.goto(`${base}/tasks`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.screenshot({ path: join(here, "tasks-populated.png"), fullPage: true });
log("captured tasks-populated");

await browser.close();
log("done");
