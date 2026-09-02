import { chromium } from "@playwright/test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.BASE_URL ?? "http://127.0.0.1:5174";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
page.on("console", (msg) => {
  if (msg.type() === "error" || msg.type() === "warning") {
    console.log(`[console.${msg.type()}]`, msg.text().slice(0, 500));
  }
});
page.on("pageerror", (err) => {
  console.log("[pageerror]", String(err).slice(0, 800));
});
await page.goto(`${base}/login`, { waitUntil: "domcontentloaded" });
await page.fill('input[type="password"]', "mock-password");
await page.click('form button:has-text("Sign in")');
await page.waitForURL("**/tasks", { timeout: 15000 });
await page.goto(`${base}/settings/notifications`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(2500);
const html = await page.evaluate(() => document.body.innerHTML.length);
console.log("body html length:", html);
await browser.close();
