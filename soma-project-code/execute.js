#!/usr/bin/env node
/**
 * execute.js — Execute a plan.json through SOMA ports.
 *
 * For each writefile step, asks Haiku for the file content.
 * For all other steps, invokes the port directly with the plan's input.
 *
 * Usage:
 *   node execute.js [plan.json]
 */

import { spawn } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
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
  const entries = existsSync(p) ? parseDotEnv(await readFile(p, "utf8")) : {};
  envCache = { ...entries, ...process.env };
  return envCache;
}

// ---------------------------------------------------------------------------
// Stdio MCP client (uses spawn — no shell)
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

// ---------------------------------------------------------------------------
// LLM
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
      max_tokens: 8192,
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

// ---------------------------------------------------------------------------
// Content generation for writefile steps
// ---------------------------------------------------------------------------

const WRITE_SYSTEM = `You generate file content for a coding project. You will be told:
- The overall project goal
- Which file to write and its purpose
- Content of previously written files (you MUST use consistent imports, module names, and APIs)

Rules:
- Use CommonJS (require/module.exports). Do NOT use ES module syntax.
- If the code needs a directory at runtime (e.g. for a database file), create it with fs.mkdirSync(path, { recursive: true }).
- Write COMPLETE production code. No TODOs, no placeholders, no stubs.
- Respond with ONLY the file content — no markdown fences, no explanation.`;

async function generateFileContent(objective, filePath, description, writtenFiles) {
  let context = "";
  if (writtenFiles.length > 0) {
    const snippets = [];
    for (const { path: fp, content } of writtenFiles) {
      if (fp.endsWith(".md") || fp.endsWith(".gitignore")) continue;
      snippets.push(`--- ${path.basename(fp)} ---\n${content}`);
    }
    context = `\nPreviously written files (use these EXACT exports/APIs):\n${snippets.join("\n\n")}`;
  }

  const fileList = writtenFiles.map(f => `  ${f.path}`).join("\n");
  const layout = fileList ? `\nProject file layout:\n${fileList}\n  ${filePath} (this file)` : "";

  const prompt = `Project goal: ${objective}
File to write: ${filePath}
Purpose: ${description}${layout}${context}

Write the complete file content:`;

  return callLLM(WRITE_SYSTEM, prompt);
}

// ---------------------------------------------------------------------------
// Port invocation
// ---------------------------------------------------------------------------

async function invokePort(soma, portId, capabilityId, input) {
  const result = await soma.callTool("invoke_port", {
    port_id: portId,
    capability_id: capabilityId,
    input,
  });
  return extractToolContent(result);
}

async function reconcileDeps(soma, workspace, writtenFiles) {
  const pkgFile = writtenFiles.find(f => f.path.endsWith("package.json"));
  if (!pkgFile) return;

  let pkg;
  try { pkg = JSON.parse(pkgFile.content); } catch { return; }
  const declared = new Set([
    ...Object.keys(pkg.dependencies || {}),
    ...Object.keys(pkg.devDependencies || {}),
  ]);

  const missing = new Set();
  for (const { path: fp, content } of writtenFiles) {
    if (!fp.endsWith(".js")) continue;
    for (const m of content.matchAll(/require\(['"]([^./][^'"]*)['"]\)/g)) {
      const mod = m[1].startsWith("@") ? m[1] : m[1].split("/")[0];
      if (!declared.has(mod) && !isBuiltin(mod)) missing.add(mod);
    }
  }

  if (missing.size === 0) return;

  const pkgDir = path.dirname(pkgFile.path);
  const names = [...missing].join(" ");
  console.log(`[exec] reconciling deps: installing ${names}`);
  try {
    await invokePort(soma, "runner", "exec", {
      cwd: pkgDir,
      command: `npm install --save-dev ${names}`,
      timeout_ms: 60000,
    });
  } catch (err) {
    console.log(`[exec] dep install failed: ${err.message}`);
  }
}

function isBuiltin(mod) {
  const builtins = new Set([
    "assert", "buffer", "child_process", "cluster", "crypto", "dgram",
    "dns", "events", "fs", "http", "http2", "https", "net", "os", "path",
    "perf_hooks", "process", "querystring", "readline", "stream",
    "string_decoder", "timers", "tls", "tty", "url", "util", "v8",
    "vm", "worker_threads", "zlib",
  ]);
  return builtins.has(mod) || mod.startsWith("node:");
}

function extractToolContent(result) {
  if (result && Array.isArray(result.content)) {
    for (const c of result.content) {
      if (c.type === "text" && c.text) return JSON.parse(c.text);
    }
  }
  if (result && typeof result === "object" && !Array.isArray(result))
    return result;
  throw new Error(`Unexpected tool result: ${JSON.stringify(result)}`);
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  const planFile = process.argv[2] || path.join(projectRoot, "plan.json");
  if (!existsSync(planFile)) {
    console.error(`Plan not found: ${planFile}\nRun: node plan.js "your goal"`);
    process.exit(1);
  }

  const plan = JSON.parse(await readFile(planFile, "utf8"));
  console.log(`[exec] Goal: ${plan.objective}`);
  console.log(`[exec] Workspace: ${plan.workspace}`);
  console.log(`[exec] ${plan.steps.length} steps to execute\n`);

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

  const soma = new StdioMcpClient(
    somaPath,
    ["--mcp", ...packArgs],
    projectRoot
  );
  await soma.start();
  console.log("[exec] SOMA connected.\n");

  let succeeded = 0;
  let failed = 0;
  const writtenFiles = [];
  let depsReconciled = false;

  try {
    for (let i = 0; i < plan.steps.length; i++) {
      const step = plan.steps[i];
      const label = `[${i + 1}/${plan.steps.length}]`;

      // Before npm_test, reconcile any missing deps
      if (!depsReconciled && step.capability_id === "npm_test" && writtenFiles.length > 0) {
        depsReconciled = true;
        await reconcileDeps(soma, plan.workspace, writtenFiles);
      }

      // For writefile: ensure parent dir exists, generate content via LLM
      if (step.capability_id === "writefile" && !step.input.content) {
        const filePath = step.input.path;
        process.stdout.write(`${label} generating ${path.basename(filePath)}...`);

        try {
          // Auto-create parent directory
          const parentDir = path.dirname(filePath);
          await invokePort(soma, "filesystem", "mkdir", { path: parentDir }).catch(() => {});

          let content = await generateFileContent(
            plan.objective,
            filePath,
            step.description,
            writtenFiles
          );
          // Strip markdown fences if present
          if (content.startsWith("```")) {
            content = content.replace(/^```(?:\w+)?\s*\n?/, "").replace(/\n?```\s*$/, "");
          }

          const result = await invokePort(soma, "filesystem", "writefile", {
            path: filePath,
            content,
          });
          const record = result.result || result;
          if (record.success === false) {
            console.log(` FAIL: ${record.error || JSON.stringify(record)}`);
            failed++;
          } else {
            console.log(` ok (${content.split("\n").length} lines)`);
            writtenFiles.push({ path: filePath, content });
            succeeded++;
          }
        } catch (err) {
          console.log(` ERROR: ${err.message}`);
          failed++;
        }
        continue;
      }

      // All other steps: invoke directly
      process.stdout.write(`${label} ${step.description}...`);

      try {
        const result = await invokePort(
          soma,
          step.port_id,
          step.capability_id,
          step.input
        );

        const record = result.result || result;
        const inner = record.structured_result || record.raw_result || record;
        const exitCode = inner.exit_code ?? record.exit_code;
        const isSuccess = inner.success ?? record.success;
        const isFailure = isSuccess === false || (exitCode !== undefined && exitCode !== 0);

        if (isFailure) {
          const errMsg = inner.error || inner.stderr || inner.stdout || record.error || JSON.stringify(inner);
          console.log(` FAIL (exit ${exitCode})`);
          console.log(`       ${String(errMsg).slice(0, 500)}`);
          failed++;
        } else {
          console.log(` ok`);
          if (inner.stdout && step.capability_id.startsWith("npm_")) {
            const lines = String(inner.stdout).trim().split("\n");
            const last5 = lines.slice(-5).join("\n       ");
            console.log(`       ${last5}`);
          }
          succeeded++;
        }
      } catch (err) {
        console.log(` ERROR: ${err.message}`);
        failed++;
      }
    }

    console.log(`\n[exec] Done: ${succeeded} ok, ${failed} failed / ${plan.steps.length} total`);

    // Show workspace
    try {
      const lsResult = await invokePort(soma, "filesystem", "readdir", {
        path: plan.workspace,
      });
      const entries = lsResult.result?.entries || [];
      if (entries.length > 0) {
        console.log(`\n[exec] Workspace:`);
        for (const e of entries) {
          console.log(`  ${e.is_dir ? "d" : "f"} ${e.name}`);
        }
      }
    } catch { /* not critical */ }
  } finally {
    soma.close();
  }
}

main().catch((err) => {
  console.error(`[exec] Fatal: ${err.message}`);
  process.exit(1);
});
