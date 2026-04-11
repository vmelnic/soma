// soma-project-terminal — commit 2 contexts tests.
//
// Asserts the context registry works end-to-end through the real
// backend (postgres port + crypto port + smtp port, no shortcuts):
//   1. Unauthenticated /api/contexts is rejected with 401.
//   2. An authenticated operator with no contexts sees an empty list.
//   3. POST /api/contexts creates a row and the list reflects it.
//   4. Multiple contexts persist and come back in recent-first order.
//   5. One operator cannot read or delete another operator's context
//      (returns 404, same shape as genuinely-not-found).
//   6. GET /api/contexts/:id loads a specific row.
//   7. DELETE /api/contexts/:id removes the row.
//   8. Missing / empty name is rejected with 400.
//   9. The authenticated UI renders the contexts list and the
//      create form, and submitting the form adds a visible entry.

import { test, expect } from "@playwright/test";
import { loginAs } from "./helpers.mjs";

test.describe("commit 2 — context registry", () => {
  test("unauthenticated GET /api/contexts is 401", async ({ request }) => {
    const res = await request.get("/api/contexts");
    expect(res.status()).toBe(401);
    const body = await res.json();
    expect(body.status).toBe("unauthenticated");
  });

  test("unauthenticated POST /api/contexts is 401", async ({ request }) => {
    const res = await request.post("/api/contexts", {
      headers: { "Content-Type": "application/json" },
      data: { name: "sneaky" },
    });
    expect(res.status()).toBe(401);
  });

  test("empty list for a brand-new operator", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.get("/api/contexts", {
      headers: { Authorization: authHeader },
    });
    expect(res.ok()).toBe(true);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.contexts).toEqual([]);
  });

  test("create a context and see it in the list", async ({ request }) => {
    const { authHeader } = await loginAs(request);

    const createRes = await request.post("/api/contexts", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: {
        name: "daily-journal",
        description: "a private log of daily entries",
      },
    });
    expect(createRes.status()).toBe(201);
    const created = await createRes.json();
    expect(created.status).toBe("ok");
    expect(created.context.name).toBe("daily-journal");
    expect(created.context.description).toBe("a private log of daily entries");
    expect(created.context.kind).toBe("active");
    expect(created.context.id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
    );

    const listRes = await request.get("/api/contexts", {
      headers: { Authorization: authHeader },
    });
    const list = await listRes.json();
    expect(list.contexts).toHaveLength(1);
    expect(list.contexts[0].name).toBe("daily-journal");
    expect(list.contexts[0].id).toBe(created.context.id);
  });

  test("multiple contexts sort recent-first", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const names = ["alpha", "bravo", "charlie"];
    // Insert with 100ms gaps so updated_at has distinct values even
    // on a container clock with millisecond-only resolution — the
    // covering index's ORDER BY updated_at DESC can still tiebreak
    // on id, but the test wants a semantic "newest first" check.
    for (const name of names) {
      const res = await request.post("/api/contexts", {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { name, description: `${name} desc` },
      });
      expect(res.status()).toBe(201);
      await new Promise((r) => setTimeout(r, 100));
    }
    const listRes = await request.get("/api/contexts", {
      headers: { Authorization: authHeader },
    });
    const list = await listRes.json();
    expect(list.contexts.map((c) => c.name)).toEqual([
      "charlie",
      "bravo",
      "alpha",
    ]);
  });

  test("one operator cannot load another's context", async ({ request }) => {
    const alice = await loginAs(request);
    const bob = await loginAs(request);

    const createRes = await request.post("/api/contexts", {
      headers: {
        Authorization: alice.authHeader,
        "Content-Type": "application/json",
      },
      data: { name: "alice-only", description: "private" },
    });
    const created = await createRes.json();
    const contextId = created.context.id;

    // Alice sees it.
    const aliceRes = await request.get(`/api/contexts/${contextId}`, {
      headers: { Authorization: alice.authHeader },
    });
    expect(aliceRes.status()).toBe(200);
    const aliceBody = await aliceRes.json();
    expect(aliceBody.context.name).toBe("alice-only");

    // Bob does not.
    const bobRes = await request.get(`/api/contexts/${contextId}`, {
      headers: { Authorization: bob.authHeader },
    });
    expect(bobRes.status()).toBe(404);

    // Bob cannot delete it either.
    const bobDel = await request.delete(`/api/contexts/${contextId}`, {
      headers: { Authorization: bob.authHeader },
    });
    expect(bobDel.status()).toBe(404);

    // After bob's attempted delete, alice still sees it.
    const recheck = await request.get(`/api/contexts/${contextId}`, {
      headers: { Authorization: alice.authHeader },
    });
    expect(recheck.status()).toBe(200);
  });

  test("GET /api/contexts/:id returns the row", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const created = await (
      await request.post("/api/contexts", {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { name: "detail-target", description: "load me" },
      })
    ).json();

    const getRes = await request.get(`/api/contexts/${created.context.id}`, {
      headers: { Authorization: authHeader },
    });
    expect(getRes.ok()).toBe(true);
    const body = await getRes.json();
    expect(body.context.id).toBe(created.context.id);
    expect(body.context.name).toBe("detail-target");
    expect(body.context.description).toBe("load me");
    expect(body.context.created_at).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(body.context.updated_at).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });

  test("DELETE /api/contexts/:id removes the row", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const created = await (
      await request.post("/api/contexts", {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { name: "doomed" },
      })
    ).json();

    const delRes = await request.delete(
      `/api/contexts/${created.context.id}`,
      { headers: { Authorization: authHeader } },
    );
    expect(delRes.ok()).toBe(true);

    const getRes = await request.get(`/api/contexts/${created.context.id}`, {
      headers: { Authorization: authHeader },
    });
    expect(getRes.status()).toBe(404);

    const listRes = await request.get("/api/contexts", {
      headers: { Authorization: authHeader },
    });
    const list = await listRes.json();
    expect(list.contexts).toEqual([]);
  });

  test("empty name is rejected with 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.post("/api/contexts", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { name: "   " },
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.status).toBe("error");
    expect(body.error).toMatch(/name/i);
  });

  test("missing name is rejected with 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.post("/api/contexts", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { description: "no name provided" },
    });
    expect(res.status()).toBe(400);
  });

  test("authenticated UI renders the contexts list and create form", async ({
    page,
    context,
    request,
  }) => {
    // Magic-link login via the API, then inject the cookie into the
    // page's browser context so the frontend treats us as logged in.
    const { sessionToken } = await loginAs(request);
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
    // Boot transitions from loading → authenticated once /api/me hits.
    await expect(page.locator("#view-authenticated")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator("#contexts-empty")).toHaveText(
      "NO CONTEXTS FOUND ON THIS OPERATOR PROFILE.",
    );

    // Submit the create form in the UI.
    await page.fill("#input-context-name", "ui-made-context");
    await page.fill("#input-context-description", "via the terminal");
    await page.click("#form-create-context button[type='submit']");

    // The list should flip from empty to one entry.
    const entry = page.locator(".context-entry").first();
    await expect(entry).toBeVisible({ timeout: 10_000 });
    await expect(entry.locator(".ctx-title")).toHaveText("ui-made-context");
    await expect(entry.locator(".ctx-desc")).toHaveText("via the terminal");

    // Clicking the entry loads the detail view.
    await entry.click();
    await expect(page.locator("#view-context-detail")).toBeVisible();
    await expect(page.locator("#ctx-name")).toHaveText("ui-made-context");
    // The detail view no longer shows a "kind" line — the
    // conversation-first architecture has no draft/active split.

    // Back button returns to the authenticated view.
    await page.click("#btn-ctx-back");
    await expect(page.locator("#view-authenticated")).toBeVisible();
  });
});
