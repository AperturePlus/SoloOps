// Visual verification driver for the polished agent page (mock mode).
// Run: bun e2e/agent-page-visual-check.mjs
import { chromium } from "@playwright/test";

const BASE = "http://localhost:5173";
const OUT = "test-results/agent-page-polish";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1360, height: 940 } });

try {
  // 1. Login
  await page.goto(`${BASE}/login`);
  await page.fill('input[autocomplete="username"]', "owner");
  await page.fill('input[type="password"]', "mock-pass");
  await page.getByRole("button", { name: /sign in/i }).click();
  await page.waitForURL("**/tasks", { timeout: 15_000 });

  // 2. Create a task -> redirects to /runs/:id
  await page.goto(`${BASE}/tasks/new`);
  await page.fill('input[maxlength="160"]', "Polish verification run");
  await page.fill("textarea", "Validate the Codex-style agent page end to end in mock mode.");
  await page.locator("form button").last().click();
  await page.waitForURL("**/runs/**", { timeout: 15_000 });

  // 3. Planning phase screenshot (plan panel + working indicator)
  await page.waitForTimeout(2_200);
  await page.screenshot({ path: `${OUT}/1-planning.png`, fullPage: false });

  // 4. Waiting for approval screenshot
  await page.waitForTimeout(4_000);
  await page.screenshot({ path: `${OUT}/2-approval.png`, fullPage: false });

  // 5. Expand the approval card, then approve
  const approveButton = page.getByRole("button", { name: "Approve", exact: true });
  if (await approveButton.count()) {
    const card = approveButton.locator("xpath=ancestor::article");
    await card.locator("button").first().click();
    await page.waitForTimeout(500);
    await page.screenshot({ path: `${OUT}/3-approval-expanded.png`, fullPage: false });
    await approveButton.click();
  }

  // 6. Wait for the run to finish and screenshot the final state
  await page.waitForTimeout(6_000);
  await page.screenshot({ path: `${OUT}/4-final.png`, fullPage: false });

  // 7. Full page capture of the finished timeline
  await page.screenshot({ path: `${OUT}/5-final-full.png`, fullPage: true });

  console.log("OK: screenshots written to", OUT);
} catch (cause) {
  await page.screenshot({ path: `${OUT}/failure.png`, fullPage: true });
  console.error("FAILED:", cause);
  process.exitCode = 1;
} finally {
  await browser.close();
}
