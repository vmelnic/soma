#!/usr/bin/env node
/**
 * plan.js — Haiku decomposes a coding goal into ordered steps.
 * Outputs plan.json that execute.js consumes.
 *
 * Usage:
 *   node plan.js "Build an Express API with user CRUD and SQLite"
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
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

function parseJson(text) {
  let cleaned = text.trim();
  if (cleaned.startsWith("```"))
    cleaned = cleaned.replace(/^```(?:json)?\s*/, "").replace(/\s*```$/, "");
  if (!cleaned.startsWith("[") && !cleaned.startsWith("{")) {
    const m = cleaned.match(/\[[\s\S]*\]/);
    if (m) cleaned = m[0];
  }
  return JSON.parse(cleaned);
}

// ---------------------------------------------------------------------------
// Plan prompt
// ---------------------------------------------------------------------------

const SYSTEM = `You decompose coding goals into ordered steps. Each step is a SOMA port invocation.

Available ports and capabilities:
- filesystem: mkdir(path), writefile(path, content), readfile(path), readdir(path), stat(path), rm(path), rmdir(path)
- git: init(cwd), add(cwd, paths[]), commit(cwd, message), status(cwd), diff(cwd), log(cwd)
- runner: npm_install(cwd), npm_test(cwd), npm_run(cwd, script), node_run(cwd, file)
- search: text_search(cwd, pattern), file_search(cwd, glob), symbol_search(cwd, pattern)
- patch: apply_patch(cwd, patch), check_patch(cwd, patch), create_patch(cwd, file, content)

Return a JSON array. Each entry:
{
  "step": 1,
  "description": "what this step does",
  "port_id": "filesystem",
  "capability_id": "writefile",
  "input": { "path": "/workspace/file.js" }
}

For writefile steps: do NOT include "content" in input — it will be generated during execution.
For all other steps: include ALL required input fields with actual values.

Order for new projects:
1. mkdir (project dir, subdirs)
2. writefile package.json
3. npm_install
4. writefile (source files one by one)
5. writefile (test files)
6. npm_test
7. git init, git add, git commit

Be thorough. List EVERY file the project needs. One writefile step per file.
Respond with ONLY the JSON array.`;

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  const objective = process.argv[2];
  if (!objective) {
    console.error('Usage: node plan.js "Build an Express API with user CRUD and SQLite"');
    process.exit(1);
  }

  const env = await loadEnv();
  const workspace = path.resolve(
    env.SOMA_CODE_WORKSPACE || path.join(projectRoot, "workspace")
  );
  mkdirSync(workspace, { recursive: true });

  console.log(`[plan] Goal: ${objective}`);
  console.log(`[plan] Workspace: ${workspace}`);
  console.log(`[plan] Model: ${env.ANTHROPIC_MODEL || "claude-haiku-4-5-20251001"}`);

  const response = await callLLM(SYSTEM, `Goal: ${objective}\nWorkspace: ${workspace}`);
  const steps = parseJson(response);

  console.log(`[plan] ${steps.length} steps:\n`);
  for (const s of steps) {
    console.log(`  ${s.step}. [${s.port_id}.${s.capability_id}] ${s.description}`);
  }

  const plan = {
    objective,
    workspace,
    created: new Date().toISOString(),
    steps,
  };

  const planPath = path.join(projectRoot, "plan.json");
  writeFileSync(planPath, JSON.stringify(plan, null, 2));
  console.log(`\n[plan] Written to ${planPath}`);
}

main().catch((err) => {
  console.error(`[plan] Fatal: ${err.message}`);
  process.exit(1);
});
