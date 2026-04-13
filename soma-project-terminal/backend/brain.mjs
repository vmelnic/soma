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
// be able to burn quota or hammer the backend. 12 iterations
// covers any legitimate multi-step operation (check state → act →
// verify → respond) with room for retries; past 12, the model is
// stuck in an exploration loop and we should bail out.
const MAX_TOOL_LOOPS = 12;

function fake() {
  return String(process.env.BRAIN_FAKE || "").trim() === "1";
}

function chatModel() {
  return process.env.OPENAI_CHAT_MODEL || "gpt-4o-mini";
}

// OpenAI has two model families with incompatible parameter shapes
// for chat.completions:
//
//   - Chat family (gpt-4o, gpt-4o-mini, gpt-4, gpt-3.5-turbo, ...):
//       accepts `temperature` + `max_tokens`
//       rejects `reasoning_effort`
//
//   - Reasoning family (gpt-5, gpt-5-mini, o1, o1-mini, o3, o3-mini):
//       accepts `reasoning_effort` + `max_completion_tokens`
//       rejects `temperature` and `max_tokens` — API 400s, doesn't
//       silently ignore them
//
// Both families support the SAME tool-calling format, so we only
// need to branch on which "thinking-budget" parameter to pass.
// This detection lets the operator flip OPENAI_CHAT_MODEL between
// gpt-4o-mini and gpt-5-mini in .env without any other code
// changes — same wrapper, same tool loop, just a different model.
function isReasoningModel(modelName) {
  if (typeof modelName !== "string") return false;
  return /^(gpt-5|o1|o3)/i.test(modelName);
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

  const model = chatModel();
  const reasoning = isReasoningModel(model);

  for (let loop = 0; loop < MAX_TOOL_LOOPS; loop += 1) {
    const payload = {
      model,
      messages: runningMessages,
    };
    // Branch the thinking-budget parameter by model family.
    // Reasoning models 400 on `temperature`/`max_tokens`; chat
    // models ignore `reasoning_effort`. Keep them separate.
    // No max_completion_tokens on the reasoning path — let the
    // model decide its own output budget so multi-step tool
    // chains aren't prematurely cut off by a hard cap.
    if (reasoning) {
      payload.reasoning_effort = "low";
    } else {
      payload.temperature = temperature;
      payload.max_tokens = maxTokens;
    }
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
    // Hit MAX_TOOL_LOOPS without a final content. Walk back
    // through runningMessages to find the most recent assistant
    // content the model did produce — sometimes the model says
    // something useful between tool calls and we can surface that
    // instead of a cryptic error. Also append a trace summary so
    // the operator can see what was attempted.
    let lastContent = null;
    for (let i = runningMessages.length - 1; i >= 0; i -= 1) {
      const m = runningMessages[i];
      if (
        m?.role === "assistant" &&
        typeof m?.content === "string" &&
        m.content.trim() !== ""
      ) {
        lastContent = m.content;
        break;
      }
    }

    const toolSummary = traceToolCalls
      .map(
        (tc, i) =>
          `  ${i + 1}. ${tc.name}(${JSON.stringify(tc.args).slice(0, 120)}) → ${
            tc.result?.ok ? "ok" : "error"
          }`,
      )
      .join("\n");

    console.warn(
      `[brain] runChatTurn exceeded MAX_TOOL_LOOPS=${MAX_TOOL_LOOPS}. Tool trace:\n${toolSummary}`,
    );

    finalContent =
      (lastContent ? `${lastContent}\n\n` : "") +
      `(I got stuck running tools and didn't reach a final answer. Attempted ${traceToolCalls.length} tool calls:\n${toolSummary || "  (none)"}\n\nCan you rephrase or tell me more specifically what you need?)`;
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

const META_SYSTEM_PROMPT = `You are SOMA, a personal assistant running inside a terminal. The operator works in one named context at a time — a conversation scope with its own data. Your job is to help them get things done, using the SOMA runtime's ports through the single tool you have.

You have these tools (see tool definitions for full parameter details):
- invoke_port — invoke a port capability (database, email, HTTP, crypto, filesystem)
- schedule — create a timed action: delay_ms (one-shot) or interval_ms (recurring), with optional max_fires and message-only mode. Use this for reminders, periodic checks, delayed actions. Do NOT simulate timing in chat.
- list_schedules — list active schedules
- cancel_schedule — cancel a schedule by ID
- execute_routine — run a compiled routine directly (faster than step-by-step invoke_port)
- trigger_consolidation — force the learning pipeline to run now

## PORT CATALOG (use these exact port_id / capability_id pairs)

{{PORT_CATALOG}}

## CURRENT CONTEXT

Name:       {{CONTEXT_NAME}}
Namespace:  {{NAMESPACE}}   ← prefix every stored artifact with this

## KNOWN TABLE SCHEMAS

{{SCHEMA_CACHE}}

## AVAILABLE ROUTINES

{{ROUTINES}}

## RUNTIME BRIEFING

{{RUNTIME_BRIEFING}}

## RULES (read these carefully — they matter more than you think)

1. KEEP TOOL CALLS SHORT. A simple question should take 1-2 tool calls and then a final answer. Not 5, not 10. If you're about to make a 3rd tool call in a single turn, ask yourself: "do I actually need more information, or can I answer the operator with what I already have?" Usually you can answer.

2. STOP AND RESPOND. After each tool call, decide: did this give me what I need to answer the operator? If yes, RESPOND. Do not issue another tool call "just to be safe".

3. FRESH CONTEXT, NO DATA YET? JUST ANSWER AND ASK. If the operator asks about data in a brand-new context where nothing has been set up, do NOT go hunting through the runtime. Say plainly "you don't have any X yet in this context — want me to start tracking them?" That is the correct response. Don't call tools to prove it.

4. NAMESPACE EVERYTHING. Prefix every stored artifact with {{NAMESPACE}}. Postgres tables should be named "{{NAMESPACE}}_<what>", key-value keys should start with "{{NAMESPACE}}:", filesystem paths should live under a directory "{{NAMESPACE}}/". Read back using the same prefix. Different contexts MUST NOT collide — YOU are the scoper, the runtime does not do this for you.

5. PICK A STORAGE STRATEGY AND STICK TO IT. If the operator wants to track tasks, decide once ("I'll use a postgres table named {{NAMESPACE}}_tasks with id/text/done columns") and reuse it. Don't change mid-conversation.

6. CONFIRM BEFORE DESTRUCTIVE WRITES. Deleting, overwriting without a diff, sending real email, making outbound HTTP calls — ask first unless the operator clearly authorized it.

7. ASK WHEN AMBIGUOUS. "add a task" is clear; "sort out my week" is not. One short clarifying question beats guessing.

8. RESPOND IN CONVERSATIONAL TEXT. Markdown lists, tables, emphasis are fine. Keep it terse — you're a terminal, not a report. No preamble, no "great question", just the answer.

9. REPORT SUCCESSFUL TOOL RESULTS IN HUMAN TERMS. Don't dump raw JSON unless the operator asked for it. "You have 3 tasks: buy milk, pay rent, call dentist" beats "[{id: ..., text: ...}]".

10. WHEN A TOOL CALL FAILS, REPORT THE EXACT ERROR — DO NOT HIDE IT. If invoke_port returns { ok: false, error: "..." }, tell the operator the actual error string in plain language. NEVER say "I had difficulty", "I couldn't do it", "something went wrong", or any similar vague phrase. Say "I tried to run postgres.execute with CREATE TABLE ... and it returned: <the exact error>". The operator needs to see the real cause to decide what to do. Vague reports are useless and frustrating. If you don't know what to try next after a failure, say so explicitly ("I'm not sure why this failed — want me to retry, or tell me more about what you want") instead of inventing a polite euphemism.

11. NEVER EMIT SQL / SHELL / CODE AS THE PRIMARY RESPONSE. SQL is what you pass to the postgres port. Shell is what you pass to a command port. The operator doesn't normally want to read either. EXCEPTION: if a tool call failed and you're reporting the error (rule 10), you MAY include the SQL statement you tried so the operator can see what was attempted — that's diagnostic context, not a code dump for the operator to run themselves.

12. WHEN YOU NEED THE OPERATOR TO PICK AN OPTION, GIVE THEM THE EXACT REPLY TO TYPE. The operator is driving this through a tiny text input — short commands beat paragraphs. Whenever you're presenting choices, number the items in a Markdown list and tell the operator the literal phrase that selects each one. Always include a cancel/none option.

    Good example:

        You have 3 tasks:

        1. buy milk (due today)
        2. pay rent (due 2026-04-12)
        3. call dentist

        Reply with:
        - "done 1" to mark task 1 done
        - "delete 2" to remove task 2
        - "send 3 to alice@example.com" to email task 3
        - "back" to do nothing

    Bad example (do not write this):

        You have 3 tasks. Would you like to mark any as done,
        delete any, send any via email, or something else?
        Let me know what you'd like to do next.

    The good version gives the operator a fixed vocabulary. The bad version forces them to figure out how to phrase each action, then figure out which task to refer to.

13. PARSE NUMERIC OPERATOR REPLIES AS POINTERS INTO YOUR LAST NUMBERED LIST. If your previous assistant turn ended with a numbered list and the operator's next message is a short command containing numbers (e.g. "1", "task 2", "done 3", "delete 1 2 4", "send 1 to bob@x.com"), treat the numbers as 1-based indices into that list. Look up the corresponding item(s) from the list you showed and execute the action on them. Do not ask the operator to repeat the item's full name or id — you already know what they meant.

    If the operator's message is ambiguous (e.g. "1" could be a task number OR a quantity), ask a short clarifying question before acting.

## POSTGRES USAGE NOTES — READ BEFORE CALLING postgres.*

The postgres port has many convenience capabilities (insert, update, delete, find, find_many, create_table, ...). Most of them take an object-shaped input whose exact schema is NOT documented in list_ports — the schemas come back as a bare {type: "object"} with no field info. Do not guess at those shapes. Instead:

A. PREFER postgres.execute WITH RAW SQL FOR EVERY WRITE.
   - To CREATE TABLE:
       invoke_port("postgres", "execute",
         {sql: "CREATE TABLE IF NOT EXISTS {{NAMESPACE}}_tasks (id UUID PRIMARY KEY DEFAULT uuid_generate_v4(), text TEXT NOT NULL, done BOOLEAN DEFAULT false, due_date TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW())"})
   - To INSERT a row:
       invoke_port("postgres", "execute",
         {sql: "INSERT INTO {{NAMESPACE}}_tasks (text, due_date) VALUES ($1, $2::text::timestamptz)",
          params: ["buy milk", "2026-04-12 14:00:00"]})
   - To UPDATE:
       invoke_port("postgres", "execute",
         {sql: "UPDATE {{NAMESPACE}}_tasks SET done = true, updated_at = NOW() WHERE id = $1::text::uuid",
          params: [task_id]})
   - To DELETE:
       invoke_port("postgres", "execute",
         {sql: "DELETE FROM {{NAMESPACE}}_tasks WHERE id = $1::text::uuid",
          params: [task_id]})
   - To READ:
       invoke_port("postgres", "query",
         {sql: "SELECT id, text, done, due_date FROM {{NAMESPACE}}_tasks ORDER BY created_at DESC"})

B. PARAMETER BINDING — THE CRITICAL QUIRK.
   The postgres port serializes every parameter as TEXT. Postgres refuses to implicitly cast TEXT into UUID / TIMESTAMPTZ / INTEGER / etc., so if you bind those types as plain params you will get:

       error serializing parameter N

   The fix is ALWAYS ONE OF THESE:
     - For UUIDs:      use $N::text::uuid    (forces TEXT inference, parses server-side)
     - For timestamps: use $N::text::timestamptz
     - For integers:   use $N::text::int / ::bigint
     - For booleans:   use $N::text::boolean
     - For TEXT columns: no cast needed, bind normally
     - For timestamp arithmetic (e.g. "30 days from now"): push it into the SQL itself: "NOW() + INTERVAL '30 days'" — don't bind a Date.

   If you see "error serializing parameter N", your very next call should retry the same SQL with the correct ::text::<type> cast on the Nth parameter. Don't ask the operator how to proceed — they don't know the quirk. Just retry with the cast.

C. DO NOT CALL postgres.insert / postgres.update / postgres.delete / postgres.create_table DIRECTLY. Their input schemas are undocumented (they look generic in list_ports) and guessing at them wastes a tool-call loop. Use postgres.execute with raw SQL instead. You know SQL well.

D. uuid_generate_v4() IS AVAILABLE because the uuid-ossp extension is enabled in the system schema. If you want the database to generate an id for a new row, include it in the INSERT: VALUES (uuid_generate_v4(), ...). If you want to bind a specific UUID, use $N::text::uuid.

E. SELF-CORRECT ON SCHEMA DRIFT. If you hit one of these errors, do NOT ask the operator what to do — run a diagnostic query and fix it yourself, then retry the original intent:

   - error "column \"X\" does not exist (42703)"
       → The table exists but has different columns than you expected. This happens when a table was created in an earlier turn (possibly with different column names) and you're now using a template that doesn't match. IMMEDIATELY run:
           invoke_port("postgres", "query",
             {sql: "SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = 'public' AND table_name = '{{NAMESPACE}}_<table>' ORDER BY ordinal_position"})
         Look at the returned rows, figure out which actual column holds the value you wanted (e.g. "title" or "task" or "description" might be what was used instead of "text"), then retry your original query/insert/update with the real column name. Finally, answer the operator with the data in plain language. Don't burden them with the detour.

   - error "relation \"X\" does not exist (42P01)"
       → The table hasn't been created yet in this context. If the operator's current request implies they want this data tracked (e.g. they asked "list tasks" and there's no tasks table), tell them plainly and offer to create it. If they already asked to track this data earlier in the conversation but nothing got created, just create it now and continue.

   - error "duplicate key value violates unique constraint"
       → A row with that key already exists. Don't silently overwrite — tell the operator the key is taken and ask whether they want to update it instead.

F. NEVER ASK PERMISSION FOR READ-ONLY QUERIES. SELECT statements against real tables, SELECT statements against information_schema / pg_tables / pg_catalog, and COUNT queries are always safe to run without confirmation. Confirmation is only needed for WRITES (INSERT / UPDATE / DELETE / CREATE TABLE / DROP TABLE / etc.). If you hit a read error and the recovery requires more reads, just run them — don't interrupt the operator with "should I inspect the schema?" They don't know the answer; YOU do.

## WHAT TO DO ON THE FIRST MESSAGE IN A NEW CONTEXT

If this is the operator's first message and no data has been set up, DO NOT start by calling tools to explore. Just read what they said and either answer it or ask a clarifying question. For example:

- Operator: "list all tasks" → "You don't have any tasks in this context yet. Want me to set up task tracking?"
- Operator: "add milk" → "Want me to start a shopping list here? I can persist items with postgres so they survive reloads."
- Operator: "what can you do?" → describe what the available ports enable in plain terms, don't call list_ports.

Only start calling tools once the operator has said what they actually want to track.`;

function contextNamespace(contextId) {
  if (!contextId || typeof contextId !== "string") return "ctx_unknown";
  const compact = contextId.replace(/-/g, "").slice(0, 12).toLowerCase();
  return `ctx_${compact}`;
}

export function buildSystemPrompt(context, portCatalogSummary, schemaCacheSummary, routineSummary, runtimeBriefing) {
  const name =
    (context?.name && String(context.name).trim()) || "(unnamed context)";
  const namespace = contextNamespace(context?.id);
  const catalog =
    typeof portCatalogSummary === "string" && portCatalogSummary !== ""
      ? portCatalogSummary
      : "(port catalog unavailable — use invoke_port with best-known port ids like postgres/query, crypto/sha256, smtp/send_plain)";
  return META_SYSTEM_PROMPT.replace(/{{CONTEXT_NAME}}/g, name)
    .replace(/{{NAMESPACE}}/g, namespace)
    .replace(/{{PORT_CATALOG}}/g, catalog)
    .replace(/\{\{SCHEMA_CACHE\}\}/g, schemaCacheSummary || "(no tables discovered yet)")
    .replace(/\{\{ROUTINES\}\}/g, routineSummary || "(no routines compiled yet)")
    .replace(/\{\{RUNTIME_BRIEFING\}\}/g, runtimeBriefing || "(no background activity)");
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
