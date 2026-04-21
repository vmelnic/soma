#!/usr/bin/env node
/**
 * soma-project-code — Haiku brain for coding via SOMA.
 *
 * LLM-driven path: brain decomposes goal, invokes ports directly via MCP.
 * No autonomous control loop. Brain decides, body executes.
 *
 * Usage:
 *   node brain.js "Build an Express API with user CRUD and SQLite"
 */

import { spawn } from "node:child_process";
import { existsSync, readdirSync, mkdirSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const projectRoot = path.dirname(fileURLToPath(import.meta.url));

// ---------------------------------------------------------------------------
// Env
// ---------------------------------------------------------------------------

function parseDotEnv(content) {
  const env = {};
  for (const rawLine of content.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq === -1) continue;
    const key = line.slice(0, eq).trim();
    let value = line.slice(eq + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    )
      value = value.slice(1, -1);
    env[key] = value;
  }
  return env;
}

let envCache;
async function loadEnv() {
  if (envCache) return envCache;
  const p = path.join(projectRoot, ".env");
  const entries = existsSync(p)
    ? parseDotEnv(await readFile(p, "utf8"))
    : {};
  envCache = { ...entries, ...process.env };
  return envCache;
}

// ---------------------------------------------------------------------------
// Stdio MCP client (uses spawn, not shell execution)
// ---------------------------------------------------------------------------

class StdioMcpClient {
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
      try {
        payload = JSON.parse(line);
      } catch {
        return;
      }
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
    if (this.child) {
      this.child.stdin.end();
      this.child.kill();
    }
  }
}

// ---------------------------------------------------------------------------
// LLM client
// ---------------------------------------------------------------------------

async function callLLM(systemPrompt, userPrompt) {
  const env = await loadEnv();
  const apiKey = env.ANTHROPIC_API_KEY;
  if (!apiKey) throw new Error("ANTHROPIC_API_KEY not set");
  const model = env.ANTHROPIC_MODEL || "claude-haiku-4-5-20251001";

  const res = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01",
    },
    body: JSON.stringify({
      model,
      max_tokens: 4096,
      system: systemPrompt,
      messages: [{ role: "user", content: userPrompt }],
    }),
  });

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Anthropic API ${res.status}: ${text}`);
  }

  const json = await res.json();
  return json.content[0].text;
}

function parseJsonResponse(text) {
  let cleaned = text.trim();
  if (cleaned.startsWith("```")) {
    cleaned = cleaned.replace(/^```(?:json)?\s*/, "").replace(/\s*```$/, "");
  }
  // Extract JSON array or object from prose if LLM didn't follow format
  if (!cleaned.startsWith("[") && !cleaned.startsWith("{")) {
    const arrMatch = cleaned.match(/\[[\s\S]*\]/);
    const objMatch = cleaned.match(/\{[\s\S]*\}/);
    if (arrMatch) cleaned = arrMatch[0];
    else if (objMatch) cleaned = objMatch[0];
  }
  return JSON.parse(cleaned);
}

// ---------------------------------------------------------------------------
// Brain — decompose goal, then drive ports step by step
// ---------------------------------------------------------------------------

const PLAN_SYSTEM = `You decompose coding goals into ordered steps. Each step is a port invocation.

Available ports and capabilities:
- filesystem: readdir(path), readfile(path), writefile(path,content), stat(path), mkdir(path), rm(path)
- git: init(cwd), status(cwd), diff(cwd), log(cwd), add(cwd,paths), commit(cwd,message)
- runner: exec(cwd,command), npm_install(cwd), npm_test(cwd)

Return a JSON array. Each entry:
{
  "description": "short description",
  "port_id": "filesystem|git|runner",
  "capability_id": "capability name",
  "input_template": { ...fields WITHOUT file content — use "<generate>" as placeholder for content... }
}

Rules:
- For writefile steps: set "content": "<generate>" — content will be generated separately
- All other fields must have real values (paths, commands, messages)
- The workspace path is provided — use it for "path" and "cwd" fields
- Order: mkdir → writefile(package.json) → npm_install → writefile(source files) → npm_test → git init → git add → git commit
- List ALL files the project needs
- Respond with ONLY the JSON array. Start with [ end with ]`;

const STEP_SYSTEM = `You are a coding brain. A previous step failed or needs adjustment. Given the step description, port, capability, and error, provide corrected input as a JSON object.

Rules:
- For writefile: provide COMPLETE file content — never partial
- Fix the specific error shown
- Respond with ONLY a JSON object mapping field names to values`;

async function decompose(objective, workspace) {
  const response = await callLLM(
    PLAN_SYSTEM,
    `Goal: ${objective}\nWorkspace: ${workspace}`
  );
  return parseJsonResponse(response);
}

async function invokePort(soma, portId, capabilityId, input) {
  const result = await soma.callTool("invoke_port", {
    port_id: portId,
    capability_id: capabilityId,
    input,
  });
  return extractToolContent(result);
}

async function retryStep(step, error, workspace) {
  const prompt = `Step: ${step.description}
Port: ${step.port_id} / ${step.capability_id}
Original input: ${JSON.stringify(step.input)}
Error: ${error}
Workspace: ${workspace}

Provide corrected input as JSON.`;

  const response = await callLLM(STEP_SYSTEM, prompt);
  return parseJsonResponse(response);
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  const objective = process.argv[2];
  if (!objective) {
    console.error('Usage: node brain.js "Build an Express API with user CRUD and SQLite"');
    process.exit(1);
  }

  const env = await loadEnv();
  const workspace = path.resolve(
    env.SOMA_CODE_WORKSPACE || path.join(projectRoot, "workspace")
  );
  mkdirSync(workspace, { recursive: true });

  console.log(`[brain] Workspace: ${workspace}`);
  console.log(`[brain] Model: ${env.ANTHROPIC_MODEL || "claude-haiku-4-5-20251001"}`);

  const somaPath = path.join(projectRoot, "bin/soma");
  if (!existsSync(somaPath)) {
    console.error(`Missing SOMA binary: ${somaPath}\nRun: bash build.sh`);
    process.exit(1);
  }

  const packArgs = [];
  const packsDir = path.join(projectRoot, "packs");
  for (const entry of readdirSync(packsDir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      const manifest = path.join(packsDir, entry.name, "manifest.json");
      if (existsSync(manifest)) packArgs.push("--pack", manifest);
    }
  }

  console.log(`[brain] Starting SOMA with ${packArgs.length / 2} packs...`);
  const soma = new StdioMcpClient(
    somaPath,
    ["--mcp", ...packArgs],
    projectRoot
  );
  await soma.start();
  console.log("[brain] SOMA connected.\n");

  try {
    // Phase 1: Decompose goal into steps
    console.log("[brain] Decomposing goal...");
    const steps = await decompose(objective, workspace);
    console.log(`[brain] ${steps.length} steps planned:\n`);
    for (let i = 0; i < steps.length; i++) {
      console.log(`  ${i + 1}. [${steps[i].port_id}.${steps[i].capability_id}] ${steps[i].description}`);
    }
    console.log();

    // Phase 2: Execute each step via invoke_port
    let succeeded = 0;
    let failed = 0;

    for (let i = 0; i < steps.length; i++) {
      const step = steps[i];
      const label = `[${i + 1}/${steps.length}]`;

      process.stdout.write(`${label} ${step.description}...`);

      try {
        const result = await invokePort(
          soma,
          step.port_id,
          step.capability_id,
          step.input
        );

        const record = result.result || result;
        if (record.success === false || (record.exit_code !== undefined && record.exit_code !== 0)) {
          const errMsg =
            record.error ||
            record.stderr ||
            record.raw_result?.error ||
            "unknown failure";
          console.log(` FAIL`);
          console.log(`       ${String(errMsg).slice(0, 200)}`);

          // Retry once with LLM correction
          console.log(`       retrying with LLM correction...`);
          try {
            const correctedInput = await retryStep(step, errMsg, workspace);
            const retryResult = await invokePort(
              soma,
              step.port_id,
              step.capability_id,
              correctedInput
            );
            const retryRecord = retryResult.result || retryResult;
            if (retryRecord.success === false || (retryRecord.exit_code !== undefined && retryRecord.exit_code !== 0)) {
              console.log(`       retry FAILED`);
              failed++;
            } else {
              console.log(`       retry OK`);
              succeeded++;
            }
          } catch (retryErr) {
            console.log(`       retry error: ${retryErr.message}`);
            failed++;
          }
        } else {
          console.log(` ok`);
          succeeded++;
        }
      } catch (err) {
        console.log(` ERROR: ${err.message}`);
        failed++;
      }
    }

    // Summary
    console.log(`\n[brain] Done: ${succeeded} succeeded, ${failed} failed out of ${steps.length} steps.`);

    // Show workspace contents
    try {
      const lsResult = await invokePort(soma, "filesystem", "readdir", {
        path: workspace,
      });
      const entries = lsResult.result?.entries || [];
      if (entries.length > 0) {
        console.log(`\n[brain] Workspace contents:`);
        for (const e of entries) {
          console.log(`  ${e.is_dir ? "d" : "f"} ${e.name}`);
        }
      }
    } catch {
      // not critical
    }
  } finally {
    soma.close();
  }
}

function extractToolContent(result) {
  if (result && Array.isArray(result.content)) {
    for (const c of result.content) {
      if (c.type === "text" && c.text) return JSON.parse(c.text);
    }
  }
  if (result && typeof result === "object" && !Array.isArray(result)) {
    return result;
  }
  throw new Error(`Unexpected tool result: ${JSON.stringify(result)}`);
}

main().catch((err) => {
  console.error(`[brain] Fatal: ${err.message}`);
  process.exit(1);
});
