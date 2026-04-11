// soma-project-terminal — commit 3 chat + brain tests.
//
// Asserts:
//   1. Unauthenticated access to /messages is 401.
//   2. Empty transcript for a brand-new context.
//   3. POST /messages stores the user turn, calls the brain, stores
//      the assistant turn, and returns both in one response. The
//      fake brain is used (BRAIN_FAKE=1 via playwright.config.js)
//      so the test is hermetic.
//   4. GET /messages returns the full transcript in insertion order.
//   5. One operator cannot read another operator's transcript
//      (404, same shape as unknown context).
//   6. Posting an empty message is rejected with 400.
//   7. Posting to a non-existent context is 404.
//   8. The UI chat shell paints user + brain bubbles end-to-end and
//      the browser-side wasm runtime boots the hello pack.

import { test, expect } from "@playwright/test";
import { loginAs } from "./helpers.mjs";

async function createContext(request, authHeader, name = "chat-target") {
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

test.describe("commit 3 — chat + brain", () => {
  test("unauthenticated GET /messages is 401", async ({ request }) => {
    // A random UUID — doesn't matter, auth fires before loading.
    const res = await request.get(
      "/api/contexts/00000000-0000-0000-0000-000000000000/messages",
    );
    expect(res.status()).toBe(401);
  });

  test("unauthenticated POST /messages is 401", async ({ request }) => {
    const res = await request.post(
      "/api/contexts/00000000-0000-0000-0000-000000000000/messages",
      {
        headers: { "Content-Type": "application/json" },
        data: { content: "hi" },
      },
    );
    expect(res.status()).toBe(401);
  });

  test("empty transcript for a fresh context", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader);
    const res = await request.get(
      `/api/contexts/${ctx.id}/messages`,
      { headers: { Authorization: authHeader } },
    );
    expect(res.ok()).toBe(true);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.messages).toEqual([]);
  });

  test("POST /messages stores user + assistant and returns both", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader);

    const res = await request.post(
      `/api/contexts/${ctx.id}/messages`,
      {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { content: "help me design a daily journal" },
      },
    );
    expect(res.status()).toBe(201);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.user_message.role).toBe("user");
    expect(body.user_message.content).toBe("help me design a daily journal");
    expect(body.assistant_message.role).toBe("assistant");
    // Fake brain echoes the user's content into the reply, so we
    // can check the round trip worked without knowing the exact
    // response shape.
    expect(body.assistant_message.content).toContain("[FAKE BRAIN]");
    expect(body.assistant_message.content).toContain("daily journal");
    expect(body.model).toBe("fake:chat");
  });

  test("GET /messages returns the transcript in order", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader);
    const prompts = ["first", "second", "third"];
    for (const p of prompts) {
      await request.post(`/api/contexts/${ctx.id}/messages`, {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { content: p },
      });
      // Tiny gap so created_at values are monotonic through the
      // postgres port even if the container clock is millisecond-
      // resolution.
      await new Promise((r) => setTimeout(r, 20));
    }

    const res = await request.get(`/api/contexts/${ctx.id}/messages`, {
      headers: { Authorization: authHeader },
    });
    const body = await res.json();
    const contents = body.messages.map((m) => m.content);
    // 3 prompts × 2 turns = 6 messages, alternating user/assistant.
    expect(body.messages).toHaveLength(6);
    expect(body.messages.map((m) => m.role)).toEqual([
      "user",
      "assistant",
      "user",
      "assistant",
      "user",
      "assistant",
    ]);
    expect(contents[0]).toBe("first");
    expect(contents[2]).toBe("second");
    expect(contents[4]).toBe("third");
  });

  test("one operator cannot read another's transcript", async ({
    request,
  }) => {
    const alice = await loginAs(request);
    const bob = await loginAs(request);
    const ctx = await createContext(
      request,
      alice.authHeader,
      "alice-transcript",
    );
    await request.post(`/api/contexts/${ctx.id}/messages`, {
      headers: {
        Authorization: alice.authHeader,
        "Content-Type": "application/json",
      },
      data: { content: "alice private thought" },
    });

    // Bob tries to read.
    const bobRead = await request.get(
      `/api/contexts/${ctx.id}/messages`,
      { headers: { Authorization: bob.authHeader } },
    );
    expect(bobRead.status()).toBe(404);

    // Bob tries to write.
    const bobWrite = await request.post(
      `/api/contexts/${ctx.id}/messages`,
      {
        headers: {
          Authorization: bob.authHeader,
          "Content-Type": "application/json",
        },
        data: { content: "hijack attempt" },
      },
    );
    expect(bobWrite.status()).toBe(404);

    // Alice still sees exactly her one turn + her brain reply.
    const aliceRead = await request.get(
      `/api/contexts/${ctx.id}/messages`,
      { headers: { Authorization: alice.authHeader } },
    );
    const body = await aliceRead.json();
    expect(body.messages).toHaveLength(2);
    expect(body.messages[0].content).toBe("alice private thought");
  });

  test("empty content is rejected with 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader);
    const res = await request.post(
      `/api/contexts/${ctx.id}/messages`,
      {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { content: "   " },
      },
    );
    expect(res.status()).toBe(400);
  });

  test("posting to a non-existent context is 404", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await request.post(
      "/api/contexts/00000000-0000-0000-0000-000000000000/messages",
      {
        headers: {
          Authorization: authHeader,
          "Content-Type": "application/json",
        },
        data: { content: "hello ghost" },
      },
    );
    expect(res.status()).toBe(404);
  });

  test("chat UI round trip + wasm runtime boots", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "ui-chat");

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

    // Click the freshly-created context in the list.
    const entry = page.locator(`.context-entry[data-context-id='${ctx.id}']`);
    await expect(entry).toBeVisible();
    await entry.click();
    await expect(page.locator("#view-context-detail")).toBeVisible();

    // Wasm runtime should boot and report the hello pack.
    await expect(page.locator("#runtime-summary")).toContainText("READY", {
      timeout: 15_000,
    });

    // Empty transcript marker.
    await expect(page.locator("#chat-empty")).toBeVisible();

    // Type a message and transmit.
    await page.fill("#input-chat", "hello terminal");
    await page.click("#btn-chat-send");

    // The transcript should now contain a user bubble + an assistant
    // bubble with the fake brain echo.
    const userBubble = page.locator(".chat-msg.user").first();
    const assistantBubble = page.locator(".chat-msg.assistant").first();
    await expect(userBubble).toContainText("hello terminal", {
      timeout: 10_000,
    });
    await expect(assistantBubble).toContainText("[FAKE BRAIN]", {
      timeout: 10_000,
    });
    await expect(assistantBubble).toContainText("hello terminal");
  });
});
