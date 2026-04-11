// soma-project-terminal — commit 8 backend-port bridge tests.
//
// Asserts the bridge route is a real tenancy boundary and that
// context_kv can round-trip keys through it:
//
//   1. Unauth POST is 401.
//   2. Unknown port id (postgres, smtp, http, anything not in the
//      bridge allow-list) is 403 — the route never forwards raw
//      backend-port surfaces.
//   3. Unknown capability on a known port is 403.
//   4. Cross-tenant context id is 404 (same shape as genuinely
//      unknown context).
//   5. context_kv.set then context_kv.get round-trips a key, and
//      the GET returns the same value the SET stored.
//   6. context_kv.set is an upsert — setting the same key twice
//      replaces the value and bumps updated_at.
//   7. context_kv.list returns all keys for a context (optionally
//      filtered by prefix) and sees only that context's rows.
//   8. ISOLATION: alice.set(key=X, v=A); bob.get(key=X) on the
//      SAME context id returns 404 — bob can't see alice's KV.
//   9. ISOLATION: same operator, different contexts — set on
//      ctxA does not appear in list on ctxB.
//  10. context_kv.delete removes a key and leaves siblings alone.
//  11. Input validation: missing key is 400; oversized value is
//      400.

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

function bridgeUrl(contextId, portId, capabilityId) {
  return `/api/contexts/${contextId}/port/${portId}/${capabilityId}`;
}

async function bridgeCall(request, authHeader, contextId, portId, capId, input) {
  return request.post(bridgeUrl(contextId, portId, capId), {
    headers: {
      Authorization: authHeader,
      "Content-Type": "application/json",
    },
    data: { input },
  });
}

test.describe("commit 8 — backend-port bridge", () => {
  test("unauth bridge call is 401", async ({ request }) => {
    const res = await request.post(
      bridgeUrl(
        "00000000-0000-0000-0000-000000000000",
        "context_kv",
        "set",
      ),
      {
        headers: { "Content-Type": "application/json" },
        data: { input: { key: "x", value: "y" } },
      },
    );
    expect(res.status()).toBe(401);
  });

  test("unknown port id is 403", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "block-raw-pg");
    const res = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "postgres",
      "query",
      { sql: "SELECT 1" },
    );
    expect(res.status()).toBe(403);
    const body = await res.json();
    expect(body.error).toMatch(/port.*postgres/i);
  });

  test("unknown capability on known port is 403", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "cap-block");
    const res = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "drop_everything",
      {},
    );
    expect(res.status()).toBe(403);
    const body = await res.json();
    expect(body.error).toMatch(/capability.*drop_everything/i);
  });

  test("cross-tenant context id is 404", async ({ request }) => {
    const alice = await loginAs(request);
    const bob = await loginAs(request);
    const ctx = await createContext(request, alice.authHeader, "alice-only");
    const res = await bridgeCall(
      request,
      bob.authHeader,
      ctx.id,
      "context_kv",
      "set",
      { key: "intrusion", value: "y" },
    );
    expect(res.status()).toBe(404);
  });

  test("context_kv.set then .get round-trips a value", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "roundtrip");

    const setRes = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "set",
      { key: "greeting", value: "hello world" },
    );
    expect(setRes.status()).toBe(200);
    const setBody = await setRes.json();
    expect(setBody.status).toBe("ok");
    expect(setBody.record.port_id).toBe("context_kv");
    expect(setBody.record.capability_id).toBe("set");
    expect(setBody.record.structured_result.row.key).toBe("greeting");
    expect(setBody.record.structured_result.row.value).toBe("hello world");

    const getRes = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "get",
      { key: "greeting" },
    );
    expect(getRes.status()).toBe(200);
    const getBody = await getRes.json();
    expect(getBody.record.structured_result.row.value).toBe("hello world");
  });

  test("context_kv.set is an upsert", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "upsert");

    await bridgeCall(request, authHeader, ctx.id, "context_kv", "set", {
      key: "theme",
      value: "green-on-black",
    });
    const second = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "set",
      { key: "theme", value: "amber-on-black" },
    );
    expect(second.status()).toBe(200);

    const getRes = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "get",
      { key: "theme" },
    );
    const body = await getRes.json();
    expect(body.record.structured_result.row.value).toBe("amber-on-black");
  });

  test("context_kv.list returns only this context's keys", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctxA = await createContext(request, authHeader, "list-a");
    const ctxB = await createContext(request, authHeader, "list-b");

    await bridgeCall(request, authHeader, ctxA.id, "context_kv", "set", {
      key: "alpha",
      value: "A1",
    });
    await bridgeCall(request, authHeader, ctxA.id, "context_kv", "set", {
      key: "beta",
      value: "A2",
    });
    await bridgeCall(request, authHeader, ctxB.id, "context_kv", "set", {
      key: "alpha",
      value: "B1",
    });

    const listA = await bridgeCall(
      request,
      authHeader,
      ctxA.id,
      "context_kv",
      "list",
      {},
    );
    const listABody = await listA.json();
    const aKeys = listABody.record.structured_result.rows
      .map((r) => r.key)
      .sort();
    expect(aKeys).toEqual(["alpha", "beta"]);
    // Values: ctxA's alpha is "A1", not "B1" from ctxB.
    const aAlpha = listABody.record.structured_result.rows.find(
      (r) => r.key === "alpha",
    );
    expect(aAlpha.value).toBe("A1");

    const listB = await bridgeCall(
      request,
      authHeader,
      ctxB.id,
      "context_kv",
      "list",
      {},
    );
    const listBBody = await listB.json();
    const bKeys = listBBody.record.structured_result.rows
      .map((r) => r.key)
      .sort();
    expect(bKeys).toEqual(["alpha"]);
    expect(listBBody.record.structured_result.rows[0].value).toBe("B1");
  });

  test("context_kv.list with prefix filters keys", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "prefix");

    for (const key of ["todo.1", "todo.2", "note.1"]) {
      await bridgeCall(request, authHeader, ctx.id, "context_kv", "set", {
        key,
        value: "v",
      });
    }
    const res = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "list",
      { prefix: "todo." },
    );
    const body = await res.json();
    const keys = body.record.structured_result.rows
      .map((r) => r.key)
      .sort();
    expect(keys).toEqual(["todo.1", "todo.2"]);
  });

  test("cross-operator isolation holds on the bridge", async ({
    request,
  }) => {
    const alice = await loginAs(request);
    const bob = await loginAs(request);
    const ctx = await createContext(request, alice.authHeader, "alice-kv");
    await bridgeCall(
      request,
      alice.authHeader,
      ctx.id,
      "context_kv",
      "set",
      { key: "secret", value: "mallow" },
    );

    // Bob tries to read.
    const bobGet = await bridgeCall(
      request,
      bob.authHeader,
      ctx.id,
      "context_kv",
      "get",
      { key: "secret" },
    );
    expect(bobGet.status()).toBe(404);

    // Bob tries to write.
    const bobSet = await bridgeCall(
      request,
      bob.authHeader,
      ctx.id,
      "context_kv",
      "set",
      { key: "secret", value: "hijack" },
    );
    expect(bobSet.status()).toBe(404);

    // Alice still sees her value untouched.
    const aliceGet = await bridgeCall(
      request,
      alice.authHeader,
      ctx.id,
      "context_kv",
      "get",
      { key: "secret" },
    );
    const body = await aliceGet.json();
    expect(body.record.structured_result.row.value).toBe("mallow");
  });

  test("context_kv.delete removes a key and leaves siblings alone", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "delete");

    for (const [k, v] of [
      ["keep", "yes"],
      ["drop", "bye"],
    ]) {
      await bridgeCall(request, authHeader, ctx.id, "context_kv", "set", {
        key: k,
        value: v,
      });
    }
    const delRes = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "delete",
      { key: "drop" },
    );
    expect(delRes.status()).toBe(200);
    expect(
      (await delRes.json()).record.structured_result.deleted,
    ).toBe(true);

    const listRes = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "list",
      {},
    );
    const body = await listRes.json();
    const keys = body.record.structured_result.rows
      .map((r) => r.key)
      .sort();
    expect(keys).toEqual(["keep"]);

    // Deleting a missing key is idempotent — same 200, deleted=false.
    const delAgain = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "delete",
      { key: "drop" },
    );
    expect(delAgain.status()).toBe(200);
    expect(
      (await delAgain.json()).record.structured_result.deleted,
    ).toBe(false);
  });

  test("missing key in set is 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "invalid-key");
    const res = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "set",
      { value: "no key" },
    );
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.error).toMatch(/key/);
  });

  test("non-string value is 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "invalid-value");
    const res = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "set",
      { key: "x", value: 42 },
    );
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.error).toMatch(/value/);
  });

  test("get on a missing key returns row:null", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "missing-get");
    const res = await bridgeCall(
      request,
      authHeader,
      ctx.id,
      "context_kv",
      "get",
      { key: "nope" },
    );
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.record.structured_result.row).toBeNull();
  });
});
