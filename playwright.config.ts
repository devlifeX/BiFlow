import { defineConfig, devices } from "@playwright/test";

// The execution environment can set both variables; Node warns before every
// web-server and worker process unless one is removed before Playwright forks.
delete process.env.NO_COLOR;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:1420",
    trace: "on-first-retry",
    viewport: { width: 1120, height: 760 },
    ...(process.env.BIFLOW_CHROMIUM_PATH
      ? {
          launchOptions: {
            executablePath: process.env.BIFLOW_CHROMIUM_PATH,
          },
        }
      : {}),
  },
  webServer: {
    command: "pnpm --dir apps/desktop dev --host 127.0.0.1",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1120, height: 760 },
      },
    },
  ],
});
