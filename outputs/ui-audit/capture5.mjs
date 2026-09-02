// P1 verification: overview workbench + tasks detail density.
import { chromium } from "@playwright/test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.BASE_URL ?? "http://127.0.0.1:5175";
const log = (...args) => console.log("[p1]", ...args);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

await page.goto(`${base}/login`, { waitUntil: "domcontentloaded" });
await page.fill('input[type="password"]', "mock-password");
await page.click('form button:has-text("Sign in")');
await page.waitForURL("**/tasks", { timeout: 15000 });
await page.waitForTimeout(1200);

await page.goto(`${base}/`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200);
await page.screenshot({ path: join(here, "overview-v2.png"), fullPage: true });
log("captured overview-v2");

await page.goto(`${base}/tasks`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.screenshot({ path: join(here, "tasks-v2.png"), fullPage: true });
log("captured tasks-v2");

await browser.close();
log("done");
