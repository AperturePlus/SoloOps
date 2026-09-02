// Full-page UI audit: capture every route in mock mode at desktop viewport.
// Run: bun e2e/ui-audit.mjs   (requires `bun run dev:mock` on :5173)
import { chromium } from "@playwright/test";

const BASE = process.env.UI_AUDIT_BASE ?? "http://127.0.0.1:5173";
const OUT = "test-results/ui-audit";
const VIEWPORTS = [
  { name: "desktop", width: 1440, height: 900 },
  { name: "narrow", width: 420, height: 900 }
];

const browser = await chromium.launch();

for (const vp of VIEWPORTS) {
  const context = await browser.newContext({ viewport: { width: vp.width, height: vp.height } });
  const page = await context.newPage();

  // Login first (mock mode accepts any credentials per scenario.ts).
  await page.goto(`${BASE}/login`);
  await page.fill('input[autocomplete="username"]', "owner");
  await page.fill('input[type="password"]', "mock-pass");
  await page.getByRole("button", { name: /sign in/i }).click();
  await page.waitForURL("**/tasks", { timeout: 15_000 });

  const shots = [
    ["tasks", `${BASE}/tasks`],
    ["task-new", `${BASE}/tasks/new`],
    ["overview", `${BASE}/`],
    ["settings", `${BASE}/settings/notifications`]
  ];

  for (const [name, url] of shots) {
    await page.goto(url);
    await page.waitForTimeout(900);
    await page.screenshot({ path: `${OUT}/${vp.name}-${name}.png`, fullPage: true });
  }

  // Run detail: create a task to enter the live run page, capture phases.
  await page.goto(`${BASE}/tasks/new`);
  await page.fill('input[maxlength="160"]', "UI audit run");
  await page.fill("textarea", "Capture run page states for the UI audit.");
  await page.locator("form button").last().click();
  await page.waitForURL("**/runs/**", { timeout: 15_000 });
  await page.waitForTimeout(1_800);
  await page.screenshot({ path: `${OUT}/${vp.name}-run-planning.png`, fullPage: false });
  await page.waitForTimeout(4_000);
  await page.screenshot({ path: `${OUT}/${vp.name}-run-approval.png`, fullPage: false });
  const approve = page.getByRole("button", { name: "Approve", exact: true });
  if (await approve.count()) await approve.click();
  await page.waitForTimeout(7_000);
  await page.screenshot({ path: `${OUT}/${vp.name}-run-final.png`, fullPage: false });
  await page.screenshot({ path: `${OUT}/${vp.name}-run-final-full.png`, fullPage: true });

  await context.close();
}

await browser.close();
console.log("OK: UI audit screenshots written to", OUT);
