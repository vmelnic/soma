// soma-project-terminal — chat + tool-calling tests.
//
// After the conversation-first pivot, every operator turn flows
// through the chat brain as a tool-calling loop: the backend runs
// runChatTurn with tool definitions (list_ports / list_skills /
// invoke_port), the model may issue tool calls, the backend
// executes each against soma-next via MCP, and the final assistant
// text lands in the transcript.
//
// Fake mode (BRAIN_FAKE=1 via playwright.config.js) handles the
// OpenAI side deterministically. To test the tool-calling path we
// use the ::tool escape trigger baked into fakeMode: if a user
// message matches `::tool <name> <json-args>`, the fake brain
// issues exactly that tool call against the REAL SomaMcpClient and
// embeds the result in its reply. This gives tests a deterministic
// way to drive the tool-call loop without an actual LLM.
//
// Covered:
//   1. Unauth GET / POST → 401
//   2. Empty transcript for a fresh context
//   3. Plain user turn round-trips (no tool calls)
//   4. Transcript ordering across multiple turns
//   5. Cross-user isolation on both read and write
//   6. Empty-content 400
//   7. Non-existent context 404
//   8. ::tool trigger runs list_ports against the real soma-next
//      subprocess and the result comes back in the assistant reply
//   9. ::tool trigger for invoke_port — the backend executes a real
//      postgres.query and the result is embedded in the response's
//      tool_calls trace
//  10. UI chat round trip (full-width, no right panel)

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

async function postMessage(request, authHeader, contextId, content) {
  return request.post(`/api/contexts/${contextId}/messages`, {
    headers: {
      Authorization: authHeader,
      "Content-Type": "application/json",
    },
    data: { content },
  });
}

test.describe("chat + tool-calling", () => {
  test("unauthenticated GET /messages is 401", async ({ request }) => {
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

  test("plain user turn round-trips through fake brain echo", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader);

    const res = await postMessage(
      request,
      authHeader,
      ctx.id,
      "help me design a daily journal",
    );
    expect(res.status()).toBe(201);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.user_message.role).toBe("user");
    expect(body.user_message.content).toBe("help me design a daily journal");
    expect(body.assistant_message.role).toBe("assistant");
    expect(body.assistant_message.content).toContain("[FAKE BRAIN]");
    expect(body.assistant_message.content).toContain("daily journal");
    expect(body.model).toBe("fake:chat");
    // No tool calls for a plain echo.
    expect(body.tool_calls).toEqual([]);
  });

  test("GET /messages returns the transcript in order", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader);
    const prompts = ["first", "second", "third"];
    for (const p of prompts) {
      await postMessage(request, authHeader, ctx.id, p);
      await new Promise((r) => setTimeout(r, 20));
    }

    const res = await request.get(`/api/contexts/${ctx.id}/messages`, {
      headers: { Authorization: authHeader },
    });
    const body = await res.json();
    expect(body.messages).toHaveLength(6);
    expect(body.messages.map((m) => m.role)).toEqual([
      "user",
      "assistant",
      "user",
      "assistant",
      "user",
      "assistant",
    ]);
    expect(body.messages[0].content).toBe("first");
    expect(body.messages[2].content).toBe("second");
    expect(body.messages[4].content).toBe("third");
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
    await postMessage(
      request,
      alice.authHeader,
      ctx.id,
      "alice private thought",
    );

    const bobRead = await request.get(
      `/api/contexts/${ctx.id}/messages`,
      { headers: { Authorization: bob.authHeader } },
    );
    expect(bobRead.status()).toBe(404);

    const bobWrite = await postMessage(
      request,
      bob.authHeader,
      ctx.id,
      "hijack attempt",
    );
    expect(bobWrite.status()).toBe(404);

    const aliceRead = await request.get(
      `/api/contexts/${ctx.id}/messages`,
      { headers: { Authorization: alice.authHeader } },
    );
    const body = await aliceRead.json();
    expect(body.messages).toHaveLength(2);
    expect(body.messages[0].content).toBe("alice private thought");
  });

  test("empty content is rejected 400", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader);
    const res = await postMessage(request, authHeader, ctx.id, "   ");
    expect(res.status()).toBe(400);
  });

  test("posting to a non-existent context is 404", async ({ request }) => {
    const { authHeader } = await loginAs(request);
    const res = await postMessage(
      request,
      authHeader,
      "00000000-0000-0000-0000-000000000000",
      "hello ghost",
    );
    expect(res.status()).toBe(404);
  });

  test("::tool invoke_port crypto.random_string runs via MCP", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "tool-crypto");

    // Drive the fake brain to invoke a real port capability via
    // the one tool we expose (invoke_port). The crypto port is
    // stateless so the result is deterministic in shape (a
    // `value` string), which is easy to assert on.
    const args = {
      port_id: "crypto",
      capability_id: "random_string",
      input: { length: 16 },
    };
    const res = await postMessage(
      request,
      authHeader,
      ctx.id,
      `::tool invoke_port ${JSON.stringify(args)}`,
    );
    expect(res.status()).toBe(201);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.tool_calls).toHaveLength(1);
    expect(body.tool_calls[0].name).toBe("invoke_port");
    expect(body.tool_calls[0].result.ok).toBe(true);
    // crypto.random_string returns { value: "<N chars>" }
    expect(
      typeof body.tool_calls[0].result.result.value,
    ).toBe("string");
    expect(body.tool_calls[0].result.result.value.length).toBe(16);
  });

  test("unknown tool name is rejected at the dispatch layer", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "tool-bogus");

    // list_ports was removed from the exposed tool catalog in the
    // conversation-first tightening — only invoke_port is callable
    // now. Driving ::tool list_ports through the fake brain should
    // surface an "unknown tool" error in the trace, not a success.
    const res = await postMessage(
      request,
      authHeader,
      ctx.id,
      "::tool list_ports {}",
    );
    expect(res.status()).toBe(201);
    const body = await res.json();
    expect(body.tool_calls).toHaveLength(1);
    expect(body.tool_calls[0].name).toBe("list_ports");
    expect(body.tool_calls[0].result.ok).toBe(false);
    expect(body.tool_calls[0].result.error).toMatch(/unknown tool/i);
  });

  test("::tool invoke_port runs a real postgres query via MCP", async ({
    request,
  }) => {
    const { authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "tool-invoke");

    // Drive the fake brain to issue an invoke_port tool call. The
    // backend will execute it against the real soma-next subprocess,
    // which runs it through the postgres port dylib.
    const args = {
      port_id: "postgres",
      capability_id: "query",
      input: { sql: "SELECT 1 AS ok" },
    };
    const res = await postMessage(
      request,
      authHeader,
      ctx.id,
      `::tool invoke_port ${JSON.stringify(args)}`,
    );
    expect(res.status()).toBe(201);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.tool_calls).toHaveLength(1);
    expect(body.tool_calls[0].name).toBe("invoke_port");
    expect(body.tool_calls[0].result.ok).toBe(true);
    // The postgres port returned rows — one with column "ok" = 1.
    const structured = body.tool_calls[0].result.result;
    expect(Array.isArray(structured.rows)).toBe(true);
    expect(structured.rows[0].ok).toBe(1);
  });

  test("UI chat round trip — full-width panel, no right side", async ({
    page,
    context,
    request,
  }) => {
    const { sessionToken, authHeader } = await loginAs(request);
    const ctx = await createContext(request, authHeader, "ui-chat");

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

    const entry = page.locator(`.context-entry[data-context-id='${ctx.id}']`);
    await expect(entry).toBeVisible();
    await entry.click();
    await expect(page.locator("#view-context-detail")).toBeVisible();

    // The runtime panel is gone — no #runtime-summary in the DOM.
    await expect(page.locator("#runtime-summary")).toHaveCount(0);
    // The skills grid is gone.
    await expect(page.locator("#skills-list")).toHaveCount(0);
    // The memory panel is gone.
    await expect(page.locator("#memory-summary")).toHaveCount(0);
    // The generate-pack button is gone.
    await expect(page.locator("#btn-generate-pack")).toHaveCount(0);

    // Chat is visible and empty.
    await expect(page.locator("#chat-empty")).toBeVisible();
    await expect(page.locator("#input-chat")).toBeVisible();

    // Type a message and submit.
    await page.fill("#input-chat", "hello terminal");
    await page.click("#btn-chat-send");

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
