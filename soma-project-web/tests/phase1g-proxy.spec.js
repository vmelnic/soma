// Phase 1g end-to-end proof — real brain proxy + real HTTP.
//
// The phase 1g.spec.js tests use `page.route` to mock the brain
// endpoint so the core harness behavior is hermetic. This file does
// the complementary half: it launches the actual Node brain proxy in
// --fake mode, tells the browser to hit http://localhost:8787/api/brain,
// and asserts the full round trip works — browser → real HTTP → real
// Node server → real HTTP response → browser → wasm port invocation.
//
// Running in --fake mode means these tests don't need OPENAI_API_KEY
// to pass in CI. To exercise the REAL OpenAI path, run the proxy
// yourself with `scripts/brain-proxy.mjs` (no --fake), set the
// "Brain endpoint" in the browser UI, and click "compose & run".
// The real-LLM path follows the exact same wire contract.

import { test, expect } from "@playwright/test";
import { spawn } from "child_process";

// Playwright's `webServer.cwd` defaults to the directory containing
// `playwright.config.js`, which is the project root — so relative
// paths like "scripts/brain-proxy.mjs" resolve correctly when
// spawned from test code too. Avoiding `import path from "path"`
// here because Playwright's transformer rewrites CJS default imports
// into `require()` calls that fail in an ESM spec file context.

const PROXY_PORT = 8787;
const PROXY_URL = `http://127.0.0.1:${PROXY_PORT}/api/brain`;

let proxyProcess = null;

async function startProxy() {
  return new Promise((resolve, reject) => {
    const proc = spawn(
      "node",
      ["scripts/brain-proxy.mjs", "--fake", "--port", String(PROXY_PORT)],
      {
        // Playwright test runner cwd is the project root.
        stdio: ["ignore", "pipe", "pipe"],
      },
    );

    let ready = false;
    const onData = (chunk) => {
      const text = chunk.toString();
      if (text.includes("listening on")) {
        ready = true;
        resolve(proc);
      }
    };
    proc.stderr.on("data", onData);
    proc.stdout.on("data", onData);
    proc.on("exit", (code) => {
      if (!ready) reject(new Error(`proxy exited early with code ${code}`));
    });
    setTimeout(() => {
      if (!ready) reject(new Error("proxy failed to report 'listening' in 3s"));
    }, 3000).unref();
  });
}

test.describe("phase 1g — real brain proxy (fake mode)", () => {
  test.beforeAll(async () => {
    proxyProcess = await startProxy();
  });

  test.afterAll(async () => {
    if (proxyProcess) {
      proxyProcess.kill("SIGTERM");
      await new Promise((resolve) =>
        proxyProcess.once("exit", () => resolve()),
      );
    }
  });

  test.beforeEach(async ({ page }) => {
    page.on("console", (msg) => {
      const text = msg.text();
      if (text.startsWith("[soma-next")) {
        console.log(`    > ${text}`);
      }
    });
    page.on("pageerror", (err) => {
      console.error(`    > page error: ${err.message}`);
    });

    await page.goto("/index.html");
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes('"booted": true'),
      { timeout: 15_000 },
    );

    // Point the brain input at the real proxy instead of /api/brain.
    await page.fill("#brain-endpoint", PROXY_URL);
    await page.locator("#brain-endpoint").blur();
  });

  test("real proxy round trip — fake plan renders h1 and paragraph", async ({
    page,
  }) => {
    await page.fill("#brain-prompt", "hello from playwright");
    await page.click("#btn-brain-run");

    // Wait for the record pane to reflect the proxy's response.
    await page.waitForFunction(
      () => {
        const text = document.getElementById("record")?.textContent || "";
        return text.includes("Fake brain");
      },
      { timeout: 10_000 },
    );

    const record = await page.locator("#record").textContent();
    const parsed = JSON.parse(record);
    expect(parsed.explanation).toContain("Fake brain");
    expect(parsed.plan).toHaveLength(2);
    expect(parsed.plan[0].port_id).toBe("dom");
    expect(parsed.plan[0].capability_id).toBe("append_heading");
    expect(parsed.plan[0].input.text).toContain("hello from playwright");
    expect(parsed.plan[1].port_id).toBe("audio");
    expect(parsed.plan[1].capability_id).toBe("say_text");

    // The real DOM should have the fake brain's heading.
    const heading = page
      .locator('h1[data-soma="true"]')
      .filter({ hasText: "(fake brain) hello from playwright" });
    await expect(heading).toBeVisible();

    // Brain log should show both steps completed.
    const log = await page.locator("#brain-log").textContent();
    expect(log).toContain("[1] dom.append_heading");
    expect(log).toContain("[2] audio.say_text");
    expect(log).toContain("✓ dom_append");
    expect(log).toContain("✓ audio_speak");
  });

  test("proxy receives the current port catalog", async ({ page }) => {
    // Different prompts produce different heading text — pick one
    // that's unambiguous so we can grep for it.
    await page.fill("#brain-prompt", "catalog test marker 4242");
    await page.click("#btn-brain-run");

    await page.waitForFunction(
      () => {
        const text = document.getElementById("record")?.textContent || "";
        return text.includes("catalog test marker 4242");
      },
      { timeout: 10_000 },
    );

    // The fake proxy echoes the prompt back in the heading text.
    // If it got here, the browser successfully POSTed JSON with the
    // port catalog to the proxy and the proxy responded with a plan.
    const heading = page
      .locator('h1[data-soma="true"]')
      .filter({ hasText: "catalog test marker 4242" });
    await expect(heading).toBeVisible();
  });
});
