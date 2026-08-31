import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "apps/web/e2e",
  globalTeardown: "./apps/web/e2e/global-teardown.ts",
  fullyParallel: false,
  retries: 0,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure"
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        ...(process.env.CI ? {} : { channel: "chrome" })
      }
    }
  ],
  webServer: {
    command:
      process.platform === "win32"
        ? "target\\debug\\examples\\e2e_harness.exe"
        : "target/debug/examples/e2e_harness",
    url: "http://127.0.0.1:4173/healthz",
    reuseExistingServer: false,
    timeout: 120_000
  }
});
