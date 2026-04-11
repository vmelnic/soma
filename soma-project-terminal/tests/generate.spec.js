// soma-project-terminal — commit 6 LLM-to-PackSpec tests.
//
// Asserts:
//   1. Unauth POST /pack/generate → 401.
//   2. Generate against a fresh context (no chat history) succeeds
//      in fake mode, flips kind to 'active', and stores a valid
//      PackSpec in pack_spec. The stored spec round-trips through
//      the contexts.mjs validator (id string + skills array).
//   3. Generated pack id is derived from the context name (fake
//      mode slugifies the context name into `soma.terminal.<slug>`).
//   4. Generate against a context WITH chat history still works —
//      commit 3's transcript feeds into the pack brain but the
//      fake mode is deterministic, so we only check that the
//      route returned 200 and the pack_spec is non-null.
//   5. Cross-user generate is 404 (same ownership gate as
//      setPackSpec from commit 4).
//   6. Generate against an unknown context id is 404.
//   7. Back-to-back generate produces the same pack id in fake
//      mode (deterministic slug), proving the route is idempotent
//      from the client's perspective.
//   8. UI: clicking the GENERATE PACK button on a fresh context
//      runs the full flow and the runtime panel hot-swaps to the
//      newly-generated pack id.

import { test, expect } from "@playwright/test";
import { loginAs } from "./helpers.mjs";

async function createContext(request, authHeader, name, description) {
  const res = await request.post("/api/contexts", {
    headers: {
      Authorization: authHeader,
      "Content-Type": "application/json",
    },
    data: {
      name,
      description: description ?? `${name} description`,
    },
  });
  expect(res.status()).toBe(201);
  return (await res.json()).context;
}

test.describe("commit 6 — LLM-to-PackSpec", () => {
  test("unauth POST /pack/generate is 401", async ({ request }) => {
    const res = await request.post(
      "/api/contexts/00000000-0000-0000-0000-000000000000/pack/generate",
    );
    expect(res.status()).toBe(401);
  });

  test("generate against a fresh context stores a valid PackSpec", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(
      request,
      authHeader,
      "gen-fresh",
      "a tiny hello pack for testing",
    );

    const res = await request.post(
      `/api/contexts/${ctx.id}/pack/generate`,
      { headers: { Authorization: authHeader } },
    );
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.model).toBe("fake:reasoning");

    // The returned context should be active + have a non-null
    // pack_spec populated.
    expect(body.context.kind).toBe("active");
    expect(body.context.pack_spec).toBeTruthy();

    // pack_spec is TEXT — re-parse and inspect.
    const parsed = JSON.parse(body.context.pack_spec);
    expect(parsed.id).toMatch(/^soma\.terminal\./);
    expect(parsed.name).toBeTruthy();
    expect(Array.isArray(parsed.skills)).toBe(true);
    expect(parsed.skills.length).toBeGreaterThanOrEqual(1);
    expect(parsed.skills[0].capability_requirements).toEqual(
      expect.arrayContaining([expect.stringMatching(/^port:(dom|audio|voice)\//)]),
    );

    // And GET /api/contexts/:id returns the same spec.
    const getRes = await request.get(`/api/contexts/${ctx.id}`, {
      headers: { Authorization: authHeader },
    });
    const getBody = await getRes.json();
    expect(getBody.context.kind).toBe("active");
    expect(JSON.parse(getBody.context.pack_spec).id).toBe(parsed.id);
  });

  test("generated pack id is derived from context name", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(
      request,
      authHeader,
      "daily-journal",
      "private log of daily entries",
    );
    const res = await request.post(
      `/api/contexts/${ctx.id}/pack/generate`,
      { headers: { Authorization: authHeader } },
    );
    expect(res.status()).toBe(200);
    const body = await res.json();
    // Fake mode slugifies "daily-journal" → "daily.journal".
    expect(body.minimal.pack_id).toBe("soma.terminal.daily.journal");
    const parsed = JSON.parse(body.context.pack_spec);
    expect(parsed.id).toBe("soma.terminal.daily.journal");
  });

  test("generate against a context with chat history works", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(
      request,
      authHeader,
      "with-history",
      "test with some chat context",
    );
    // Push a chat turn so the reasoning brain has grounding.
    await request.post(`/api/contexts/${ctx.id}/messages`, {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { content: "I want a pack that greets the operator" },
    });

    const res = await request.post(
      `/api/contexts/${ctx.id}/pack/generate`,
      { headers: { Authorization: authHeader } },
    );
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.context.pack_spec).toBeTruthy();
  });

  test("cross-user generate is 404", async ({ request }) => {
    const alice = await loginAs(request);
    const bob = await loginAs(request);
    const ctx = await createContext(
      request,
      alice.authHeader,
      "alice-gen",
    );
    const res = await request.post(
      `/api/contexts/${ctx.id}/pack/generate`,
      { headers: { Authorization: bob.authHeader } },
    );
    expect(res.status()).toBe(404);

    // Alice's context should still be draft + pack_spec null.
    const check = await request.get(`/api/contexts/${ctx.id}`, {
      headers: { Authorization: alice.authHeader },
    });
    const body = await check.json();
    expect(body.context.kind).toBe("draft");
    expect(body.context.pack_spec).toBeNull();
  });

  test("generate against an unknown context is 404", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.post(
      "/api/contexts/00000000-0000-0000-0000-000000000000/pack/generate",
      { headers: { Authorization: authHeader } },
    );
    expect(res.status()).toBe(404);
  });

  test("back-to-back generate is idempotent (same pack id)", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "idempotent");
    const first = await (
      await request.post(`/api/contexts/${ctx.id}/pack/generate`, {
        headers: { Authorization: authHeader },
      })
    ).json();
    const second = await (
      await request.post(`/api/contexts/${ctx.id}/pack/generate`, {
        headers: { Authorization: authHeader },
      })
    ).json();
    expect(first.minimal.pack_id).toBe("soma.terminal.idempotent");
    expect(second.minimal.pack_id).toBe("soma.terminal.idempotent");
    // And the stored pack_spec is still valid after the second
    // write — setPackSpec's ownership-scoped UPDATE handled the
    // overwrite cleanly.
    expect(JSON.parse(second.context.pack_spec).id).toBe(
      "soma.terminal.idempotent",
    );
  });

  test("UI: clicking GENERATE PACK reboots the runtime to the new pack", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const ctx = await createContext(
      request,
      authHeader,
      "ui-generate",
      "make a pack from the button",
    );

    await context.addCookies([
      {
        name: "soma_session",
        value: sessionToken,
        domain: "127.0.0.1",
        path: "/",
        httpOnly: true,
        sameSite: "Lax",
      },
    ]);
    await page.goto("/");
    await expect(page.locator("#view-authenticated")).toBeVisible({
      timeout: 10_000,
    });

    // Open the fresh context — runtime should boot the hello
    // fallback since pack_spec is null.
    await page
      .locator(`.context-entry[data-context-id='${ctx.id}']`)
      .click();
    await expect(page.locator("#view-context-detail")).toBeVisible();
    const runtimeEl = page.locator("#runtime-summary");
    await expect(runtimeEl).toContainText("READY", { timeout: 15_000 });
    await expect(runtimeEl).toContainText("SOURCE: fallback:hello");

    // Click GENERATE PACK and wait for the runtime panel to
    // hot-swap to the freshly-generated pack.
    await page.click("#btn-generate-pack");
    await expect(page.locator("#generate-status")).toContainText(
      "soma.terminal.ui.generate",
      { timeout: 15_000 },
    );
    await expect(runtimeEl).toContainText("soma.terminal.ui.generate", {
      timeout: 15_000,
    });
    await expect(runtimeEl).toContainText("SOURCE: context");
    // Context kind should flip to ACTIVE in the metadata pre.
    await expect(page.locator("#ctx-kind")).toHaveText("ACTIVE");
  });
});
