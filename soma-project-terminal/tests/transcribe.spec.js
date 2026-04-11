// soma-project-terminal — commit 7 voice input tests.
//
// Asserts:
//   1. Unauth POST /api/transcribe → 401.
//   2. Empty body → 400.
//   3. application/json content-type → 400 (guard against
//      accidental form posts).
//   4. Raw audio bytes + audio/* content-type → 200 with the fake
//      transcription string (BRAIN_FAKE=1 makes this hermetic).
//   5. UI: clicking [ MIC ] twice runs the full click →
//      MediaRecorder stub → /api/transcribe → fill #input-chat
//      flow and leaves the operator able to submit the dictated
//      text via the existing chat form. The MediaRecorder +
//      getUserMedia APIs are stubbed via page.addInitScript
//      because Playwright can't feed a real microphone.
//
// Real-mode Whisper is NOT tested here — that would burn API
// quota on every CI run. Fake mode exercises the full
// request/response wire with deterministic output.

import { test, expect } from "@playwright/test";
import { loginAs } from "./helpers.mjs";

test.describe("commit 7 — voice input", () => {
  test("unauth POST /api/transcribe is 401", async ({ request }) => {
    const res = await request.post("/api/transcribe", {
      headers: { "Content-Type": "audio/webm" },
      data: Buffer.from("some bytes"),
    });
    expect(res.status()).toBe(401);
  });

  test("empty audio body is 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.post("/api/transcribe", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "audio/webm",
      },
      data: Buffer.alloc(0),
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.error).toMatch(/empty/i);
  });

  test("application/json content-type is 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.post("/api/transcribe", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: Buffer.from(JSON.stringify({ text: "no" })),
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.error).toMatch(/audio/i);
  });

  test("valid audio bytes return fake transcript", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const fakeAudio = Buffer.from(
      "NOT REAL AUDIO — fake mode ignores content, only byte count",
    );
    const res = await request.post("/api/transcribe", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "audio/webm",
      },
      data: fakeAudio,
    });
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.model).toBe("fake:whisper");
    expect(body.text).toContain("[FAKE TRANSCRIBE]");
    expect(body.text).toContain(`${fakeAudio.length} bytes received`);
    expect(body.text).toContain("audio/webm");
  });

  test("UI: mic button records, transcribes, fills chat input", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    // Create a context so the mic button is actually rendered
    // (it only exists inside the context-detail view).
    const ctxRes = await request.post("/api/contexts", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: {
        name: "voice-test",
        description: "testing the mic button",
      },
    });
    const ctx = (await ctxRes.json()).context;

    // Stub MediaRecorder + getUserMedia BEFORE any page JS runs.
    // The real APIs need a microphone + codec pipeline we can't
    // feed from Playwright — the stub returns a canned blob on
    // stop() so the full click → /api/transcribe → fill path
    // still runs end-to-end.
    await page.addInitScript(() => {
      // Replace navigator.mediaDevices.getUserMedia with a stub
      // that hands back a fake stream whose tracks can be
      // stopped. The real app only cares about getTracks().
      Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: {
          getUserMedia: async () => ({
            getTracks: () => [{ stop: () => {} }],
          }),
        },
      });
      // Replace MediaRecorder with a tiny state machine that
      // emits a canned blob on stop(). Keeps the same event
      // shape the real implementation uses.
      class FakeMediaRecorder {
        constructor(stream) {
          this.stream = stream;
          this.mimeType = "audio/webm";
          this.state = "inactive";
          this.ondataavailable = null;
          this.onstop = null;
        }
        start() {
          this.state = "recording";
        }
        stop() {
          this.state = "inactive";
          const blob = new Blob(["fake-ui-audio-bytes"], {
            type: "audio/webm",
          });
          if (this.ondataavailable) {
            this.ondataavailable({ data: blob });
          }
          if (this.onstop) this.onstop();
        }
      }
      window.MediaRecorder = FakeMediaRecorder;
    });

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

    const micBtn = page.locator("#btn-mic");
    await expect(micBtn).toHaveText("[ MIC ]");

    // First click — starts recording. Button should flip into
    // the "recording" state with the STOP label.
    await micBtn.click();
    await expect(micBtn).toHaveText("[ STOP ]");
    await expect(micBtn).toHaveClass(/recording/);

    // Second click — stops, uploads, fills the input. The fake
    // brain echoes the byte count, so we just assert the input
    // contains the FAKE TRANSCRIBE marker.
    await micBtn.click();
    const inputEl = page.locator("#input-chat");
    await expect(inputEl).toHaveValue(/FAKE TRANSCRIBE/, {
      timeout: 10_000,
    });
    await expect(micBtn).toHaveText("[ MIC ]");
    await expect(micBtn).not.toHaveClass(/recording/);
  });

  test("UI: mic appends to existing draft instead of clobbering it", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const ctxRes = await request.post("/api/contexts", {
      headers: {
        Authorization: authHeader,
        "Content-Type": "application/json",
      },
      data: { name: "voice-append" },
    });
    const ctx = (await ctxRes.json()).context;

    await page.addInitScript(() => {
      Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: {
          getUserMedia: async () => ({
            getTracks: () => [{ stop: () => {} }],
          }),
        },
      });
      class FakeMediaRecorder {
        constructor() {
          this.mimeType = "audio/webm";
          this.state = "inactive";
        }
        start() {
          this.state = "recording";
        }
        stop() {
          this.state = "inactive";
          if (this.ondataavailable) {
            this.ondataavailable({
              data: new Blob(["xyz"], { type: "audio/webm" }),
            });
          }
          if (this.onstop) this.onstop();
        }
      }
      window.MediaRecorder = FakeMediaRecorder;
    });

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

    // Prefill the input so we can assert append-vs-clobber.
    const inputEl = page.locator("#input-chat");
    await inputEl.fill("i want to build a");

    await page.click("#btn-mic");
    await page.click("#btn-mic");
    await expect(inputEl).toHaveValue(/^i want to build a .*FAKE TRANSCRIBE/, {
      timeout: 10_000,
    });
  });
});
