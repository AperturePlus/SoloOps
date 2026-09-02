// UI audit capture: logs into the mock WebUI and screenshots every page at desktop size.
// Usage: node capture.mjs   (or bun capture.mjs)   BASE_URL env overrides target
import { chromium } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.BASE_URL ?? "http://127.0.0.1:5174";
mkdirSync(here, { recursive: true });
const log = (...args) => console.log("[capture]", ...args);

const browser = await chromium.launch();
log("browser launched");
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

await page.goto(`${base}/login`, { waitUntil: "domcontentloaded", timeout: 30000 });
log("login page open");
await page.fill('input[type="password"]', "mock-password");
await page.click('form button:has-text("Sign in")');
await page.waitForURL("**/tasks", { timeout: 15000 });
await page.waitForTimeout(2000);
log("logged in");

const runId = await page.evaluate(async () => {
  const res = await fetch("/api/tasks");
  const data = await res.json();
  return data.items?.find((t) => t.latestRunId)?.latestRunId ?? null;
});
log("runId:", runId);

const shots = [
  ["overview", "/"],
  ["tasks", "/tasks"],
  ["new-task", "/tasks/new"],
  ...(runId ? [["run-detail", `/runs/${runId}`]] : []),
  ["settings", "/settings/notifications"]
];

for (const [name, path] of shots) {
  await page.goto(`${base}${path}`, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.waitForTimeout(1800);
  await page.screenshot({ path: join(here, `${name}.png`), fullPage: true });
  log("captured", name);
}

await browser.close();
log("done");
