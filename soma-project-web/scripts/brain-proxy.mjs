#!/usr/bin/env node
// Brain proxy for soma-project-web.
//
// Listens on localhost:8787 (override with --port N) and handles
// POST /api/brain by forwarding the incoming prompt + port catalog
// to OpenAI (gpt-5-mini by default) via the official `openai` SDK,
// parsing the model's JSON response as a SOMA plan, and returning it
// to the browser.
//
// The browser points its "Brain endpoint" input at
// http://localhost:8787/api/brain (the UI persists this to
// localStorage) and the phase 1g flow becomes real: a prompt typed
// into the tab becomes a gpt-5-mini-generated multi-port execution
// against the wasm SOMA runtime.
//
// Wire contract — request:
//   { "prompt": "say hello to marcu",
//     "port_catalog": [
//       { "port_id": "dom", "namespace": "...", "kind": "Renderer",
//         "capabilities": [...] },
//       ...
//     ]
//   }
// Wire contract — response:
//   { "plan": [
//       { "port_id": "dom", "capability_id": "append_heading",
//         "input": { "text": "Hello marcu!", "level": 1 } },
//       ...
//     ],
//     "explanation": "Greet the user visually."
//   }
//
// Prerequisites (real mode):
//   npm install openai                      # already in package.json
//   export OPENAI_API_KEY=sk-...
//
// Run:
//   node scripts/brain-proxy.mjs             # gpt-5-mini, port 8787
//   node scripts/brain-proxy.mjs --fake      # no API key needed
//   node scripts/brain-proxy.mjs --port 9090 --model gpt-5
//
// Then in the browser set "Brain endpoint" to
//   http://localhost:8787/api/brain
// and hit Tab to persist it.

import http from "node:http";
import OpenAI from "openai";

const DEFAULT_PORT = 8787;
const DEFAULT_MODEL = "gpt-5-mini";
const REASONING_EFFORT = "low";

// Parse --port N / --model X / --fake flags. Simple scanner — no deps.
function parseArgs(argv) {
  const opts = { port: DEFAULT_PORT, model: DEFAULT_MODEL, fake: false };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === "--port" && argv[i + 1]) {
      opts.port = Number(argv[++i]);
    } else if (flag === "--model" && argv[i + 1]) {
      opts.model = argv[++i];
    } else if (flag === "--fake") {
      opts.fake = true;
    } else if (flag === "--help" || flag === "-h") {
      process.stderr.write(
        "usage: brain-proxy.mjs [--port N] [--model MODEL] [--fake]\n" +
          "\n" +
          "  --port N     listen on localhost:N (default 8787)\n" +
          "  --model M    OpenAI model (default gpt-5-mini)\n" +
          "  --fake       return a canned plan without calling OpenAI.\n" +
          "               Useful for testing the full browser → proxy →\n" +
          "               browser round trip without an OPENAI_API_KEY.\n",
      );
      process.exit(0);
    }
  }
  return opts;
}

// Deterministic plan used when --fake is passed. Echoes the prompt
// back through the dom + audio ports so the user can visually confirm
// the whole round trip (browser → HTTP → proxy → HTTP → wasm → DOM)
// works before plugging in a real OpenAI key.
function fakePlan(prompt) {
  return {
    plan: [
      {
        port_id: "dom",
        capability_id: "append_heading",
        input: { text: `(fake brain) ${prompt}`, level: 1 },
      },
      {
        port_id: "audio",
        capability_id: "say_text",
        input: { text: prompt },
      },
    ],
    explanation:
      "Fake brain: echoed the prompt as a heading and spoke it aloud. " +
      "Run without --fake (and with OPENAI_API_KEY) to get real gpt-5-mini plans.",
  };
}

// System prompt: locks the model into emitting ONLY the plan JSON.
// OpenAI's `response_format: { type: "json_object" }` guarantees the
// response is valid JSON, so we only have to worry about SHAPE, not
// syntax. The word "JSON" appears in the prompt, which json_object
// mode requires.
const SYSTEM_PROMPT = `You are the brain of a SOMA runtime running inside a browser tab. The runtime is a body with a set of loaded ports; each port exposes some capabilities. Your job: translate a natural-language user prompt into a structured JSON plan of port invocations that achieves the user's intent.

Output a single JSON object matching this exact shape:

{
  "plan": [
    {"port_id": "<string>", "capability_id": "<string>", "input": {<object matching the capability's input schema>}}
  ],
  "explanation": "<brief natural-language reason for this plan>"
}

Rules:
- Only use port_id + capability_id values that appear in the PORT CATALOG block below.
- "input" must match the capability's declared input schema. If a schema requires a field (e.g. "text"), provide it; if the field is optional, include it only when relevant.
- Prefer short plans (1-3 steps). Long plans only when the intent genuinely needs them.
- If the prompt cannot be satisfied by the available ports, return {"plan": [], "explanation": "why this is impossible with the given ports"}.
- Return ONLY the JSON object. No extra commentary.`;

function buildUserMessage(prompt, portCatalog) {
  const catalogBlock = JSON.stringify(portCatalog ?? [], null, 2);
  return `USER PROMPT:\n${prompt}\n\nPORT CATALOG:\n${catalogBlock}\n\nReturn the plan as JSON.`;
}

async function callOpenAI(client, model, prompt, portCatalog) {
  // gpt-5-mini is a reasoning model. We pass reasoning_effort: "low"
  // (cheapest / fastest internal thinking budget), rely on
  // response_format: { type: "json_object" } for guaranteed-valid JSON,
  // and DO NOT set temperature or max_tokens — per instructions.
  const response = await client.chat.completions.create({
    model,
    reasoning_effort: REASONING_EFFORT,
    response_format: { type: "json_object" },
    messages: [
      { role: "system", content: SYSTEM_PROMPT },
      { role: "user", content: buildUserMessage(prompt, portCatalog) },
    ],
  });

  const content = response.choices?.[0]?.message?.content;
  if (!content) {
    throw new Error("OpenAI returned empty content");
  }

  let parsed;
  try {
    parsed = JSON.parse(content);
  } catch (e) {
    throw new Error(
      `OpenAI returned non-JSON output: ${e.message}\nraw: ${content}`,
    );
  }
  if (!Array.isArray(parsed.plan)) {
    throw new Error(
      `OpenAI response missing plan array. Got: ${JSON.stringify(parsed)}`,
    );
  }
  return parsed;
}

// ---- HTTP server ------------------------------------------------------------

function corsHeaders() {
  return {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type",
  };
}

function send(res, status, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(body),
    ...corsHeaders(),
  });
  res.end(body);
}

async function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (chunk) => chunks.push(chunk));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

function makeServer(client, model, fake) {
  return http.createServer(async (req, res) => {
    // CORS preflight.
    if (req.method === "OPTIONS") {
      res.writeHead(204, corsHeaders());
      res.end();
      return;
    }

    if (req.method !== "POST" || req.url !== "/api/brain") {
      send(res, 404, { error: "not found — POST /api/brain" });
      return;
    }

    let parsed;
    try {
      const body = await readBody(req);
      parsed = body ? JSON.parse(body) : {};
    } catch (e) {
      send(res, 400, { error: `invalid JSON: ${e.message}` });
      return;
    }

    const { prompt, port_catalog: portCatalog } = parsed;
    if (typeof prompt !== "string" || prompt.trim() === "") {
      send(res, 400, { error: "missing or empty `prompt`" });
      return;
    }

    process.stderr.write(
      `[brain-proxy] ${new Date().toISOString()}  ` +
        `prompt="${prompt}"  ` +
        `ports=${Array.isArray(portCatalog) ? portCatalog.length : 0}\n`,
    );

    try {
      const plan = fake
        ? fakePlan(prompt)
        : await callOpenAI(client, model, prompt, portCatalog);
      process.stderr.write(
        `[brain-proxy]   → ${plan.plan.length}-step plan  ` +
          `"${(plan.explanation || "").slice(0, 80)}"\n`,
      );
      send(res, 200, plan);
    } catch (err) {
      process.stderr.write(`[brain-proxy]   ✗ ${err.message}\n`);
      send(res, 500, { error: err.message });
    }
  });
}

// ---- entry point ------------------------------------------------------------

function main() {
  const { port, model, fake } = parseArgs(process.argv.slice(2));

  let client = null;
  if (!fake) {
    const apiKey = process.env.OPENAI_API_KEY;
    if (!apiKey) {
      process.stderr.write(
        "error: OPENAI_API_KEY not set.\n" +
          "  export OPENAI_API_KEY=sk-...\n" +
          "or pass --fake to run without an LLM.\n",
      );
      process.exit(1);
    }
    client = new OpenAI({ apiKey });
  }

  const server = makeServer(client, model, fake);

  server.listen(port, "127.0.0.1", () => {
    const mode = fake
      ? "FAKE (no LLM)"
      : `OpenAI ${model} (reasoning_effort=${REASONING_EFFORT})`;
    process.stderr.write(
      `[brain-proxy] listening on http://127.0.0.1:${port}/api/brain\n` +
        `[brain-proxy] mode: ${mode}\n` +
        `[brain-proxy] browser: set "Brain endpoint" to\n` +
        `[brain-proxy]   http://localhost:${port}/api/brain\n`,
    );
  });

  const shutdown = () => {
    process.stderr.write("\n[brain-proxy] shutting down\n");
    server.close(() => process.exit(0));
    setTimeout(() => process.exit(1), 2000).unref();
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main();
