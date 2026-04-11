// soma-project-terminal — commit 1.1 smoke test.
//
// Asserts:
//   1. The Fallout terminal shell renders with the expected CSS
//      (VT323 font, green-on-black, CRT scanline overlay).
//   2. The login → request-link → verify → authenticated → logout
//      round trip works end-to-end through the REAL backend (which
//      spawns real soma-next and routes through real postgres +
//      smtp + crypto ports).
//   3. Replaying a used magic-link token is rejected with a clear
//      error.
//   4. /api/health reports soma_mcp_ready: true.
//
// Prerequisites documented in README.md — docker services must be
// up, schema applied, binaries copied, .env exists. Playwright's
// webServer auto-starts ./scripts/start-backend.sh.

import { test, expect } from "@playwright/test";
import {
  clearAll,
  waitForMessageTo,
  getPlainBody,
  extractToken,
} from "./mailcatcher.mjs";

async function waitForBootView(page) {
  // The app.mjs boot() function flips from #view-loading to either
  // #view-request-link (unauthenticated) or #view-authenticated.
  await page.waitForFunction(
    () => {
      const loading = document.getElementById("view-loading");
      return loading?.classList.contains("hidden");
    },
    { timeout: 10_000 },
  );
}

test.describe("commit 1 — Fallout terminal + magic-link auth", () => {
  test.beforeEach(async () => {
    // Each test starts with an empty mailbox — previous test emails
    // would poison `waitForMessageTo` otherwise. (DB truncation
    // happens once in globalSetup; for multi-user tests we rely on
    // unique emails per test.)
    await clearAll();
  });

  test("health endpoint reports soma MCP ready", async ({ request }) => {
    const res = await request.get("/api/health");
    expect(res.ok()).toBe(true);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.commit).toBe(5);
    expect(body.soma_mcp_ready).toBe(true);
  });

  test("terminal shell renders with Fallout styling", async ({ page }) => {
    await page.goto("/");
    await waitForBootView(page);

    // The RobCo header + version line should be in the DOM.
    await expect(page.locator(".term-header")).toBeVisible();
    await expect(page.locator(".term-meta")).toContainText(
      "SOMA TERMINAL v0.1",
    );
    await expect(page.locator(".term-meta")).toContainText("RobCo");

    // Request-link view should be active, other views hidden.
    await expect(page.locator("#view-request-link")).toBeVisible();
    await expect(page.locator("#view-link-sent")).toBeHidden();
    await expect(page.locator("#view-authenticated")).toBeHidden();

    // Visual sanity: body is green-on-black monospace. Grab computed
    // styles rather than asserting exact pixel values so minor theme
    // tweaks don't break the test.
    const bodyBg = await page.evaluate(
      () => getComputedStyle(document.body).backgroundColor,
    );
    const bodyColor = await page.evaluate(
      () => getComputedStyle(document.body).color,
    );
    const bodyFont = await page.evaluate(
      () => getComputedStyle(document.body).fontFamily,
    );
    // Dark background (any form of near-black)
    expect(bodyBg).toMatch(/rgb\(0, [0-9]+, 0\)|rgba\(0, [0-9]+, 0/);
    // Green foreground
    expect(bodyColor).toMatch(/rgb\(51, 255, 102\)|rgb\(\d+, 255, \d+\)/);
    // Monospace family including VT323 or fallback
    expect(bodyFont.toLowerCase()).toMatch(/vt323|mono|courier/);

    // CRT scanline overlay should exist.
    await expect(page.locator(".crt-scanlines")).toHaveCount(1);
    await expect(page.locator(".crt-vignette")).toHaveCount(1);

    // Blinking cursor in the footer.
    await expect(page.locator(".term-footer .blink")).toHaveText("█");
  });

  test("full magic-link round trip → authenticated → logout", async ({
    page,
    request,
  }) => {
    const email = `operator-${Date.now()}@somacorp.net`;
    await page.goto("/");
    await waitForBootView(page);

    // --- step 1: submit the email form ---
    await page.fill("#input-email", email);
    await page.click("button[type='submit']");

    // UI should flip to the "link sent" view.
    await expect(page.locator("#view-link-sent")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator("#sent-to")).toHaveText(email);

    // --- step 2: pull the token out of Mailcatcher ---
    const msg = await waitForMessageTo(email);
    const body = await getPlainBody(msg.id);
    expect(body).toContain("SOMA TERMINAL v0.1");
    expect(body).toContain("authorization link");
    const token = extractToken(body);
    expect(token.length).toBeGreaterThanOrEqual(16);

    // --- step 3: follow the verify link via the API ---
    // We use `request` rather than `page.goto` because Playwright's
    // page context doesn't share cookies with the page's browser
    // context we've already been using. Reading the session token
    // from the JSON response and hitting /api/me with Bearer auth is
    // the cleanest way to assert end-to-end auth worked.
    const verifyRes = await request.get(
      `/api/auth/verify?token=${token}`,
      { headers: { Accept: "application/json" } },
    );
    expect(verifyRes.ok()).toBe(true);
    const verifyBody = await verifyRes.json();
    expect(verifyBody.status).toBe("ok");
    expect(verifyBody.user.email).toBe(email);
    expect(verifyBody.session_token).toBeTruthy();
    expect(verifyBody.expires_at).toBeTruthy();

    const sessionToken = verifyBody.session_token;

    // --- step 4: /api/me with the session token ---
    const meRes = await request.get("/api/me", {
      headers: { Authorization: `Bearer ${sessionToken}` },
    });
    expect(meRes.ok()).toBe(true);
    const me = await meRes.json();
    expect(me.status).toBe("ok");
    expect(me.user.email).toBe(email);

    // --- step 5: logout via the API ---
    const logoutRes = await request.post("/api/auth/logout", {
      headers: { Authorization: `Bearer ${sessionToken}` },
    });
    expect(logoutRes.ok()).toBe(true);
    const logoutBody = await logoutRes.json();
    expect(logoutBody.status).toBe("ok");

    // --- step 6: /api/me after logout should be unauthenticated ---
    const meAfter = await request.get("/api/me", {
      headers: { Authorization: `Bearer ${sessionToken}` },
    });
    expect(meAfter.status()).toBe(401);
    const meAfterBody = await meAfter.json();
    expect(meAfterBody.status).toBe("unauthenticated");
  });

  test("replay attack — verifying a used token is rejected", async ({
    request,
  }) => {
    const email = `replay-${Date.now()}@somacorp.net`;

    // Request link
    const reqRes = await request.post("/api/auth/request-link", {
      headers: { "Content-Type": "application/json" },
      data: { email },
    });
    expect(reqRes.ok()).toBe(true);

    const msg = await waitForMessageTo(email);
    const body = await getPlainBody(msg.id);
    const token = extractToken(body);

    // First verify — should succeed.
    const first = await request.get(`/api/auth/verify?token=${token}`, {
      headers: { Accept: "application/json" },
    });
    expect(first.ok()).toBe(true);

    // Second verify with the SAME token — should be rejected.
    const second = await request.get(`/api/auth/verify?token=${token}`, {
      headers: { Accept: "application/json" },
    });
    expect(second.status()).toBe(401);
    const secondBody = await second.json();
    expect(secondBody.status).toBe("error");
    expect(secondBody.error).toMatch(/invalid|expired|used/);
  });

  test("invalid email is rejected before any email is dispatched", async ({
    request,
  }) => {
    const res = await request.post("/api/auth/request-link", {
      headers: { "Content-Type": "application/json" },
      data: { email: "not-an-email" },
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.status).toBe("error");
    expect(body.error).toMatch(/invalid/i);
  });
});
