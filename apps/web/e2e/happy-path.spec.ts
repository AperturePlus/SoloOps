import { expect, test, type Page } from "@playwright/test";

const E2E_PASSWORD = "correct horse battery staple";

async function signIn(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Username").fill("owner");
  await page.getByLabel("Password").fill(E2E_PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("textbox", { name: "Filter tasks…" })).toBeVisible();
}

async function createTask(page: Page, title: string, goal: string) {
  await page.getByRole("link", { name: "New task" }).first().click();
  await page.getByLabel("Title").fill(title);
  await page.getByLabel("Goal").fill(goal);
  await page.getByRole("button", { name: "Create task" }).click();
  await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();
}

// The run detail page shows the title both in the toolbar and in the report
// markdown, so report assertions must be scoped inside the run header.
function runStatus(page: Page) {
  return page.getByRole("banner").getByText("Succeeded", { exact: true });
}

test("owner approves a workspace change and receives a final report after reload", async ({
  page
}) => {
  await signIn(page);
  await createTask(
    page,
    "Browser E2E",
    "Create one approved artifact and report the evidence."
  );

  const banner = page.getByRole("alert", { name: "Approval required" });
  await expect(banner.getByText("workspace.create")).toBeVisible({ timeout: 15_000 });
  await expect(banner.getByText("workspace_write", { exact: true })).toBeVisible();

  await page.reload();
  await expect(banner.getByText("workspace.create")).toBeVisible();
  await banner.getByRole("button", { name: "Approve" }).click();

  await expect(runStatus(page)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole("heading", { name: "Final report" })).toBeVisible();
  await expect(
    page.getByText("The browser-driven E2E task completed successfully.").first()
  ).toBeVisible();
});

test("owner reviews and approves an exact managed deployment proposal", async ({ page }) => {
  await signIn(page);
  await createTask(
    page,
    "Managed deployment E2E",
    "Complete the managed deployment flow with exact Owner approval."
  );

  // `managed.deploy.plan` is read-only: the proposal is produced without a
  // decision, and only the privileged `managed.deploy.apply` gates the run.
  const banner = page.getByRole("alert", { name: "Approval required" });
  await expect(banner.getByText("managed.deploy.apply")).toBeVisible({ timeout: 15_000 });
  await expect(banner.getByText("privileged", { exact: true })).toBeVisible();
  await expect(banner.getByText("e2e.example.test")).toBeVisible();
  await expect(banner.getByText(/proposalSha256/)).toBeVisible();

  // Reloading must restore the exact same proposal before the decision.
  await page.reload();
  await expect(banner.getByText("managed.deploy.apply")).toBeVisible();
  await expect(banner.getByText("e2e.example.test")).toBeVisible();
  await banner.getByRole("button", { name: "Approve" }).click();

  await expect(runStatus(page)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole("heading", { name: "Final report" })).toBeVisible();
  await expect(page.getByText("The managed deployment completed successfully.").first()).toBeVisible();
});
