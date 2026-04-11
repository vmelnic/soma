// soma-project-terminal — commit 4 dynamic pack loading tests.
//
// Asserts:
//   1. A fresh context has pack_spec = null.
//   2. PUT /api/contexts/:id/pack stores a manifest and flips kind
//      from 'draft' to 'active'. GET returns the stored manifest.
//   3. PUT with a bare (non-wrapped) manifest also works — the
//      route accepts both `{pack: {...}}` and just `{...}`.
//   4. PUT with invalid JSON or a non-object body is rejected 400.
//   5. PUT with a manifest missing `id` or `skills` is rejected 400.
//   6. One operator cannot set another operator's pack (404).
//   7. Unauth PUT is 401.
//   8. UI hot-swap: create two contexts, set a distinct manifest on
//      each, open one then the other, and the runtime panel shows
//      a different pack id each time — proving the wasm runtime
//      actually hot-swaps, not just displays stale metadata.

import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, resolve as resolvePath } from "node:path";
import { loginAs } from "./helpers.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const HELLO_PATH = resolvePath(
  __dirname,
  "..",
  "frontend",
  "packs",
  "hello",
  "manifest.json",
);

// Read the real hello manifest and clone it into a new pack with a
// different id/name so the wasm runtime will accept it (same shape,
// valid fields, just renamed).
async function buildAltManifest(newId, newName) {
  const raw = await readFile(HELLO_PATH, "utf8");
  const parsed = JSON.parse(raw);
  parsed.id = newId;
  parsed.name = newName;
  parsed.namespace = newId;
  // Rewrite every skill's pack/namespace so they match the new pack
  // id, otherwise the runtime rejects the manifest with "skill X
  // does not belong to pack Y".
  if (Array.isArray(parsed.skills)) {
    parsed.skills = parsed.skills.map((s) => ({
      ...s,
      pack: newId,
      namespace: newId,
      skill_id: `${newId}.${(s.skill_id ?? "skill").split(".").pop()}`,
    }));
  }
  if (parsed.exposure?.local_skills) {
    parsed.exposure.local_skills = parsed.skills.map((s) => s.skill_id);
  }
  return parsed;
}

async function createContext(request, authHeader, name) {
  const res = await request.post("/api/contexts", {
    headers: {
      Authorization: authHeader,
      "Content-Type": "application/json",
    },
    data: { name, description: `${name} description` },
  });
  expect(res.status()).toBe(201);
  const body = await res.json();
  return body.context;
}

test.describe("commit 4 — dynamic pack loading", () => {
  test("fresh context has pack_spec null and kind draft", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "fresh-pack");
    const res = await request.get(`/api/contexts/${ctx.id}`, {
      headers: { Authorization: authHeader },
    });
    const body = await res.json();
    expect(body.context.pack_spec).toBeNull();
    expect(body.context.kind).toBe("draft");
  });

  test("PUT stores a manifest and flips kind to active", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "pack-set");
    const manifest = await buildAltManifest(
      "soma.terminal.alpha",
      "Alpha Pack",
    );

    const putRes = await request.put(`/api/contexts/${ctx.id}/pack`, {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { pack: manifest },
    });
    expect(putRes.status()).toBe(200);
    const putBody = await putRes.json();
    expect(putBody.status).toBe("ok");
    expect(putBody.context.kind).toBe("active");
    expect(putBody.context.pack_spec).toBeTruthy();

    // The round-trip is via TEXT storage — re-parse and check.
    const reloaded = JSON.parse(putBody.context.pack_spec);
    expect(reloaded.id).toBe("soma.terminal.alpha");
    expect(reloaded.name).toBe("Alpha Pack");

    // GET should return the same thing.
    const getRes = await request.get(`/api/contexts/${ctx.id}`, {
      headers: { Authorization: authHeader },
    });
    const getBody = await getRes.json();
    expect(getBody.context.kind).toBe("active");
    const fromGet = JSON.parse(getBody.context.pack_spec);
    expect(fromGet.id).toBe("soma.terminal.alpha");
  });

  test("PUT accepts a bare manifest body (no `pack` wrapper)", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "pack-bare");
    const manifest = await buildAltManifest(
      "soma.terminal.bare",
      "Bare Pack",
    );
    const putRes = await request.put(`/api/contexts/${ctx.id}/pack`, {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: manifest,
    });
    expect(putRes.status()).toBe(200);
    const body = await putRes.json();
    expect(JSON.parse(body.context.pack_spec).id).toBe("soma.terminal.bare");
  });

  test("PUT with non-object body is rejected 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "pack-bad");
    const res = await request.put(`/api/contexts/${ctx.id}/pack`, {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { pack: "not-an-object" },
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.status).toBe("error");
  });

  test("PUT with missing id is rejected 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "pack-no-id");
    const res = await request.put(`/api/contexts/${ctx.id}/pack`, {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { pack: { skills: [] } },
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.error).toMatch(/id/i);
  });

  test("PUT with non-array skills is rejected 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "pack-no-skills");
    const res = await request.put(`/api/contexts/${ctx.id}/pack`, {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { pack: { id: "soma.terminal.broken", skills: "not-array" } },
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.error).toMatch(/skills/i);
  });

  test("one operator cannot set another's pack", async ({ request }) => {
    const alice = await loginAs(request);
    const bob = await loginAs(request);
    const ctx = await createContext(
      request,
      alice.authHeader,
      "alice-pack",
    );
    const manifest = await buildAltManifest(
      "soma.terminal.intrusion",
      "Intrusion",
    );
    const res = await request.put(`/api/contexts/${ctx.id}/pack`, {
      headers: {
        Authorization: bob.authHeader,
        "Content-Type": "application/json",
      },
      data: { pack: manifest },
    });
    expect(res.status()).toBe(404);

    // Alice's context should still have pack_spec null.
    const check = await request.get(`/api/contexts/${ctx.id}`, {
      headers: { Authorization: alice.authHeader },
    });
    const body = await check.json();
    expect(body.context.pack_spec).toBeNull();
    expect(body.context.kind).toBe("draft");
  });

  test("unauth PUT is 401", async ({ request }) => {
    const res = await request.put(
      "/api/contexts/00000000-0000-0000-0000-000000000000/pack",
      {
        headers: { "Content-Type": "application/json" },
        data: { pack: { id: "x", skills: [] } },
      },
    );
    expect(res.status()).toBe(401);
  });

  test("UI hot-swap: two contexts, two packs, runtime reboots", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);

    // Create two contexts and put a distinct pack on each.
    const ctxAlpha = await createContext(request, authHeader, "ctx-alpha");
    const ctxBeta = await createContext(request, authHeader, "ctx-beta");
    const alphaManifest = await buildAltManifest(
      "soma.terminal.alpha",
      "Alpha Pack",
    );
    const betaManifest = await buildAltManifest(
      "soma.terminal.beta",
      "Beta Pack",
    );
    for (const [ctx, manifest] of [
      [ctxAlpha, alphaManifest],
      [ctxBeta, betaManifest],
    ]) {
      const put = await request.put(`/api/contexts/${ctx.id}/pack`, {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { pack: manifest },
      });
      expect(put.status()).toBe(200);
    }

    // Inject the session cookie so the browser treats us as logged in.
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

    // Open ctx-alpha first and assert the runtime summary shows it.
    const alphaEntry = page.locator(
      `.context-entry[data-context-id='${ctxAlpha.id}']`,
    );
    await expect(alphaEntry).toBeVisible();
    await alphaEntry.click();
    await expect(page.locator("#view-context-detail")).toBeVisible();
    await expect(page.locator("#runtime-summary")).toContainText("READY", {
      timeout: 15_000,
    });
    await expect(page.locator("#runtime-summary")).toContainText(
      "soma.terminal.alpha",
    );
    await expect(page.locator("#runtime-summary")).toContainText(
      "SOURCE: context",
    );

    // Back to the list, open ctx-beta, and the runtime should hot-
    // swap to the other pack.
    await page.click("#btn-ctx-back");
    await expect(page.locator("#view-authenticated")).toBeVisible();
    const betaEntry = page.locator(
      `.context-entry[data-context-id='${ctxBeta.id}']`,
    );
    await betaEntry.click();
    await expect(page.locator("#view-context-detail")).toBeVisible();
    await expect(page.locator("#runtime-summary")).toContainText("READY", {
      timeout: 15_000,
    });
    await expect(page.locator("#runtime-summary")).toContainText(
      "soma.terminal.beta",
    );
    await expect(page.locator("#runtime-summary")).not.toContainText(
      "soma.terminal.alpha",
    );
  });

  test("UI falls back to hello pack when context has no pack_spec", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "no-pack");
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
    await page
      .locator(`.context-entry[data-context-id='${ctx.id}']`)
      .click();
    await expect(page.locator("#view-context-detail")).toBeVisible();
    await expect(page.locator("#runtime-summary")).toContainText("READY", {
      timeout: 15_000,
    });
    await expect(page.locator("#runtime-summary")).toContainText(
      "SOURCE: fallback:hello",
    );
  });
});
