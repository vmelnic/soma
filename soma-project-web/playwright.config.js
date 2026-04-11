// Playwright config for soma-project-web browser proofs.
//
// Each phase of the SOMA-in-the-browser work gets a spec file in tests/.
// The test harness uses `webServer` to spin up the local HTTP server
// automatically, so a developer can run `npx playwright test` without
// juggling ./scripts/serve.sh in a second terminal. Chromium is launched
// headless with fake media devices so future phases (voice input, etc.)
// can mock microphones without hitting real hardware.

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
  use: {
    baseURL: "http://localhost:8765",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    // Permit the browser to use fake microphone / camera so voice-input
    // tests can run without a real device. The fake devices are driven
    // from a synthesized silence stream by default; tests that need real
    // audio content pass `--use-file-for-fake-audio-capture=FILE.wav`.
    launchOptions: {
      args: [
        "--use-fake-ui-for-media-stream",
        "--use-fake-device-for-media-stream",
      ],
    },
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    // Serve the project root (index.html + pkg/) on port 8765.
    // reuseExistingServer lets a developer leave ./scripts/serve.sh
    // running in another terminal without Playwright fighting it.
    command: "python3 -m http.server 8765",
    url: "http://localhost:8765/index.html",
    reuseExistingServer: true,
    timeout: 10_000,
  },
});
