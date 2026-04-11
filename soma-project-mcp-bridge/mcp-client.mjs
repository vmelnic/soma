#!/usr/bin/env node
// Minimal MCP stdio client for soma-project-mcp-bridge.
//
// Spawns scripts/run-mcp.sh (which runs `soma --mcp --pack packs/hello/manifest.json`),
// drives it over JSON-RPC 2.0 on stdio, and exercises invoke_port against the
// `hello` port — which is itself another MCP server that soma-next spawned as a
// child process. Two chained stdio bridges:
//
//     mcp-client.mjs <-stdio-> soma-next <-stdio-> servers/hello_py/server.py
//        (brain)                (body)             (Bridge-A port)
//
// Usage:
//   node mcp-client.mjs skills                    # list tools on the soma MCP server
//   node mcp-client.mjs list_ports                # show the hello port + its capabilities
//   node mcp-client.mjs greet --name marcu        # invoke the greet tool
//   node mcp-client.mjs reverse --text 'hello'    # invoke the reverse tool
//   node mcp-client.mjs smoke                     # run all of the above and assert

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const projectRoot = path.dirname(fileURLToPath(import.meta.url));
const runMcpScript = path.join(projectRoot, "scripts", "run-mcp.sh");

class StdioMcpClient {
  constructor(command, args) {
    this.command = command;
    this.args = args;
    this.nextId = 1;
    this.pending = new Map();
  }

  async start() {
    this.child = spawn(this.command, this.args, {
      cwd: projectRoot,
      stdio: ["pipe", "pipe", "pipe"],
    });

    this.child.stderr.on("data", (chunk) => {
      process.stderr.write(chunk);
    });

    this.child.on("exit", (code) => {
      for (const { reject } of this.pending.values()) {
        reject(new Error(`soma MCP server exited with code ${code}`));
      }
      this.pending.clear();
    });

    const rl = readline.createInterface({ input: this.child.stdout });
    rl.on("line", (line) => {
      if (!line.trim()) return;
      let payload;
      try {
        payload = JSON.parse(line);
      } catch (error) {
        return;
      }
      const id = String(payload.id);
      const pending = this.pending.get(id);
      if (!pending) return;
      this.pending.delete(id);
      if (payload.error) {
        pending.reject(new Error(payload.error.message));
      } else {
        pending.resolve(payload.result);
      }
    });

    await this.request("initialize", {});
  }

  request(method, params) {
    const id = String(this.nextId++);
    const msg = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(`${JSON.stringify(msg)}\n`);
    });
  }

  async callTool(name, args) {
    return this.request("tools/call", { name, arguments: args });
  }

  async close() {
    if (!this.child) return;
    this.child.stdin.end();
    this.child.kill();
  }
}

// The soma MCP server wraps tool results in { content: [{type:"text", text:"..."}] }.
// Unwrap that to the actual PortCallRecord JSON when it's present.
function unwrap(result) {
  if (
    result &&
    Array.isArray(result.content) &&
    result.content[0]?.type === "text"
  ) {
    try {
      return JSON.parse(result.content[0].text);
    } catch {
      return result.content[0].text;
    }
  }
  return result;
}

async function invokePort(client, portId, capabilityId, input) {
  const raw = await client.callTool("invoke_port", {
    port_id: portId,
    capability_id: capabilityId,
    input,
  });
  return unwrap(raw);
}

const LANG_PORTS = [
  { id: "hello_py",   label: "python" },
  { id: "hello_node", label: "node"   },
  { id: "hello_php",  label: "php"    },
];

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let i = 0; i < rest.length; i += 1) {
    const part = rest[i];
    if (!part.startsWith("--")) continue;
    const key = part.slice(2);
    const next = rest[i + 1];
    if (!next || next.startsWith("--")) {
      options[key] = true;
      continue;
    }
    options[key] = next;
    i += 1;
  }
  return { command, options };
}

function pretty(value) {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

async function run() {
  if (!existsSync(runMcpScript)) {
    throw new Error("missing scripts/run-mcp.sh");
  }

  const { command, options } = parseArgs(process.argv.slice(2));
  const client = new StdioMcpClient(runMcpScript, []);
  await client.start();

  try {
    switch (command) {
      case "skills": {
        pretty(unwrap(await client.request("tools/list", {})));
        break;
      }
      case "list_ports": {
        pretty(unwrap(await client.callTool("list_ports", {})));
        break;
      }
      case "greet": {
        const portId = options.port ?? "hello_py";
        const name = options.name ?? "world";
        pretty(await invokePort(client, portId, "greet", { name }));
        break;
      }
      case "reverse": {
        const portId = options.port ?? "hello_py";
        const text = options.text ?? "hello";
        pretty(await invokePort(client, portId, "reverse", { text }));
        break;
      }
      case "smoke": {
        const results = {};
        for (const { id, label } of LANG_PORTS) {
          const greet = await invokePort(client, id, "greet", { name: "marcu" });
          if (!greet.success) {
            throw new Error(`${id} greet failed: ${JSON.stringify(greet, null, 2)}`);
          }
          const expected = `hello marcu! (from ${label})`;
          if (greet.structured_result?.message !== expected) {
            throw new Error(
              `${id} greet: expected ${JSON.stringify(expected)}, got ${JSON.stringify(greet.structured_result)}`,
            );
          }

          const reverse = await invokePort(client, id, "reverse", { text: "hello marcu!" });
          if (!reverse.success) {
            throw new Error(`${id} reverse failed: ${JSON.stringify(reverse, null, 2)}`);
          }
          if (reverse.structured_result?.reversed !== "!ucram olleh") {
            throw new Error(
              `${id} reverse: unexpected output ${JSON.stringify(reverse.structured_result)}`,
            );
          }

          results[label] = {
            greet: greet.structured_result,
            reverse: reverse.structured_result,
          };
        }
        pretty({ ports: results, ok: true });
        break;
      }
      default:
        throw new Error(
          "usage: node mcp-client.mjs <skills|list_ports|greet|reverse|smoke> [--port hello_py|hello_node|hello_php] [--name <...>] [--text <...>]",
        );
    }
  } finally {
    await client.close();
  }
}

run().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
