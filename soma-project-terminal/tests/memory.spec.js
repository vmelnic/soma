// soma-project-terminal — commit 5 memory isolation tests.
//
// Asserts the per-context memory store is fully isolated:
//   1. Unauth GET / POST / DELETE returns 401.
//   2. A brand-new context has three empty tiers.
//   3. POST appends to each tier; GET returns them in insertion order.
//   4. ISOLATION: appending to ctx A never bleeds into ctx B, even
//      when both belong to the same operator. Then the same check
//      across TWO operators (alice / bob) — the storage layer can't
//      leak, even accidentally.
//   5. One operator cannot read or write another operator's memory
//      (404, same shape as genuinely unknown context).
//   6. Invalid payloads (non-JSON, wrong type) → 400.
//   7. Schemas and routines require a `name` — missing → 400.
//   8. DELETE clears the whole memory for one context without
//      touching sibling contexts.
//   9. UI: the memory panel renders the right counts per context
//      and updates them when the operator switches between
//      contexts (hot-swap proof for the memory panel).

import { test, expect } from "@playwright/test";
import { loginAs } from "./helpers.mjs";

async function createContext(request, authHeader, name) {
  const res = await request.post("/api/contexts", {
    headers: {
      Authorization: authHeader,
      "Content-Type": "application/json",
    },
    data: { name, description: `${name} description` },
  });
  expect(res.status()).toBe(201);
  return (await res.json()).context;
}

async function post(request, authHeader, url, payload) {
  return request.post(url, {
    headers: {
      Authorization: authHeader,
      "Content-Type": "application/json",
    },
    data: payload,
  });
}

// A valid episode shape — the backend only validates that `payload`
// is parseable JSON, so any object will do. These fixtures look
// runtime-ish so the test is readable.
const EPISODE_FIXTURE = {
  goal: "list files in /tmp",
  steps: [{ skill_id: "filesystem.readdir", outcome: "success" }],
};

const SCHEMA_FIXTURE = {
  candidate_skill_ordering: ["filesystem.stat", "filesystem.readdir"],
  confidence: 0.95,
};

const ROUTINE_FIXTURE = {
  name: "list_tmp",
  compiled_skill_path: [
    { skill: "filesystem.stat", bindings: { path: "/tmp" } },
    { skill: "filesystem.readdir", bindings: { path: "/tmp" } },
  ],
  confidence: 0.95,
};

test.describe("commit 5 — per-context memory isolation", () => {
  test("unauth GET /memory is 401", async ({ request }) => {
    const res = await request.get(
      "/api/contexts/00000000-0000-0000-0000-000000000000/memory",
    );
    expect(res.status()).toBe(401);
  });

  test("unauth POST /memory/episodes is 401", async ({ request }) => {
    const res = await request.post(
      "/api/contexts/00000000-0000-0000-0000-000000000000/memory/episodes",
      {
        headers: { "Content-Type": "application/json" },
        data: { payload: EPISODE_FIXTURE },
      },
    );
    expect(res.status()).toBe(401);
  });

  test("fresh context has empty memory", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "fresh-memory");
    const res = await request.get(`/api/contexts/${ctx.id}/memory`, {
      headers: { Authorization: authHeader },
    });
    expect(res.ok()).toBe(true);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.memory).toEqual({
      episodes: [],
      schemas: [],
      routines: [],
    });
  });

  test("POST appends to each tier and GET returns them", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "append-check");

    const epRes = await post(
      request,
      authHeader,
      `/api/contexts/${ctx.id}/memory/episodes`,
      { payload: EPISODE_FIXTURE },
    );
    expect(epRes.status()).toBe(201);
    const epBody = await epRes.json();
    expect(epBody.category).toBe("episodes");
    expect(epBody.row.payload).toBe(JSON.stringify(EPISODE_FIXTURE));

    const schRes = await post(
      request,
      authHeader,
      `/api/contexts/${ctx.id}/memory/schemas`,
      { name: "list_things", payload: SCHEMA_FIXTURE },
    );
    expect(schRes.status()).toBe(201);

    const rtRes = await post(
      request,
      authHeader,
      `/api/contexts/${ctx.id}/memory/routines`,
      { name: "list_tmp", payload: ROUTINE_FIXTURE },
    );
    expect(rtRes.status()).toBe(201);

    const getRes = await request.get(`/api/contexts/${ctx.id}/memory`, {
      headers: { Authorization: authHeader },
    });
    const getBody = await getRes.json();
    expect(getBody.memory.episodes).toHaveLength(1);
    expect(getBody.memory.schemas).toHaveLength(1);
    expect(getBody.memory.routines).toHaveLength(1);
    expect(getBody.memory.schemas[0].name).toBe("list_things");
    expect(getBody.memory.routines[0].name).toBe("list_tmp");
  });

  test("same-operator isolation: writing to A leaves B empty", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctxA = await createContext(request, authHeader, "iso-a");
    const ctxB = await createContext(request, authHeader, "iso-b");

    // Fill A with one of each tier.
    await post(
      request,
      authHeader,
      `/api/contexts/${ctxA.id}/memory/episodes`,
      { payload: EPISODE_FIXTURE },
    );
    await post(
      request,
      authHeader,
      `/api/contexts/${ctxA.id}/memory/schemas`,
      { name: "a-schema", payload: SCHEMA_FIXTURE },
    );
    await post(
      request,
      authHeader,
      `/api/contexts/${ctxA.id}/memory/routines`,
      { name: "a-routine", payload: ROUTINE_FIXTURE },
    );

    // B should still be empty.
    const bRes = await request.get(`/api/contexts/${ctxB.id}/memory`, {
      headers: { Authorization: authHeader },
    });
    const bBody = await bRes.json();
    expect(bBody.memory).toEqual({
      episodes: [],
      schemas: [],
      routines: [],
    });

    // A should have the three rows.
    const aRes = await request.get(`/api/contexts/${ctxA.id}/memory`, {
      headers: { Authorization: authHeader },
    });
    const aBody = await aRes.json();
    expect(aBody.memory.episodes).toHaveLength(1);
    expect(aBody.memory.schemas).toHaveLength(1);
    expect(aBody.memory.routines).toHaveLength(1);
  });

  test("cross-operator isolation: alice's memory is invisible to bob", async ({
    request,
  }) => {
    const alice = await loginAs(request);
    const bob = await loginAs(request);
    const ctx = await createContext(
      request,
      alice.authHeader,
      "alice-memory",
    );
    await post(
      request,
      alice.authHeader,
      `/api/contexts/${ctx.id}/memory/episodes`,
      { payload: EPISODE_FIXTURE },
    );

    // Bob tries to read.
    const bobRead = await request.get(
      `/api/contexts/${ctx.id}/memory`,
      { headers: { Authorization: bob.authHeader } },
    );
    expect(bobRead.status()).toBe(404);

    // Bob tries to write.
    const bobWrite = await post(
      request,
      bob.authHeader,
      `/api/contexts/${ctx.id}/memory/episodes`,
      { payload: { goal: "hijack" } },
    );
    expect(bobWrite.status()).toBe(404);

    // Bob tries to clear.
    const bobClear = await request.delete(
      `/api/contexts/${ctx.id}/memory`,
      { headers: { Authorization: bob.authHeader } },
    );
    expect(bobClear.status()).toBe(404);

    // Alice's memory is untouched.
    const aliceRead = await request.get(
      `/api/contexts/${ctx.id}/memory`,
      { headers: { Authorization: alice.authHeader } },
    );
    const aliceBody = await aliceRead.json();
    expect(aliceBody.memory.episodes).toHaveLength(1);
  });

  test("non-JSON payload is rejected 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "bad-payload");
    const res = await post(
      request,
      authHeader,
      `/api/contexts/${ctx.id}/memory/episodes`,
      { payload: "this is not json {[" },
    );
    expect(res.status()).toBe(400);
  });

  test("missing name on schema is rejected 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "no-name");
    const res = await post(
      request,
      authHeader,
      `/api/contexts/${ctx.id}/memory/schemas`,
      { payload: SCHEMA_FIXTURE },
    );
    expect(res.status()).toBe(400);
  });

  test("missing name on routine is rejected 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "no-name-r");
    const res = await post(
      request,
      authHeader,
      `/api/contexts/${ctx.id}/memory/routines`,
      { payload: ROUTINE_FIXTURE },
    );
    expect(res.status()).toBe(400);
  });

  test("DELETE /memory clears one context without touching siblings", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctxA = await createContext(request, authHeader, "del-a");
    const ctxB = await createContext(request, authHeader, "del-b");

    // Put one episode in each.
    for (const ctx of [ctxA, ctxB]) {
      await post(
        request,
        authHeader,
        `/api/contexts/${ctx.id}/memory/episodes`,
        { payload: EPISODE_FIXTURE },
      );
    }

    // Clear only A.
    const delRes = await request.delete(
      `/api/contexts/${ctxA.id}/memory`,
      { headers: { Authorization: authHeader } },
    );
    expect(delRes.ok()).toBe(true);

    // A is empty, B is not.
    const aRes = await request.get(`/api/contexts/${ctxA.id}/memory`, {
      headers: { Authorization: authHeader },
    });
    expect((await aRes.json()).memory.episodes).toEqual([]);

    const bRes = await request.get(`/api/contexts/${ctxB.id}/memory`, {
      headers: { Authorization: authHeader },
    });
    expect((await bRes.json()).memory.episodes).toHaveLength(1);
  });

  test("memory on unknown context is 404", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.get(
      "/api/contexts/00000000-0000-0000-0000-000000000000/memory",
      { headers: { Authorization: authHeader } },
    );
    expect(res.status()).toBe(404);
  });

  test("UI memory panel reflects per-context counts across hot-swap", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const ctxA = await createContext(request, authHeader, "ui-mem-a");
    const ctxB = await createContext(request, authHeader, "ui-mem-b");

    // Fill A: 2 episodes, 1 schema, 0 routines.
    for (let i = 0; i < 2; i += 1) {
      await post(
        request,
        authHeader,
        `/api/contexts/${ctxA.id}/memory/episodes`,
        { payload: { i } },
      );
    }
    await post(
      request,
      authHeader,
      `/api/contexts/${ctxA.id}/memory/schemas`,
      { name: "a-sch", payload: SCHEMA_FIXTURE },
    );
    // Fill B: 0 episodes, 0 schemas, 1 routine.
    await post(
      request,
      authHeader,
      `/api/contexts/${ctxB.id}/memory/routines`,
      { name: "b-rt", payload: ROUTINE_FIXTURE },
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

    // Open A and check counts.
    await page
      .locator(`.context-entry[data-context-id='${ctxA.id}']`)
      .click();
    await expect(page.locator("#view-context-detail")).toBeVisible();
    const memEl = page.locator("#memory-summary");
    await expect(memEl).toContainText("EPISODES: 2", { timeout: 10_000 });
    await expect(memEl).toContainText("SCHEMAS:  1");
    await expect(memEl).toContainText("ROUTINES: 0");

    // Back to list, open B.
    await page.click("#btn-ctx-back");
    await expect(page.locator("#view-authenticated")).toBeVisible();
    await page
      .locator(`.context-entry[data-context-id='${ctxB.id}']`)
      .click();
    await expect(page.locator("#view-context-detail")).toBeVisible();
    await expect(memEl).toContainText("EPISODES: 0", { timeout: 10_000 });
    await expect(memEl).toContainText("SCHEMAS:  0");
    await expect(memEl).toContainText("ROUTINES: 1");
  });
});
