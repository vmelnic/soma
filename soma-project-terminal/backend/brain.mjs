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

// The ports a generated pack may reference. Two disjoint groups:
//
//   scope: "wasm"            — already registered inside the
//                              soma-next wasm runtime in the browser
//                              tab. `soma_invoke_port` runs them
//                              natively. Zero network hops.
//
//   scope: "backend_bridge"  — NOT resident in the wasm runtime.
//                              The browser's JS-side skill executor
//                              routes these invocations via fetch
//                              to POST /api/contexts/:id/port/:portId
//                              /:capabilityId, where the Node gateway
//                              calls a backend-hosted handler in
//                              contextkv.mjs (or, later, other
//                              bridged modules). Still SOMA-native at
//                              heart — the gateway's own side effects
//                              flow through soma-ports via MCP — but
//                              the wasm runtime never sees these
//                              calls.
//
// Both groups are advertised to the chat brain AND the pack brain
// so generated skills can freely mix them. The JS executor inspects
// the scope field at dispatch time to pick wasm vs. bridge.
const BROWSER_PORTS = Object.freeze([
  {
    port_id: "dom",
    scope: "wasm",
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
    scope: "wasm",
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
    scope: "wasm",
    description:
      "Listens to the operator's microphone (browser SpeechRecognition). No callable capabilities yet — voice input arrives via the chat form's mic button, not a pack skill.",
    capabilities: [],
  },
  {
    port_id: "context_kv",
    scope: "backend_bridge",
    description:
      "Per-context key/value store backed by postgres. Survives page reloads. Scoped to the current operator's context — reads and writes from other contexts are impossible.",
    capabilities: [
      {
        capability_id: "set",
        description:
          "Upsert a key. Input: { key: string, value: string }. Overwrites if the key already exists.",
      },
      {
        capability_id: "get",
        description:
          "Fetch a value by key. Input: { key: string }. Returns null if the key doesn't exist.",
      },
      {
        capability_id: "delete",
        description:
          "Remove a key. Input: { key: string }. Idempotent — no error if the key wasn't there.",
      },
      {
        capability_id: "list",
        description:
          "List keys, optionally filtered by prefix. Input: { prefix?: string }. Returns rows with key + value.",
      },
    ],
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
- 1 to 4 skills is ideal. Never emit 0 skills.
- Every skill's (port_id, capability_id) MUST appear in the BROWSER PORTS block below. Do not invent ports.
- pack_id MUST start with "soma.terminal." and be lowercase-dotted.
- Each skill's input_schema_text is a JSON Schema object describing the skill's inputs.
- The output will be expanded into a full PackSpec server-side, so leave observability / cost_prior / remote_exposure etc. out.

PORT SCOPES (both are usable, they just run differently):
- wasm-scope ports (dom, audio): execute inside the browser wasm runtime via soma_invoke_port. Fast, offline, no server round trip. Effects live only in the current page until reload.
- backend_bridge-scope ports (context_kv): execute via a POST to the Node gateway, which calls the backend handler and returns a PortCallRecord. Useful for anything that must survive a page reload. Feel free to combine wasm and bridge skills in the same pack — a "todo list" pack naturally has dom.append_heading (render) and context_kv.set / context_kv.list (persist).

GRACEFUL DEGRADATION:
- If the operator's chat history mentions capabilities beyond the BROWSER PORTS catalog (e.g. real SMTP email, outbound HTTP, filesystem writes, S3, push notifications), reduce the scope to a DEMO VERSION that the available ports can actually deliver. context_kv already covers persistence, so "save my todos" is fine; "email me reminders" is not — flag it as future work and skip the skill.
- NEVER fake an unavailable port with an available one. Do not claim dom.append_heading "emails" anything, or that context_kv.set "sends SMS". Honest minimal scope beats ambitious unbuildable scope.`;

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
  // The fake pack deliberately mixes wasm + bridge scopes so the
  // full executor path gets exercised in hermetic tests. Real-
  // mode gpt-5-mini will usually produce something similar because
  // the PACK_SYSTEM_PROMPT recommends combining context_kv and dom
  // for any "remember something and render it" intent.
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
      {
        skill_id: `${packId}.remember`,
        name: `${title} Remember`,
        description: `Stores a key/value pair in the ${title} context's persistent KV.`,
        port_id: "context_kv",
        capability_id: "set",
        input_schema_text: {
          type: "object",
          properties: {
            key: { type: "string" },
            value: { type: "string" },
          },
          required: ["key", "value"],
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
//
// Scope-aware routing:
//   - wasm-scope skills (dom, audio, voice) put their real port+
//     capability into `capability_requirements` so the wasm
//     runtime's pack validator can check port registration.
//   - bridge-scope skills (context_kv, future http/smtp) leave
//     `capability_requirements` EMPTY — the wasm runtime doesn't
//     host those ports and would reject the pack otherwise — and
//     instead encode the port/capability as a `bridge:PORT:CAP`
//     tag that the JS skill executor parses at invocation time.
//   - Pack-level `capabilities` block ONLY lists wasm ports. The
//     `exposure.local_skills` list still includes every skill id
//     so the pack's public surface is complete.
function scopeForPort(portId) {
  return BROWSER_PORTS.find((p) => p.port_id === portId)?.scope || "wasm";
}

function expandToFullPackSpec(minimal) {
  const packId = minimal.pack_id;
  const packName = minimal.pack_name || packId;
  const description = minimal.description || "";
  const minimalSkills = minimal.skills ?? [];

  const skills = minimalSkills.map((s) => {
    const skillId = s.skill_id;
    const inputSchema = s.input_schema_text || {
      type: "object",
      properties: {},
    };
    const scope = scopeForPort(s.port_id);
    const isWasm = scope === "wasm";
    const capRequirements = isWasm
      ? [`port:${s.port_id}/${s.capability_id}`]
      : [];
    // Bridge tag convention: "bridge:<port_id>:<capability_id>".
    // The JS executor parses this at invocation time. Keep it
    // even for wasm skills as a breadcrumb (the executor prefers
    // the tag if present, but falls back to capability_requirements).
    const tags = [
      "browser",
      "generated",
      isWasm ? `scope:wasm` : `scope:bridge`,
      `bridge:${s.port_id}:${s.capability_id}`,
    ];
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
          description: `${s.port_id} invocation error`,
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
      tags,
      capability_requirements: capRequirements,
      confidence_threshold: null,
      locality: null,
      remote_endpoint: null,
      remote_trust_requirement: null,
      remote_capability_contract: null,
      fallback_skill: null,
      partial_success_behavior: null,
    };
  });

  // Pack-level capabilities block only lists WASM ports. Bridge
  // ports aren't registered in the wasm runtime and listing them
  // here would make soma_boot_runtime reject the pack.
  const wasmPortIds = Array.from(
    new Set(
      minimalSkills
        .filter((s) => scopeForPort(s.port_id) === "wasm")
        .map((s) => s.port_id),
    ),
  );

  return {
    id: packId,
    name: packName,
    version: "0.1.0",
    runtime_compatibility: ">=0.1.0",
    namespace: packId,
    capabilities: wasmPortIds.map((portId) => ({
      group_name: portId,
      scope: "local",
      capabilities: Array.from(
        new Set(
          minimalSkills
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
        new Set(minimalSkills.map((s) => s.capability_id)),
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
// Keeps the context's name + description visible on every turn so
// the conversation doesn't drift off-topic, and — crucially — tells
// the brain the EXACT ceiling of what the browser-side wasm runtime
// can actually execute today. Otherwise the chat brain happily
// agrees to "we'll use Postgres for storage and SMTP for reminders"
// when the runtime only has dom / audio / voice ports, leaving the
// operator with a compiled pack that can't deliver the conversation's
// promises.
//
// Two ground rules baked in here:
//   1. List the in-tab ports the wasm body actually ships, including
//      their exact capabilities. Anything else is "future work" and
//      the brain has to say so out loud.
//   2. When the operator is ready to build, the brain must tell them
//      to click [ GENERATE PACK ]. No manual setup instructions,
//      ever — the pack brain does all the compiling.
export function buildSystemPrompt(context) {
  const name = context?.name ?? "unnamed";
  const description =
    (context?.description && context.description.trim()) ||
    "(no description provided yet)";
  // Re-project BROWSER_PORTS into a flat bullet list. Kept in sync
  // with the catalog the pack brain uses in PACK_SYSTEM_PROMPT —
  // both brains speak the same truth about what's callable. The
  // scope tag tells the operator (and the brain) whether a port
  // runs inside the wasm body or hops through the backend bridge.
  const portBullets = BROWSER_PORTS.flatMap((p) => {
    const scopeTag =
      p.scope === "backend_bridge" ? " [backend bridge]" : " [in-tab wasm]";
    if (p.capabilities.length === 0) {
      return [
        `  - ${p.port_id}${scopeTag}: ${p.description} (no callable capabilities yet)`,
      ];
    }
    return [
      `  - ${p.port_id}${scopeTag}: ${p.description}`,
      ...p.capabilities.map(
        (c) => `      * ${c.capability_id} — ${c.description}`,
      ),
    ];
  }).join("\n");

  return [
    "You are the conversational brain of a SOMA terminal context.",
    "A context is the operator's project — a named workspace where",
    "they describe what they want to build and you help them shape it.",
    "",
    `Current context: ${name}`,
    `Description:    ${description}`,
    "",
    "=============================================================",
    "WHAT THE BROWSER RUNTIME CAN ACTUALLY DO RIGHT NOW",
    "=============================================================",
    "The context runs inside a soma-next wasm runtime living in the",
    "operator's browser tab. That runtime has a FIXED set of ports:",
    "",
    portBullets,
    "",
    "When the operator clicks [ GENERATE PACK ], a reasoning brain",
    "compiles a PackSpec whose skills MUST map onto one of those",
    "(port_id, capability_id) pairs. Skills referencing anything",
    "else will be rejected before the wasm boots.",
    "",
    "=============================================================",
    "WHAT THE BROWSER RUNTIME CANNOT DO YET",
    "=============================================================",
    "The wider SOMA project has production ports for filesystem,",
    "http, postgres, redis, smtp, crypto, s3, timer, push, image,",
    "and geo. Most of them are not yet exposed to the browser.",
    "",
    "Today the backend-port bridge only exposes context_kv (above).",
    "That means `persistence across page reloads` IS buildable —",
    "recommend context_kv when the operator wants their todos,",
    "notes, or counters to survive a reload. But real SMTP email,",
    "outbound HTTP fetches, S3, SMS push, and filesystem writes",
    "are NOT YET buildable — flag them as future work and do not",
    "promise them in a pack.",
    "",
    "=============================================================",
    "HOW THE OPERATOR ACTUALLY BUILDS THINGS",
    "=============================================================",
    "The terminal has a [ GENERATE PACK ] button next to the",
    "runtime panel. When the operator is ready, tell them to click",
    "it. The reasoning brain will read this chat transcript, emit",
    "a minimal pack shape, and the backend will compile it into a",
    "full PackSpec + hot-swap the wasm body. No manual database",
    "setup. No shell commands. No 'first do X, then do Y' steps.",
    "The button does all of it.",
    "",
    "=============================================================",
    "CONVERSATIONAL STYLE",
    "=============================================================",
    "Stay concise. Imagine you are speaking over a 1980s monochrome",
    "terminal — short sentences, no markdown headings, no emoji.",
    "Ask clarifying questions when the ask is ambiguous.",
    "",
    "When the operator's request exceeds the browser runtime's",
    "current ceiling, acknowledge the gap directly and offer a",
    "buildable demo version: for example, a todo list that lives",
    "as <h1> entries in the DOM for the current session, with the",
    "real persistence + email + sharing flagged as future work.",
    "Then, when they agree, tell them to click [ GENERATE PACK ].",
    "",
    "Never emit JSON, PackSpec fragments, SQL, or shell commands",
    "unless the operator explicitly asks for them.",
  ].join("\n");
}
