#!/usr/bin/env node
/**
 * plan.js — Decomposes a coding goal into ordered SOMA port steps.
 * Outputs plan.json for execute.js to consume.
 *
 * Usage:
 *   node src/plan.js "Build an Express API with user CRUD and SQLite"
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { loadEnv, projectRoot } from "./env.js";
import { callLLM, parseJsonResponse } from "./llm.js";

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

async function main() {
  const objective = process.argv[2];
  if (!objective) {
    console.error('Usage: node src/plan.js "Build an Express API with user CRUD and SQLite"');
    process.exit(1);
  }

  const env = await loadEnv();
  const workspace = path.resolve(
    env.SOMA_CODER_WORKSPACE || path.join(projectRoot, "workspace")
  );
  mkdirSync(workspace, { recursive: true });

  console.log(`[plan] Goal: ${objective}`);
  console.log(`[plan] Workspace: ${workspace}`);

  const result = await callLLM(SYSTEM, `Goal: ${objective}\nWorkspace: ${workspace}`, {
    maxTokens: 2048,
  });

  console.log(`[plan] Model: ${result.model} (Tier ${result.tier}, ${result.latencyMs}ms)`);

  const steps = parseJsonResponse(result.text);
  console.log(`[plan] ${steps.length} steps:\n`);
  for (const s of steps) {
    console.log(`  ${s.step}. [${s.port_id}.${s.capability_id}] ${s.description}`);
  }

  const plan = {
    objective,
    workspace,
    created: new Date().toISOString(),
    planner: { model: result.model, tier: result.tier, tokens: result.tokens },
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
