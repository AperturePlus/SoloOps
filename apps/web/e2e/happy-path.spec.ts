import { expect, test } from "@playwright/test";

test("owner approves a workspace change and receives a final report after reload", async ({
  page
}) => {
  await page.goto("/login");
  await page.getByLabel("Username").fill("owner");
  await page.getByLabel("Password").fill("correct horse battery staple");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { name: "Tasks" })).toBeVisible();
  await expect(page.getByText("Phase 3.1", { exact: true })).toBeVisible();

  await page.getByRole("link", { name: "New task" }).click();
  await page.getByLabel("Title").fill("Browser E2E");
  await page.getByLabel("Goal").fill("Create one approved artifact and report the evidence.");
  await page.getByRole("button", { name: "Create task" }).click();
  await expect(page.getByRole("heading", { name: "Run timeline" })).toBeVisible();
  await expect(page.getByText("workspace.create")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText("workspace_write · waiting_for_approval")).toBeVisible();

  await page.reload();
  await expect(page.getByText("workspace_write · waiting_for_approval")).toBeVisible();
  await page.getByRole("button", { name: "Approve" }).click();

  await expect(page.getByText("succeeded", { exact: true })).toBeVisible({ timeout: 15_000 });
  const report = page.getByRole("heading", { name: "Final report" }).locator("..");
  await expect(report).toBeVisible();
  await expect(
    report.getByText("The browser-driven E2E task completed successfully.")
  ).toBeVisible();
  await expect(page.getByText("run.reported", { exact: true })).toBeVisible();
});

test("owner reviews and approves an exact managed deployment proposal", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Username").fill("owner");
  await page.getByLabel("Password").fill("correct horse battery staple");
  await page.getByRole("button", { name: "Sign in" }).click();

  await page.getByRole("link", { name: "New task" }).click();
  await page.getByLabel("Title").fill("Managed deployment E2E");
  await page
    .getByLabel("Goal")
    .fill("Complete the managed deployment flow with exact Owner approval.");
  await page.getByRole("button", { name: "Create task" }).click();

  await expect(page.getByText("managed.deploy.apply")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText("privileged · waiting_for_approval")).toBeVisible();
  await expect(page.getByText("Approval preview")).toBeVisible();
  await expect(page.getByText("e2e.example.test")).toBeVisible();
  await expect(page.getByText(/arguments sha256:/)).toBeVisible();

  await page.reload();
  await expect(page.getByText("Approval preview")).toBeVisible();
  await page.getByRole("button", { name: "Approve" }).click();

  await expect(page.getByText("succeeded", { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText("managed_deployment", { exact: true })).toBeVisible();
  const report = page.getByRole("heading", { name: "Final report" }).locator("..");
  await expect(report.getByText("The managed deployment completed successfully.")).toBeVisible();
});
