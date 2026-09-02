// Phase-2 capture: populate the mock with a live task, then screenshot
// the run detail page in its two most important states (approval + report).
import { chromium } from "@playwright/test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.BASE_URL ?? "http://127.0.0.1:5174";
const log = (...args) => console.log("[capture2]", ...args);

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

// scenario steps are 1.5s each; waiting_for_approval is reached after ~4 steps
await page.waitForTimeout(6500);

await page.goto(`${base}/runs/${task.latestRunId}`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.screenshot({ path: join(here, "run-approval.png"), fullPage: true });
log("captured run-approval");

// approve through the UI and wait for the run to finish with a report
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
