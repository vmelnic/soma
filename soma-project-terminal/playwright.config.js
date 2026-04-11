// Playwright config for soma-project-terminal.
//
// Prerequisites before `npx playwright test` can run:
//   1. Docker stack is up:  ./scripts/start.sh
//   2. Schema applied:      ./scripts/setup-db.sh
//   3. Binaries copied:     ./scripts/copy-binaries.sh
//   4. .env exists:         cp .env.example .env
//
// Playwright's webServer auto-starts the backend (which spawns
// soma-next). `globalSetup` truncates the tables + clears
// Mailcatcher before any test runs so tests start from a clean
// slate. Tests use unique emails so they don't stomp on each
// other if run in parallel.

import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  timeout: 30_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: [["list"], ["html", { open: "never", outputFolder: "playwright-report" }]],

  globalSetup: "./tests/global-setup.mjs",

  use: {
    baseURL: "http://127.0.0.1:8765",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],

  webServer: {
    command: "./scripts/start-backend.sh",
    url: "http://127.0.0.1:8765/api/health",
    reuseExistingServer: true,
    timeout: 30_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
