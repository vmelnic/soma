// Phase 1d proof — soma-next Runtime boots in the browser.
//
// Asserts the same things a human would verify by clicking:
//   1. The wasm module loads and logs its boot banner to the console.
//   2. `soma_boot_runtime("")` is called during page init and returns
//      a summary listing the three in-tab ports (dom, audio, voice)
//      with their capabilities.
//   3. Clicking `append_heading` invokes the port through the real
//      DefaultPortRuntime pipeline — the PortCallRecord must have the
//      gate fields populated (auth_result, policy_result, sandbox_result)
//      which earlier phases left as null.
//   4. The actual <h1 data-soma="true">hello marcu</h1> is present in
//      the DOM after the click.

import { test, expect } from "@playwright/test";

test.describe("phase 1d — Runtime in the browser", () => {
  test.beforeEach(async ({ page }) => {
    // Route console messages to the test output so a failing run shows
    // exactly what the wasm module logged before the failure.
    page.on("console", (msg) => {
      const text = msg.text();
      if (text.startsWith("[soma-next") || text.startsWith("[voice]")) {
        console.log(`    > ${text}`);
      }
    });
    page.on("pageerror", (err) => {
      console.error(`    > page error: ${err.message}`);
    });

    await page.goto("/index.html");
  });

  test("wasm boot banner appears in console", async ({ page }) => {
    const messages = [];
    page.on("console", (msg) => messages.push(msg.text()));
    await page.reload();
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes("booted"),
      { timeout: 10_000 },
    );
    const joined = messages.join("\n");
    expect(joined).toContain("[soma-next wasm] boot");
    expect(joined).toContain("Runtime booted with ports: dom, audio, voice");
  });

  test("soma_boot_runtime returns a valid summary", async ({ page }) => {
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes('"booted": true'),
      { timeout: 10_000 },
    );
    // The harness auto-boots with the phase-1e hello pack, so the first
    // summary has pack_count 1. Re-boot with an empty manifest to test
    // the zero-pack case explicitly.
    await page.click("#btn-boot-empty");
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes('"pack_count": 0'),
      { timeout: 5_000 },
    );
    const record = await page.locator("#record").textContent();
    const summary = JSON.parse(record);
    expect(summary.booted).toBe(true);
    expect(summary.pack_count).toBe(0);
    expect(summary.ports).toHaveLength(3);

    const byId = Object.fromEntries(
      summary.ports.map((p) => [p.port_id, p]),
    );
    expect(byId.dom).toBeDefined();
    expect(byId.dom.kind).toBe("Renderer");
    expect(byId.dom.capabilities).toEqual(
      expect.arrayContaining([
        "append_heading",
        "append_paragraph",
        "set_title",
        "clear_soma",
      ]),
    );
    expect(byId.audio).toBeDefined();
    expect(byId.audio.kind).toBe("Actuator");
    expect(byId.audio.capabilities).toContain("say_text");
    expect(byId.voice).toBeDefined();
    expect(byId.voice.kind).toBe("Sensor");
    expect(byId.voice.capabilities).toEqual(
      expect.arrayContaining([
        "start_listening",
        "stop_listening",
        "get_last_transcript",
        "get_all_transcripts",
        "clear_transcripts",
      ]),
    );
  });

  test("soma_list_ports returns the three in-tab ports", async ({ page }) => {
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes('"booted"'),
      { timeout: 10_000 },
    );
    await page.click("#btn-ports");
    await page.waitForFunction(() => {
      const text = document.getElementById("record")?.textContent || "";
      return text.startsWith("[") && text.includes("dom");
    });
    const record = await page.locator("#record").textContent();
    const ids = JSON.parse(record);
    expect(ids.sort()).toEqual(["audio", "dom", "voice"]);
  });

  test("append_heading invokes through the real runtime pipeline", async ({
    page,
  }) => {
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes('"booted"'),
      { timeout: 10_000 },
    );

    await page.fill("#heading-text", "hello marcu");
    await page.fill("#heading-level", "1");
    await page.click("#btn-append-heading");

    // The PortCallRecord pane should now contain the observation.
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes("append_heading"),
      { timeout: 5_000 },
    );

    const record = await page.locator("#record").textContent();
    const parsed = JSON.parse(record);

    expect(parsed.success).toBe(true);
    expect(parsed.port_id).toBe("dom");
    expect(parsed.capability_id).toBe("append_heading");
    expect(parsed.structured_result).toMatchObject({
      rendered: true,
      tag: "h1",
      text: "hello marcu",
      level: 1,
    });
    expect(parsed.side_effect_summary).toBe("dom_append");

    // Phase 1d's defining feature: invocations now go through the real
    // DefaultPortRuntime, which populates all the gate fields. Earlier
    // phases left these as null because the thread_local HashMap skipped
    // the runtime pipeline.
    expect(parsed.auth_result).not.toBeNull();
    expect(parsed.policy_result).not.toBeNull();
    expect(parsed.sandbox_result).not.toBeNull();
    expect(parsed.auth_result.status).toBe("not_required");
    expect(parsed.policy_result.status).toBe("allowed");
    expect(parsed.sandbox_result.status).toBe("satisfied");

    // The actual DOM mutation must be visible.
    const h1 = page.locator('h1[data-soma="true"]').filter({ hasText: "hello marcu" });
    await expect(h1).toBeVisible();
  });

  test("append_paragraph + set_title + clear_soma", async ({ page }) => {
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes('"booted"'),
      { timeout: 10_000 },
    );

    // append two things, then set title, then clear.
    await page.fill("#heading-text", "first heading");
    await page.click("#btn-append-heading");
    await expect(
      page.locator('h1[data-soma="true"]').filter({ hasText: "first heading" }),
    ).toBeVisible();

    await page.fill("#paragraph-text", "a paragraph from the dom port");
    await page.click("#btn-append-paragraph");
    await expect(
      page
        .locator('p[data-soma="true"]')
        .filter({ hasText: "a paragraph from the dom port" }),
    ).toBeVisible();

    await page.fill("#title-text", "soma phase 1d");
    await page.click("#btn-set-title");
    await expect(page).toHaveTitle("soma phase 1d");

    await page.click("#btn-clear");
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes("clear_soma"),
    );
    const recordText = await page.locator("#record").textContent();
    const cleared = JSON.parse(recordText);
    expect(cleared.success).toBe(true);
    expect(cleared.structured_result.removed_count).toBeGreaterThanOrEqual(2);

    // After clear_soma the soma-rendered elements should be gone.
    await expect(page.locator('h1[data-soma="true"]')).toHaveCount(0);
    await expect(page.locator('p[data-soma="true"]')).toHaveCount(0);
  });

  test("audio.say_text runs through the runtime", async ({ page }) => {
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes('"booted"'),
      { timeout: 10_000 },
    );

    await page.fill("#say-text", "hello marcu");
    await page.click("#btn-say-text");
    await page.waitForFunction(
      () =>
        document
          .getElementById("record")
          ?.textContent?.includes("audio_speak"),
      { timeout: 5_000 },
    );
    const record = await page.locator("#record").textContent();
    const parsed = JSON.parse(record);
    expect(parsed.port_id).toBe("audio");
    expect(parsed.capability_id).toBe("say_text");
    expect(parsed.success).toBe(true);
    expect(parsed.structured_result.spoken).toBe("hello marcu");
    expect(parsed.sandbox_result.status).toBe("satisfied");
  });
});
