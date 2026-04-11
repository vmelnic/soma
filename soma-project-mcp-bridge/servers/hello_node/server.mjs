#!/usr/bin/env node
// Minimal MCP stdio server exposing greet + reverse tools.
//
// Pure Node.js stdlib — no `@modelcontextprotocol/sdk` dependency so this
// proof project works with any Node 18+. The greet response includes
// "(from node)" so the smoke test can tell the three language servers
// apart when they run side by side.

import { createInterface } from "node:readline";

const PROTOCOL_VERSION = "2024-11-05";

const TOOLS = [
  {
    name: "greet",
    description: "Return a personalized greeting for the given name.",
    inputSchema: {
      type: "object",
      required: ["name"],
      properties: {
        name: { type: "string", description: "Who to greet." },
      },
    },
  },
  {
    name: "reverse",
    description: "Reverse the given text.",
    inputSchema: {
      type: "object",
      required: ["text"],
      properties: {
        text: { type: "string" },
      },
    },
  },
];

function respond(id, result, error) {
  const msg = { jsonrpc: "2.0", id };
  if (error !== undefined) {
    msg.error = error;
  } else {
    msg.result = result;
  }
  process.stdout.write(`${JSON.stringify(msg)}\n`);
}

function handle(req) {
  const method = req.method ?? "";
  const id = req.id;
  const params = req.params ?? {};

  if (method === "initialize") {
    respond(id, {
      protocolVersion: PROTOCOL_VERSION,
      capabilities: { tools: {} },
      serverInfo: { name: "hello-mcp-node", version: "0.1.0" },
    });
    return;
  }

  if (method === "notifications/initialized") {
    return;
  }

  if (method === "tools/list") {
    respond(id, { tools: TOOLS });
    return;
  }

  if (method === "tools/call") {
    const name = params.name;
    const args = params.arguments ?? {};

    if (name === "greet") {
      const who = args.name ?? "stranger";
      const payload = { message: `hello ${who}! (from node)` };
      respond(id, {
        content: [{ type: "text", text: JSON.stringify(payload) }],
        isError: false,
      });
      return;
    }

    if (name === "reverse") {
      const text = args.text ?? "";
      // Use the spread operator so code-point reversal handles surrogates
      // more sensibly than plain .split("").reverse().join("").
      const reversed = [...text].reverse().join("");
      const payload = { reversed };
      respond(id, {
        content: [{ type: "text", text: JSON.stringify(payload) }],
        isError: false,
      });
      return;
    }

    respond(id, undefined, { code: -32601, message: `unknown tool: ${name}` });
    return;
  }

  if (id !== undefined && id !== null) {
    respond(id, undefined, { code: -32601, message: `unknown method: ${method}` });
  }
}

const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  try {
    handle(JSON.parse(trimmed));
  } catch (e) {
    process.stderr.write(`[hello-mcp-node] invalid JSON: ${e.message}\n`);
  }
});
