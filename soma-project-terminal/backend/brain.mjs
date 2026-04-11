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
const OPENAI_AUDIO_URL = "https://api.openai.com/v1/audio/transcriptions";
const WHISPER_MODEL = "whisper-1";

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

// -------------------------------------------------------------------
// commit 6 — LLM-to-PackSpec
// -------------------------------------------------------------------
//
// `generatePackSpec(context, history)` asks gpt-5-mini to emit a
// valid PackSpec for the context based on the operator's chat log
// and the context name/description. The reasoning brain is given
// a constrained shape (the in-browser ports + a minimal skill
// schema) and returns a MINIMAL JSON object — we then expand it
// into a full PackSpec server-side so the wasm runtime accepts it.
//
// This split keeps the LLM's job small: instead of regurgitating
// 80 lines of observability/cost_prior/remote_exposure boilerplate,
// the model only has to pick an id, a name, and 1-3 skills. The
// boilerplate is always identical and gets filled in by
// `expandToFullPackSpec` below.

// The three ports already live in the browser wasm runtime (from
// soma-project-web's wasm build). Any skill the generated pack
// declares must map to one of these (port_id, capability_id) pairs
// or the wasm runtime rejects the manifest on boot.
const BROWSER_PORTS = Object.freeze([
  {
    port_id: "dom",
    description: "Renders text into the terminal document body.",
    capabilities: [
      {
        capability_id: "append_heading",
        description:
          "Appends an <h1..h6> to document.body. Input: { text: string, level?: 1|2|3|4|5|6 }",
      },
    ],
  },
  {
    port_id: "audio",
    description:
      "Speaks text aloud via the browser's built-in speechSynthesis API.",
    capabilities: [
      {
        capability_id: "say_text",
        description: "Speaks the given text. Input: { text: string }",
      },
    ],
  },
  {
    port_id: "voice",
    description:
      "Listens to the operator's microphone (browser SpeechRecognition). Output-only capability for commit 6; voice-in is commit 7.",
    capabilities: [],
  },
]);

const PACK_SYSTEM_PROMPT = `You are the PACK COMPILER of a SOMA terminal. Given the operator's stated goal and the chat history, emit a minimal JSON object describing a PackSpec that fits their intent.

Output a single JSON object with this exact shape:

{
  "pack_id":    "<dotted lowercase id, must start with soma.terminal.>",
  "pack_name":  "<human readable short name>",
  "description":"<one-sentence summary of what this pack does>",
  "skills": [
    {
      "skill_id":    "<dotted id under the pack_id>",
      "name":        "<human readable skill name>",
      "description": "<one-sentence summary of the skill>",
      "port_id":     "<one of the port_ids from the BROWSER PORTS block>",
      "capability_id": "<one of the listed capabilities for that port>",
      "input_schema_text": {
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "required": ["text"]
      }
    }
  ]
}

RULES:
- Emit ONLY the JSON object. No markdown, no commentary, no code fences.
- 1 to 3 skills is ideal. Never emit 0 skills.
- Every skill's (port_id, capability_id) MUST appear in the BROWSER PORTS block below. Do not invent ports.
- pack_id MUST start with "soma.terminal." and be lowercase-dotted.
- Each skill's input_schema_text is a JSON Schema object describing the skill's inputs.
- The output will be expanded into a full PackSpec server-side, so leave observability / cost_prior / remote_exposure etc. out.`;

function buildPackUserMessage(context, history) {
  const name = context?.name ?? "unnamed";
  const description =
    (context?.description && context.description.trim()) ||
    "(no description)";
  const transcriptLines = (history ?? [])
    .map(
      (m) =>
        `${m.role === "user" ? "OPERATOR" : "BRAIN"}: ${m.content}`,
    )
    .slice(-20); // last 20 turns is plenty for a reasoning brain
  const catalog = JSON.stringify(BROWSER_PORTS, null, 2);
  return [
    `CONTEXT NAME: ${name}`,
    `CONTEXT DESCRIPTION: ${description}`,
    "",
    "CHAT HISTORY (most recent first may be empty — operator may click 'generate pack' before talking):",
    transcriptLines.length === 0 ? "(none)" : transcriptLines.join("\n"),
    "",
    "BROWSER PORTS (the only ports your skills may reference):",
    catalog,
    "",
    "Return the pack as JSON.",
  ].join("\n");
}

// Slug a free-form context name into a valid pack id fragment.
// Lowercases, strips non-alphanumerics, collapses runs, and guards
// against empty results. Used as the fallback pack_id in fake
// mode and as a safety net when the LLM returns a malformed id.
function slugifyForPackId(name) {
  const slug = String(name ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ".")
    .replace(/^\.+|\.+$/g, "")
    .replace(/\.+/g, ".");
  return slug === "" ? "unnamed" : slug;
}

function fakePack(context) {
  const slug = slugifyForPackId(context?.name);
  const packId = `soma.terminal.${slug}`;
  const title = context?.name || "Unnamed";
  return {
    pack_id: packId,
    pack_name: `${title} Pack`,
    description:
      (context?.description && context.description.trim()) ||
      `Auto-generated fake pack for context "${title}".`,
    skills: [
      {
        skill_id: `${packId}.greet`,
        name: `${title} Greet`,
        description: `Renders a greeting for the ${title} context via the in-tab dom port.`,
        port_id: "dom",
        capability_id: "append_heading",
        input_schema_text: {
          type: "object",
          properties: { text: { type: "string" } },
          required: ["text"],
        },
      },
    ],
  };
}

// Minimal -> full PackSpec expansion. Everything the wasm runtime
// needs that the LLM didn't have to emit. Shape matches
// frontend/packs/hello/manifest.json (which is what the browser
// wasm successfully boots today), only the ids / names / skills
// differ.
function expandToFullPackSpec(minimal) {
  const packId = minimal.pack_id;
  const packName = minimal.pack_name || packId;
  const description = minimal.description || "";
  const skills = (minimal.skills ?? []).map((s) => {
    const skillId = s.skill_id;
    const inputSchema = s.input_schema_text || {
      type: "object",
      properties: {},
    };
    return {
      skill_id: skillId,
      namespace: packId,
      pack: packId,
      kind: "primitive",
      name: s.name || skillId,
      description: s.description || "",
      version: "0.1.0",
      inputs: { schema: inputSchema },
      outputs: {
        schema: {
          type: "object",
          properties: {
            rendered: { type: "boolean" },
            text: { type: "string" },
          },
        },
      },
      required_resources: [],
      preconditions: [],
      expected_effects: [],
      observables: [{ field: "rendered", role: "confirm_success" }],
      termination_conditions: [
        {
          condition_type: "success",
          expression: { rendered: true },
          description: `${s.capability_id} completed`,
        },
        {
          condition_type: "failure",
          expression: { error: "any" },
          description: `${s.port_id} port error`,
        },
      ],
      rollback_or_compensation: {
        support: "irreversible",
        compensation_skill: null,
        description: "browser effects are not rolled back",
      },
      cost_prior: {
        latency: {
          expected_latency_ms: 1,
          p95_latency_ms: 5,
          max_latency_ms: 50,
        },
        resource_cost: {
          cpu_cost_class: "negligible",
          memory_cost_class: "negligible",
          io_cost_class: "negligible",
          network_cost_class: "negligible",
          energy_cost_class: "negligible",
        },
      },
      risk_class: "low",
      determinism: "deterministic",
      remote_exposure: {
        remote_scope: "local",
        peer_trust_requirements: "none",
        serialization_requirements: "json",
        rate_limits: "none",
        replay_protection: false,
        observation_streaming: false,
        delegation_support: false,
        enabled: false,
      },
      tags: ["browser", "generated"],
      capability_requirements: [`port:${s.port_id}/${s.capability_id}`],
      confidence_threshold: null,
      locality: null,
      remote_endpoint: null,
      remote_trust_requirement: null,
      remote_capability_contract: null,
      fallback_skill: null,
      partial_success_behavior: null,
    };
  });

  return {
    id: packId,
    name: packName,
    version: "0.1.0",
    runtime_compatibility: ">=0.1.0",
    namespace: packId,
    capabilities: Array.from(
      new Set(
        (minimal.skills ?? []).map((s) => s.port_id),
      ),
    ).map((portId) => ({
      group_name: portId,
      scope: "local",
      capabilities: Array.from(
        new Set(
          (minimal.skills ?? [])
            .filter((s) => s.port_id === portId)
            .map((s) => s.capability_id),
        ),
      ),
    })),
    dependencies: [],
    resources: [],
    schemas: [],
    routines: [],
    policies: [],
    exposure: {
      local_skills: skills.map((s) => s.skill_id),
      remote_skills: [],
      local_resources: [],
      remote_resources: [],
      default_deny_destructive: false,
    },
    observability: {
      health_checks: [],
      version_metadata: { version: "0.1.0" },
      dependency_status: [],
      capability_inventory: Array.from(
        new Set(
          (minimal.skills ?? []).map((s) => s.capability_id),
        ),
      ),
      expected_latency_classes: ["fast"],
      expected_failure_modes: ["validation_error"],
      trace_categories: ["browser", "generated"],
      metric_names: [],
      pack_load_state: "active",
    },
    description,
    authors: [],
    tags: ["browser", "generated"],
    ports: [],
    port_dependencies: [],
    skills,
  };
}

// Validate the minimal shape the LLM is supposed to emit. Surfaces
// clear error messages rather than letting a malformed output fall
// through to the wasm runtime's cryptic boot failure.
function validateMinimal(minimal) {
  if (!minimal || typeof minimal !== "object" || Array.isArray(minimal)) {
    return "pack must be a JSON object";
  }
  const { pack_id: packId, pack_name: packName, skills } = minimal;
  if (typeof packId !== "string" || !packId.startsWith("soma.terminal.")) {
    return "pack_id must start with 'soma.terminal.'";
  }
  if (typeof packName !== "string" || packName.trim() === "") {
    return "pack_name is required";
  }
  if (!Array.isArray(skills) || skills.length === 0) {
    return "skills must be a non-empty array";
  }
  const validPortIds = new Set(BROWSER_PORTS.map((p) => p.port_id));
  for (const s of skills) {
    if (!s || typeof s !== "object") return "skill must be an object";
    if (typeof s.skill_id !== "string" || !s.skill_id.startsWith(packId)) {
      return `skill_id must start with pack_id "${packId}"`;
    }
    if (!validPortIds.has(s.port_id)) {
      return `unknown port_id "${s.port_id}" (valid: ${[...validPortIds].join(", ")})`;
    }
    const port = BROWSER_PORTS.find((p) => p.port_id === s.port_id);
    const validCapIds = new Set(
      port.capabilities.map((c) => c.capability_id),
    );
    if (!validCapIds.has(s.capability_id)) {
      return `unknown capability_id "${s.capability_id}" for port "${s.port_id}"`;
    }
  }
  return null;
}

// Main entry point. Returns { ok, pack } where `pack` is the full
// PackSpec ready to hand to contexts.setPackSpec. On failure
// returns { ok: false, error }.
export async function generatePackSpec({ context, history }) {
  // Fake mode always succeeds with a canned minimal pack derived
  // from the context name. This keeps the test suite hermetic and
  // preserves the full expand-to-PackSpec + validation path that
  // the real brain output also travels through.
  if (fake() || !apiKey()) {
    const minimal = fakePack(context);
    const err = validateMinimal(minimal);
    if (err) return { ok: false, error: `fake pack invalid: ${err}` };
    return {
      ok: true,
      pack: expandToFullPackSpec(minimal),
      model: "fake:reasoning",
      minimal,
    };
  }

  let reply;
  try {
    reply = await reasoningCompletion({
      systemPrompt: PACK_SYSTEM_PROMPT,
      messages: [
        {
          role: "user",
          content: buildPackUserMessage(context, history),
        },
      ],
      effort: "low",
      responseFormat: { type: "json_object" },
    });
  } catch (e) {
    return { ok: false, error: `reasoning brain failed: ${e.message}` };
  }

  let minimal;
  try {
    minimal = JSON.parse(reply.content);
  } catch (e) {
    return {
      ok: false,
      error: `reasoning brain returned non-JSON: ${e.message}`,
    };
  }

  const err = validateMinimal(minimal);
  if (err) return { ok: false, error: `generated pack invalid: ${err}` };

  return {
    ok: true,
    pack: expandToFullPackSpec(minimal),
    model: reply.model,
    minimal,
  };
}

// -------------------------------------------------------------------
// commit 7 — Whisper voice input
// -------------------------------------------------------------------
//
// `transcribeAudio({audioBuffer, mimeType})` forwards a raw audio
// blob to OpenAI's Whisper endpoint via multipart/form-data and
// returns the extracted text. The frontend records via
// MediaRecorder and posts the bytes to /api/transcribe, which
// calls this function. Fake mode bypasses OpenAI with a
// deterministic echo string — the Playwright suite runs with
// BRAIN_FAKE=1 so tests never hit real quota.
//
// Node 20+ ships global FormData / Blob, so we don't need any
// form-data dep — the wrapper stays zero-dep like the rest of
// backend/brain.mjs.

export async function transcribeAudio({ audioBuffer, mimeType }) {
  if (!audioBuffer || audioBuffer.length === 0) {
    return { ok: false, error: "empty audio" };
  }
  if (fake() || !apiKey()) {
    return {
      ok: true,
      text: `[FAKE TRANSCRIBE] ${audioBuffer.length} bytes received (${mimeType || "unknown"})`,
      model: "fake:whisper",
    };
  }

  const blob = new Blob([audioBuffer], {
    type: mimeType || "audio/webm",
  });
  // File extension is mostly cosmetic — OpenAI sniffs the content
  // type from the blob. The extension matching the actual codec
  // makes log output friendlier. MediaRecorder defaults to webm/
  // opus in Chromium, so that's our default.
  const extMap = {
    "audio/webm": "webm",
    "audio/ogg": "ogg",
    "audio/mp4": "mp4",
    "audio/mpeg": "mp3",
    "audio/wav": "wav",
  };
  const baseMime = String(mimeType || "audio/webm").split(";")[0].trim();
  const ext = extMap[baseMime] || "webm";

  const form = new FormData();
  form.append("file", blob, `audio.${ext}`);
  form.append("model", WHISPER_MODEL);

  const res = await fetch(OPENAI_AUDIO_URL, {
    method: "POST",
    headers: { Authorization: `Bearer ${apiKey()}` },
    body: form,
  });
  if (!res.ok) {
    const body = await res.text();
    return {
      ok: false,
      error: `OpenAI whisper ${res.status}: ${body.slice(0, 400)}`,
    };
  }
  const data = await res.json();
  if (typeof data.text !== "string") {
    return {
      ok: false,
      error: `whisper response missing text: ${JSON.stringify(data).slice(0, 400)}`,
    };
  }
  return {
    ok: true,
    text: data.text,
    model: WHISPER_MODEL,
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
