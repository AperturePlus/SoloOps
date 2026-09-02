// Verify the surfaced approval banner: create a task, wait for approval state,
// screenshot, approve through the banner, then screenshot the report state.
import { chromium } from "@playwright/test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.BASE_URL ?? "http://127.0.0.1:5175";
const log = (...args) => console.log("[verify]", ...args);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

await page.goto(`${base}/login`, { waitUntil: "domcontentloaded" });
await page.fill('input[type="password"]', "mock-password");
await page.click('form button:has-text("Sign in")');
await page.waitForURL("**/tasks", { timeout: 15000 });
log("logged in");

const task = await page.evaluate(async () => {
  const res = await fetch("/api/tasks", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      title: "Rotate staging deploy",
      goal: "Redeploy the compose stack on staging, verify health endpoints and report."
    })
  });
  return res.json();
});
log("task created:", task.latestRunId);

await page.waitForTimeout(6500);
await page.goto(`${base}/runs/${task.latestRunId}`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.screenshot({ path: join(here, "run-approval.png"), fullPage: true });
log("captured run-approval");

// approve through the surfaced banner (must be directly clickable now)
await page.click('section[aria-label="Approval required"] button:has-text("Approve")', {
  timeout: 8000
});
log("approved via banner");
await page.waitForTimeout(9000);
await page.screenshot({ path: join(here, "run-report.png"), fullPage: true });
log("captured run-report");

await page.goto(`${base}/tasks`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.screenshot({ path: join(here, "tasks-populated.png"), fullPage: true });
log("captured tasks-populated");

await browser.close();
log("done");
