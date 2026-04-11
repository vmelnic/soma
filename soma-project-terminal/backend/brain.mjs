// OpenAI brain wrapper — commits 3+.
//
// Two distinct functions, because OpenAI's reasoning and chat
// families take incompatible parameter shapes:
//
//   `chatCompletion`       → gpt-4o-mini (and the 3.5/4 chat family)
//     - accepts temperature, max_tokens, top_p, stop
//     - rejects reasoning_effort
//     - used by commit 3 for the per-context conversational brain
//
//   `reasoningCompletion`  → gpt-5-mini (and the o1/o3 reasoning family)
//     - accepts reasoning_effort, response_format, max_completion_tokens
//     - rejects temperature, max_tokens — API 400s if you pass them
//     - used by commit 6 for structured pack generation
//
// Mixing them in one wrapper always regresses — the reasoning path
// doesn't tolerate the chat path's defaults and vice versa.
//
// Zero runtime deps: this module uses only global `fetch` (Node 20+)
// and talks directly to api.openai.com. Keeps the backend's
// package.json empty, consistent with commits 1 + 2.
//
// Fake mode: set BRAIN_FAKE=1 to bypass OpenAI entirely and return a
// canned reply. Used by the Playwright suite and by any dev who
// doesn't want to burn API quota. Fake mode still exercises the full
// request/response wire — only the model call is stubbed.

const OPENAI_URL = "https://api.openai.com/v1/chat/completions";

function fake() {
  return String(process.env.BRAIN_FAKE || "").trim() === "1";
}

function chatModel() {
  return process.env.OPENAI_CHAT_MODEL || "gpt-4o-mini";
}

function reasoningModel() {
  return process.env.OPENAI_PACK_MODEL || "gpt-5-mini";
}

function apiKey() {
  const k = process.env.OPENAI_API_KEY;
  if (!k || k === "sk-...") return null;
  return k;
}

// Strip any array entries the caller shouldn't have sent (empty
// content, unknown roles). OpenAI refuses messages with empty
// content strings in some model versions.
function sanitizeMessages(messages) {
  if (!Array.isArray(messages)) return [];
  const out = [];
  for (const m of messages) {
    if (!m || typeof m.content !== "string") continue;
    const content = m.content.trim();
    if (content === "") continue;
    const role = m.role === "assistant" ? "assistant" : "user";
    out.push({ role, content });
  }
  return out;
}

// Build the `messages` array OpenAI expects: [system, ...history].
// `systemPrompt` is required — this is how the per-context personality
// gets into the conversation.
function buildMessages(systemPrompt, history) {
  const msgs = [];
  if (systemPrompt && systemPrompt.trim() !== "") {
    msgs.push({ role: "system", content: systemPrompt });
  }
  for (const m of sanitizeMessages(history)) msgs.push(m);
  return msgs;
}

// Canned fake replies. Deterministic so tests can assert substrings.
// The last user message is echoed back so tests can round-trip
// user content through the fake brain.
function fakeChat(history) {
  const last = [...history].reverse().find((m) => m?.role === "user");
  const text = last?.content ?? "";
  return (
    "[FAKE BRAIN] I received your message: " +
    text.slice(0, 200) +
    (text.length > 200 ? "..." : "")
  );
}

function fakeReasoning(history) {
  const last = [...history].reverse().find((m) => m?.role === "user");
  const prompt = last?.content ?? "";
  return JSON.stringify({
    plan: [],
    explanation: `[FAKE REASONING] Received prompt of length ${prompt.length}. Returning an empty plan.`,
  });
}

// ---- chat (gpt-4o-mini) -------------------------------------------

export async function chatCompletion({
  systemPrompt,
  messages,
  temperature = 0.7,
  maxTokens = 800,
}) {
  const history = sanitizeMessages(messages);
  if (fake() || !apiKey()) {
    return {
      role: "assistant",
      content: fakeChat(history),
      model: "fake:chat",
      usage: null,
    };
  }

  const payload = {
    model: chatModel(),
    messages: buildMessages(systemPrompt, history),
    temperature,
    max_tokens: maxTokens,
  };

  const res = await fetch(OPENAI_URL, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${apiKey()}`,
    },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`OpenAI chat ${res.status}: ${body.slice(0, 400)}`);
  }
  const data = await res.json();
  const content = data.choices?.[0]?.message?.content;
  if (!content) {
    throw new Error(
      `OpenAI chat returned empty content: ${JSON.stringify(data).slice(0, 400)}`,
    );
  }
  return {
    role: "assistant",
    content,
    model: data.model ?? chatModel(),
    usage: data.usage ?? null,
  };
}

// ---- reasoning (gpt-5-mini) ---------------------------------------
// NOTE: this function is wired up so commit 6 can call it without
// further plumbing. Commit 3 itself does not hit this path — the
// in-repo test coverage for reasoning arrives in commit 6.

export async function reasoningCompletion({
  systemPrompt,
  messages,
  effort = "low",
  responseFormat,
  maxCompletionTokens,
}) {
  const history = sanitizeMessages(messages);
  if (fake() || !apiKey()) {
    return {
      role: "assistant",
      content: fakeReasoning(history),
      model: "fake:reasoning",
      usage: null,
    };
  }

  // Reasoning models reject `temperature` and `max_tokens`. Don't
  // put them in the payload even with a default — the API 400s
  // rather than ignoring the unknown field.
  const payload = {
    model: reasoningModel(),
    messages: buildMessages(systemPrompt, history),
    reasoning_effort: effort,
  };
  if (responseFormat) payload.response_format = responseFormat;
  if (maxCompletionTokens) {
    payload.max_completion_tokens = maxCompletionTokens;
  }

  const res = await fetch(OPENAI_URL, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${apiKey()}`,
    },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`OpenAI reasoning ${res.status}: ${body.slice(0, 400)}`);
  }
  const data = await res.json();
  const content = data.choices?.[0]?.message?.content;
  if (!content) {
    throw new Error(
      `OpenAI reasoning returned empty content: ${JSON.stringify(data).slice(0, 400)}`,
    );
  }
  return {
    role: "assistant",
    content,
    model: data.model ?? reasoningModel(),
    usage: data.usage ?? null,
  };
}

// System prompt builder for the per-context conversational brain.
// Keeps the context's name + description visible to the model on
// every turn so the conversation doesn't drift off-topic. Commit 6
// will replace or extend this when the context grows a compiled
// PackSpec — at that point the system prompt can describe the
// actual loaded ports and skills.
export function buildSystemPrompt(context) {
  const name = context?.name ?? "unnamed";
  const description =
    (context?.description && context.description.trim()) ||
    "(no description provided yet)";
  return [
    "You are the conversational brain of a SOMA terminal context.",
    "A context is the operator's project — a named workspace where",
    "they describe what they want to build and you help them shape it.",
    "",
    `Current context: ${name}`,
    `Description:    ${description}`,
    "",
    "Stay concise. Ask clarifying questions when the ask is ambiguous.",
    "When the operator describes a capability they want, suggest which",
    "SOMA ports (filesystem, http, postgres, smtp, crypto, ...) would",
    "implement it — but don't emit JSON or code unless the operator",
    "explicitly asks. Plain text, conversational register. Imagine you",
    "are speaking over a 1980s monochrome terminal — short sentences,",
    "no markdown headings, no emoji.",
  ].join("\n");
}
