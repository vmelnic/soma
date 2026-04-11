// soma-project-terminal — commit 9 skill executor tests.
//
// Asserts the JS-side skill executor ties bridge ports into
// pack-driven execution end-to-end. Scope:
//
//   1. Fake pack expansion produces BOTH wasm and bridge skills —
//      the dom skill has real capability_requirements, the
//      context_kv skill has [] + a bridge:context_kv:set tag.
//   2. pack-level `capabilities` block only lists wasm ports, so
//      the wasm runtime's validator accepts the generated pack
//      (would reject context_kv otherwise).
//   3. UI: generate a pack for a fresh context, SKILLS panel
//      renders two cards, one tagged WASM and one tagged BRIDGE.
//   4. UI: clicking RUN on the bridge skill with { key, value }
//      POSTs to /api/contexts/:id/port/context_kv/set and the
//      written value is immediately visible via the memory / KV
//      list route.
//   5. UI: clicking RUN on the wasm skill calls soma_invoke_port
//      and renders a real <h1> into document.body (the actual
//      side effect of dom.append_heading).
//   6. UI: persistence survives a page reload — store via RUN,
//      refresh the browser tab, open the same context, and the
//      key is still there when the operator lists keys.
//   7. UI: invalid JSON in the skill input textarea is rejected
//      inline without hitting the backend.

import { test, expect } from "@playwright/test";
import { loginAs } from "./helpers.mjs";

async function createContextAndGeneratePack(request, authHeader, name) {
  const created = await request.post("/api/contexts", {
    headers: {
      Authorization: authHeader,
      "Content-Type": "application/json",
    },
    data: { name, description: `${name} description` },
  });
  expect(created.status()).toBe(201);
  const ctx = (await created.json()).context;
  const gen = await request.post(`/api/contexts/${ctx.id}/pack/generate`, {
    headers: { Authorization: authHeader },
  });
  expect(gen.status()).toBe(200);
  const body = await gen.json();
  return { ctx: body.context, genBody: body };
}

async function setSessionCookie(context, sessionToken) {
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
}

test.describe("commit 9 — JS skill executor", () => {
  test("fake pack expansion routes wasm + bridge skills", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const { ctx } = await createContextAndGeneratePack(
      request,
      authHeader,
      "expansion-check",
    );
    const pack = JSON.parse(ctx.pack_spec);

    // Two skills: dom.append_heading (wasm) + context_kv.set (bridge)
    expect(pack.skills.length).toBeGreaterThanOrEqual(2);

    const wasmSkill = pack.skills.find((s) =>
      s.tags.includes("scope:wasm"),
    );
    const bridgeSkill = pack.skills.find((s) =>
      s.tags.includes("scope:bridge"),
    );
    expect(wasmSkill).toBeTruthy();
    expect(bridgeSkill).toBeTruthy();

    // wasm skill has real capability_requirements
    expect(wasmSkill.capability_requirements).toEqual([
      "port:dom/append_heading",
    ]);
    expect(wasmSkill.tags).toEqual(
      expect.arrayContaining(["bridge:dom:append_heading"]),
    );

    // bridge skill has EMPTY capability_requirements + the bridge tag
    expect(bridgeSkill.capability_requirements).toEqual([]);
    expect(bridgeSkill.tags).toEqual(
      expect.arrayContaining(["bridge:context_kv:set"]),
    );

    // Pack-level capabilities block only lists wasm ports —
    // context_kv must NOT appear here or the wasm runtime would
    // reject the whole pack.
    const portIds = pack.capabilities.map((c) => c.group_name);
    expect(portIds).toContain("dom");
    expect(portIds).not.toContain("context_kv");

    // But exposure.local_skills still covers both skills (the
    // generated pack's public surface is complete).
    expect(pack.exposure.local_skills).toContain(wasmSkill.skill_id);
    expect(pack.exposure.local_skills).toContain(bridgeSkill.skill_id);
  });

  test("SKILLS panel renders two scope-tagged cards after generate", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const { ctx } = await createContextAndGeneratePack(
      request,
      authHeader,
      "ui-skills",
    );

    await setSessionCookie(context, sessionToken);
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

    // Two skill cards, tagged WASM and BRIDGE.
    const cards = page.locator(".skill-entry");
    await expect(cards).toHaveCount(2, { timeout: 10_000 });

    const wasmCard = page.locator(
      ".skill-entry:has(.skill-scope.wasm)",
    );
    const bridgeCard = page.locator(
      ".skill-entry:has(.skill-scope.bridge)",
    );
    await expect(wasmCard).toHaveCount(1);
    await expect(bridgeCard).toHaveCount(1);
    await expect(wasmCard.locator(".skill-scope")).toHaveText("WASM");
    await expect(bridgeCard.locator(".skill-scope")).toHaveText("BRIDGE");
  });

  test("running the bridge skill persists a value via /api/port/context_kv/set", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const { ctx } = await createContextAndGeneratePack(
      request,
      authHeader,
      "bridge-run",
    );

    await setSessionCookie(context, sessionToken);
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

    // Find the bridge card and fill its JSON input with a real
    // key/value pair, then click RUN.
    const bridgeCard = page.locator(
      ".skill-entry:has(.skill-scope.bridge)",
    );
    const textarea = bridgeCard.locator("textarea");
    await textarea.fill(
      JSON.stringify({ key: "persistent-note", value: "hello from UI" }),
    );
    await bridgeCard.locator(".btn-run-skill").click();

    // Result panel should show a PortCallRecord-shaped response
    // with port_id: context_kv.
    const resultEl = bridgeCard.locator(".skill-result");
    await expect(resultEl).toContainText('"port_id": "context_kv"', {
      timeout: 10_000,
    });
    await expect(resultEl).toContainText('"capability_id": "set"');
    await expect(resultEl).toContainText("persistent-note");

    // Verify via a direct bridge GET that the key is now there.
    const getRes = await request.post(
      `/api/contexts/${ctx.id}/port/context_kv/get`,
      {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { input: { key: "persistent-note" } },
      },
    );
    const getBody = await getRes.json();
    expect(getBody.record.structured_result.row.value).toBe("hello from UI");
  });

  test("running the wasm skill renders an h1 into document.body", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const { ctx } = await createContextAndGeneratePack(
      request,
      authHeader,
      "wasm-run",
    );

    await setSessionCookie(context, sessionToken);
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

    const wasmCard = page.locator(
      ".skill-entry:has(.skill-scope.wasm)",
    );
    await wasmCard
      .locator("textarea")
      .fill(JSON.stringify({ text: "SOMA-TERMINAL-RENDER-TEST", level: 1 }));
    await wasmCard.locator(".btn-run-skill").click();

    // The result panel should show a PortCallRecord, and the
    // page should now contain an h1 element with the rendered
    // text — that's the actual DOM side effect of
    // dom.append_heading.
    const resultEl = wasmCard.locator(".skill-result");
    await expect(resultEl).toContainText('"port_id": "dom"', {
      timeout: 10_000,
    });
    // append_heading writes to document.body outside the
    // view-context-detail tree, so we locate it globally.
    await expect(
      page.locator("h1", { hasText: "SOMA-TERMINAL-RENDER-TEST" }),
    ).toBeVisible();
  });

  test("bridge-written value survives a page reload", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const { ctx } = await createContextAndGeneratePack(
      request,
      authHeader,
      "reload-persist",
    );

    await setSessionCookie(context, sessionToken);
    await page.goto("/");
    await expect(page.locator("#view-authenticated")).toBeVisible({
      timeout: 10_000,
    });
    await page
      .locator(`.context-entry[data-context-id='${ctx.id}']`)
      .click();
    await expect(page.locator("#view-context-detail")).toBeVisible({
      timeout: 15_000,
    });
    await expect(page.locator("#runtime-summary")).toContainText("READY", {
      timeout: 15_000,
    });

    // Store via the UI.
    const bridgeCard = page.locator(
      ".skill-entry:has(.skill-scope.bridge)",
    );
    await bridgeCard.locator("textarea").fill(
      JSON.stringify({ key: "reload.test", value: "before-reload" }),
    );
    await bridgeCard.locator(".btn-run-skill").click();
    await expect(bridgeCard.locator(".skill-result")).toContainText(
      "reload.test",
      { timeout: 10_000 },
    );

    // Hard reload the page. Session cookie + context pack
    // persist. After reopening the context we run the same
    // bridge card with a get request to confirm the stored
    // value survived.
    await page.reload();
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

    // Read back through the bridge route so we don't need a
    // 'get' skill card (fake pack only emits 'set').
    const getRes = await request.post(
      `/api/contexts/${ctx.id}/port/context_kv/get`,
      {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { input: { key: "reload.test" } },
      },
    );
    const getBody = await getRes.json();
    expect(getBody.record.structured_result.row.value).toBe("before-reload");
  });

  test("invalid JSON in skill input is rejected inline", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const { ctx } = await createContextAndGeneratePack(
      request,
      authHeader,
      "invalid-json",
    );

    await setSessionCookie(context, sessionToken);
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

    const bridgeCard = page.locator(
      ".skill-entry:has(.skill-scope.bridge)",
    );
    await bridgeCard.locator("textarea").fill("{ this is not JSON");
    await bridgeCard.locator(".btn-run-skill").click();

    const resultEl = bridgeCard.locator(".skill-result");
    await expect(resultEl).toContainText("invalid JSON input", {
      timeout: 5_000,
    });
    await expect(resultEl).toHaveClass(/error/);
  });
});
