// OpenAI brain wrapper — conversation-first, tool-calling.
//
// The terminal's only brain is a chat model (default gpt-4o-mini).
// Every operator turn runs one chat-completion loop:
//
//   1. Build messages: [system prompt, ...conversation history, new user msg]
//   2. Call chat.completions with `tools` = the live MCP tool catalog
//      derived from the backend's soma-next subprocess
//   3. If the response contains tool_calls, execute each one via the
//      provided `invokeTool(name, args)` handler and feed the results
//      back into the loop
//   4. Repeat until the response has no more tool_calls
//   5. The final assistant content string is the transcript message
//
// There is NO reasoning brain, NO pack generation, NO view DSL. The
// brain does all its work via tool calls against the SOMA runtime,
// which is the one body we maintain as code. Adding a new capability
// = adding a new port to the master pack (a human change), not an
// LLM-generated artifact.
//
// Fake mode: set BRAIN_FAKE=1 to bypass OpenAI entirely and return
// a deterministic canned reply shape. Tool-call execution still
// runs in fake mode (against the live SOMA runtime) so tests can
// exercise the full wire without hitting OpenAI quota.
//
// Zero runtime deps: Node 20+ global `fetch`, `FormData`, `Blob`.

const OPENAI_URL = "https://api.openai.com/v1/chat/completions";
const OPENAI_AUDIO_URL = "https://api.openai.com/v1/audio/transcriptions";
const WHISPER_MODEL = "whisper-1";

// Safety cap on tool-call loops. A runaway model that keeps
// invoking tools without ever producing a final answer shouldn't
// be able to burn quota or hammer the backend. 8 iterations is
// well above what any sane chat turn needs.
const MAX_TOOL_LOOPS = 8;

function fake() {
  return String(process.env.BRAIN_FAKE || "").trim() === "1";
}

function chatModel() {
  return process.env.OPENAI_CHAT_MODEL || "gpt-4o-mini";
}

function apiKey() {
  const k = process.env.OPENAI_API_KEY;
  if (!k || k === "sk-...") return null;
  return k;
}

// Drop entries the model should never see: empty content strings
// (OpenAI rejects them), unknown roles, system messages in the
// history (the caller already prepends its own system prompt).
function sanitizeHistory(history) {
  if (!Array.isArray(history)) return [];
  const out = [];
  for (const m of history) {
    if (!m || typeof m !== "object") continue;
    const role = m.role;
    if (role !== "user" && role !== "assistant") continue;
    const content = typeof m.content === "string" ? m.content.trim() : "";
    if (content === "") continue;
    out.push({ role, content });
  }
  return out;
}

function buildMessages(systemPrompt, history) {
  const msgs = [];
  if (systemPrompt && systemPrompt.trim() !== "") {
    msgs.push({ role: "system", content: systemPrompt });
  }
  for (const m of sanitizeHistory(history)) msgs.push(m);
  return msgs;
}

// -------------------------------------------------------------------
// fake-mode canned reply
// -------------------------------------------------------------------
//
// Tests run with BRAIN_FAKE=1 and don't want to hit OpenAI. But the
// conversation-first architecture still needs the full tool-calling
// loop to run so tests can exercise it. So fake mode simulates a
// simple pattern:
//
//   - If the latest user message matches a "tool trigger" pattern
//     (`::tool <name> <json-args>`), the fake brain emits a tool call
//     with those args, runs the tool, feeds the result back, and
//     produces a final message that embeds the result summary.
//   - Otherwise it produces an echo reply so plain chat still works.
//
// The trigger syntax is only used in tests. Real users never see
// it. It gives tests a deterministic way to say "now the model
// should call tool X with these args" without actually prompting
// an LLM.

function fakeTextReply(history) {
  const last = [...history].reverse().find((m) => m?.role === "user");
  const text = (last?.content ?? "").trim();
  return `[FAKE BRAIN] I received your message: ${text.slice(0, 200)}${text.length > 200 ? "..." : ""}`;
}

function parseFakeToolTrigger(history) {
  const last = [...history].reverse().find((m) => m?.role === "user");
  const text = (last?.content ?? "").trim();
  const match = text.match(/^::tool\s+(\S+)\s+(.*)$/s);
  if (!match) return null;
  try {
    return { name: match[1], args: JSON.parse(match[2]) };
  } catch {
    return null;
  }
}

// -------------------------------------------------------------------
// chat turn with tool calling
// -------------------------------------------------------------------

// Run one full chat turn with tool calling against OpenAI (or the
// fake brain).
//
//   systemPrompt:  the universal meta prompt, with the context name
//                  inlined as a label
//   history:       prior conversation turns, [{role, content}, ...]
//   tools:         array of OpenAI tool definitions (function tools),
//                  e.g. [{type: "function", function: {name, description, parameters}}]
//   invokeTool:    async (name, args) => { ok, result } | { ok: false, error }
//                  the caller injects this so the route handler
//                  decides what each tool call actually does
//
// Returns { content, tool_calls: [{name, args, result}], model }.
// `content` is the final assistant text (stored as the assistant
// message in the transcript). `tool_calls` is a trace the caller
// may ignore or log. `model` is "fake:chat" in fake mode.
export async function runChatTurn({
  systemPrompt,
  history,
  tools,
  invokeTool,
  temperature = 0.7,
  maxTokens = 800,
}) {
  const sanitized = sanitizeHistory(history);
  const traceToolCalls = [];

  if (fake() || !apiKey()) {
    // Fake mode: check for a ::tool trigger in the latest user
    // message. If present, run the tool and embed the result in
    // the reply. Otherwise echo.
    const trigger = parseFakeToolTrigger(sanitized);
    if (trigger && typeof invokeTool === "function") {
      let toolResult;
      try {
        toolResult = await invokeTool(trigger.name, trigger.args);
      } catch (e) {
        toolResult = { ok: false, error: e.message };
      }
      traceToolCalls.push({
        name: trigger.name,
        args: trigger.args,
        result: toolResult,
      });
      return {
        content: `[FAKE BRAIN] tool ${trigger.name} returned ${JSON.stringify(toolResult).slice(0, 300)}`,
        tool_calls: traceToolCalls,
        model: "fake:chat",
      };
    }
    return {
      content: fakeTextReply(sanitized),
      tool_calls: traceToolCalls,
      model: "fake:chat",
    };
  }

  // Real path: multi-turn tool-calling loop against OpenAI.
  const runningMessages = buildMessages(systemPrompt, sanitized);
  let finalContent = null;
  let lastModel = chatModel();

  for (let loop = 0; loop < MAX_TOOL_LOOPS; loop += 1) {
    const payload = {
      model: chatModel(),
      messages: runningMessages,
      temperature,
      max_tokens: maxTokens,
    };
    if (Array.isArray(tools) && tools.length > 0) {
      payload.tools = tools;
      payload.tool_choice = "auto";
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
      throw new Error(
        `OpenAI chat ${res.status}: ${body.slice(0, 400)}`,
      );
    }
    const data = await res.json();
    lastModel = data.model ?? chatModel();
    const choice = data.choices?.[0];
    const message = choice?.message;
    if (!message) {
      throw new Error(
        `OpenAI chat returned empty choice: ${JSON.stringify(data).slice(0, 400)}`,
      );
    }

    // If the model produced a plain content reply and no tool
    // calls, we're done.
    const toolCalls = Array.isArray(message.tool_calls)
      ? message.tool_calls
      : [];
    if (toolCalls.length === 0) {
      finalContent = message.content ?? "";
      break;
    }

    // Otherwise push the assistant message with its tool_calls
    // into the history, execute each call, push the tool results
    // back, and loop.
    runningMessages.push({
      role: "assistant",
      content: message.content ?? null,
      tool_calls: toolCalls,
    });

    for (const tc of toolCalls) {
      const name = tc.function?.name;
      let args = {};
      try {
        args = tc.function?.arguments
          ? JSON.parse(tc.function.arguments)
          : {};
      } catch (e) {
        args = { __parse_error: e.message };
      }

      let toolResult;
      try {
        toolResult = await invokeTool(name, args);
      } catch (e) {
        toolResult = { ok: false, error: e.message };
      }
      traceToolCalls.push({ name, args, result: toolResult });

      runningMessages.push({
        role: "tool",
        tool_call_id: tc.id,
        content: JSON.stringify(toolResult).slice(0, 8000),
      });
    }
  }

  if (finalContent === null) {
    // Hit the MAX_TOOL_LOOPS cap without a final content. Return
    // whatever trace we have with a clear error so the transcript
    // isn't silently blank.
    finalContent =
      "(brain exceeded the tool-call loop cap — check logs for the full trace)";
  }

  return { content: finalContent, tool_calls: traceToolCalls, model: lastModel };
}

// -------------------------------------------------------------------
// universal meta system prompt
// -------------------------------------------------------------------
//
// Same prompt for every context. The context's identity comes from
// (a) its name, inlined as a label, and (b) the conversation
// history, which is the real source of truth. Observation-first
// rules force the model to re-read the runtime state each turn
// instead of "remembering" what it decided.

const META_SYSTEM_PROMPT = `You are SOMA, a personal assistant running inside a terminal. The operator is working in a single conversation context — a named space with its own chat history and its own scoped data. You have access to tools that invoke real ports in the SOMA runtime: storage, email, HTTP, crypto, filesystem, etc.

You have three tools available by default:
- list_ports:  returns the live port catalog with capabilities. Call this when you don't know what's available.
- list_skills: returns the skill catalog for the currently loaded pack.
- invoke_port: invokes a specific (port_id, capability_id) with an input object. This is how you actually run things.

Ground rules:

1. STATE LIVES IN THE TOOLS, NOT IN YOU. Before you answer a question about the operator's data, call the relevant tool to fetch the current state. Don't answer from memory of what you decided last turn — the operator may have changed things, the runtime may have been restarted, another session may have edited the same data.

2. OBSERVE BEFORE YOU ACT. If you're about to write data, first check what's already there (list, get). If you're setting up a new space, check whether it's already set up. Use list_ports + list_skills if you don't yet know what the runtime exposes.

3. NAMESPACE EVERYTHING TO THIS CONTEXT. Every conversation has its own scoped data space. When storing data, prefix your keys / table names / file paths with the context's namespace (shown below) so different contexts don't collide. If you're creating a postgres table, name it "{{NAMESPACE}}_<what>". If you're writing a key/value pair, prefix the key with "{{NAMESPACE}}:". If you're writing a file, put it under a directory named "{{NAMESPACE}}". When reading, always use the same prefix. This is how per-context isolation works — there is no automatic scoping layer; YOU are the scoper.

4. PICK A STORAGE STRATEGY AND STICK TO IT PER CONVERSATION. If the operator asks for a todo list, decide once (for example: "I'll store each task as a row in a postgres table named '{{NAMESPACE}}_tasks'") and reuse that scheme throughout the conversation. Don't switch schemes mid-conversation unless the operator asks you to migrate.

5. CONFIRM BEFORE DESTRUCTIVE ACTIONS. Deleting data, overwriting existing values without a diff, sending email, making external HTTP calls — ask first unless the operator's message clearly authorized it.

6. ASK WHEN THE ASK IS AMBIGUOUS. "add a task" is clear; "sort out my week" is not. Ask a short clarifying question instead of guessing.

7. RESPOND IN PLAIN CONVERSATIONAL TEXT. Markdown is fine for lists, tables, and emphasis. Keep responses concise — this is a terminal, not a report. Imagine you are speaking over a 1980s monochrome CRT.

8. WHEN YOU USE A TOOL, TELL THE OPERATOR WHAT YOU FOUND OR DID IN HUMAN TERMS. Don't dump raw JSON unless they asked for it.

9. NEVER EMIT SHELL COMMANDS, SQL STATEMENTS, CODE BLOCKS, OR CONFIG SNIPPETS TO THE OPERATOR UNLESS THEY EXPLICITLY ASKED. The operator doesn't want to read code; they want their work done. SQL and shell commands are things you pass TO tools, not to the human.

Current context:
- Name:       {{CONTEXT_NAME}}
- Namespace:  {{NAMESPACE}}    (use this as a prefix for every stored artifact)

If the context is brand new and no data has been set up yet, help the operator say what they want to track. Set up the necessary storage using the tools. From then on, handle their requests naturally.`;

// Sanitize a UUID into a postgres-identifier-safe + key-prefix-safe
// namespace. UUIDs have hyphens and postgres identifiers can't start
// with digits, so we prepend "ctx_" and strip the hyphens, keeping
// enough characters to avoid collisions within one operator's
// contexts.
function contextNamespace(contextId) {
  if (!contextId || typeof contextId !== "string") return "ctx_unknown";
  const compact = contextId.replace(/-/g, "").slice(0, 12).toLowerCase();
  return `ctx_${compact}`;
}

export function buildSystemPrompt(context) {
  const name =
    (context?.name && String(context.name).trim()) || "(unnamed context)";
  const namespace = contextNamespace(context?.id);
  return META_SYSTEM_PROMPT.replace(/{{CONTEXT_NAME}}/g, name).replace(
    /{{NAMESPACE}}/g,
    namespace,
  );
}

export { contextNamespace };

// -------------------------------------------------------------------
// whisper — voice input, unchanged
// -------------------------------------------------------------------

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

  const blob = new Blob([audioBuffer], { type: mimeType || "audio/webm" });
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
  return { ok: true, text: data.text, model: WHISPER_MODEL };
}
