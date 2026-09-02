// Final verification: full page sweep after the component-library migration.
import { chromium } from "@playwright/test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.BASE_URL ?? "http://127.0.0.1:5174";
const log = (...args) => console.log("[final]", ...args);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

await page.goto(`${base}/login`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(800);
await page.screenshot({ path: join(here, "final-login.png"), fullPage: true });
log("captured final-login");

await page.fill('input[type="password"]', "mock-password");
await page.click('form button:has-text("Sign in")');
await page.waitForURL("**/tasks", { timeout: 15000 });
await page.waitForTimeout(1200);

await page.goto(`${base}/`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200);
await page.screenshot({ path: join(here, "final-overview.png"), fullPage: true });
log("captured final-overview");

await page.goto(`${base}/tasks/new`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(800);
await page.screenshot({ path: join(here, "final-new-task.png"), fullPage: true });
log("captured final-new-task");

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
await page.screenshot({ path: join(here, "final-run-approval.png"), fullPage: true });
log("captured final-run-approval");

await page.click('section[aria-label="Approval required"] button:has-text("Approve")', {
  timeout: 8000
});
await page.waitForTimeout(9000);
await page.screenshot({ path: join(here, "final-run-report.png"), fullPage: true });
log("captured final-run-report");

await page.goto(`${base}/tasks`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.screenshot({ path: join(here, "final-tasks.png"), fullPage: true });
log("captured final-tasks");

await page.goto(`${base}/settings/notifications`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200);
await page.screenshot({ path: join(here, "final-settings.png"), fullPage: true });
log("captured final-settings");

await browser.close();
log("done");
