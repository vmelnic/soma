/**
 * Stdio MCP client — spawns SOMA binary and communicates via JSON-RPC.
 * Extracted as shared module (used by plan.js, execute.js, prove.js).
 */

import { spawn } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { projectRoot } from "./env.js";

export class StdioMcpClient {
  constructor(command, args, cwd) {
    this.command = command;
    this.args = args;
    this.cwd = cwd;
    this.nextId = 1;
    this.pending = new Map();
  }

  async start() {
    this.child = spawn(this.command, this.args, {
      cwd: this.cwd,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child.stderr.on("data", (chunk) => process.stderr.write(chunk));
    this.child.on("exit", (code) => {
      for (const { reject } of this.pending.values())
        reject(new Error(`SOMA exited with code ${code}`));
      this.pending.clear();
    });
    const rl = readline.createInterface({ input: this.child.stdout });
    rl.on("line", (line) => {
      if (!line.trim()) return;
      let payload;
      try { payload = JSON.parse(line); } catch { return; }
      if (payload.method && payload.id === undefined) return;
      const entry = this.pending.get(String(payload.id));
      if (!entry) return;
      this.pending.delete(String(payload.id));
      if (payload.error) entry.reject(new Error(payload.error.message));
      else entry.resolve(payload.result);
    });
    await this.request("initialize", {});
  }

  request(method, params) {
    const id = String(this.nextId++);
    const req = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(`${JSON.stringify(req)}\n`);
    });
  }

  async callTool(name, args) {
    return this.request("tools/call", { name, arguments: args });
  }

  close() {
    if (this.child) { this.child.stdin.end(); this.child.kill(); }
  }
}

export function extractToolContent(result) {
  if (result && Array.isArray(result.content)) {
    for (const c of result.content) {
      if (c.type === "text" && c.text) return JSON.parse(c.text);
    }
  }
  if (result && typeof result === "object" && !Array.isArray(result))
    return result;
  throw new Error(`Unexpected tool result: ${JSON.stringify(result)}`);
}

export async function invokePort(soma, portId, capabilityId, input) {
  const result = await soma.callTool("invoke_port", {
    port_id: portId,
    capability_id: capabilityId,
    input,
  });
  return extractToolContent(result);
}

export function connectSoma() {
  const somaPath = path.join(projectRoot, "bin/soma");
  if (!existsSync(somaPath)) {
    throw new Error(`Missing SOMA binary: ${somaPath}\nRun: bash build.sh`);
  }

  const packArgs = [];
  const packsDir = path.join(projectRoot, "packs");
  if (existsSync(packsDir)) {
    for (const entry of readdirSync(packsDir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        const manifest = path.join(packsDir, entry.name, "manifest.json");
        if (existsSync(manifest)) packArgs.push("--pack", manifest);
      }
    }
  }

  return new StdioMcpClient(somaPath, ["--mcp", ...packArgs], projectRoot);
}
