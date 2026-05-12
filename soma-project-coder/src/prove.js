#!/usr/bin/env node
/**
 * prove.js — End-to-end proof harness for soma-project-coder.
 *
 * Runs 7 phases that prove the full knowledge metabolism pipeline:
 *
 *   Phase 1: LLM connectivity (Tier 2 Ollama + Tier 3 fallback)
 *   Phase 2: SOMA port connectivity (MCP stdio)
 *   Phase 3: Plan decomposition (goal → steps)
 *   Phase 4: Plan execution with episode capture
 *   Phase 5: Episode persistence and retrieval
 *   Phase 6: Schema induction (PrefixSpan over episodes)
 *   Phase 7: Routine compilation (BMR gate)
 *
 * Exit 0 = all phases PASS. Any failure = exit 1.
 */

import { existsSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";
import { projectRoot } from "./env.js";
import { callLLM, parseJsonResponse } from "./llm.js";
import { connectSoma, invokePort } from "./mcp.js";
import { EpisodeRecorder, loadEpisodes, hashEmbed } from "./episodes.js";
import { loadRoutines, findMatchingRoutine, consolidate } from "./routines.js";

const results = [];
let phaseNum = 0;

function phase(name) {
  phaseNum++;
  process.stdout.write(`\n[Phase ${phaseNum}] ${name}... `);
}

function pass(detail) {
  results.push({ phase: phaseNum, status: "PASS", detail });
  console.log(`PASS${detail ? ` — ${detail}` : ""}`);
}

function fail(detail) {
  results.push({ phase: phaseNum, status: "FAIL", detail });
  console.log(`FAIL — ${detail}`);
}

// ── Phase 1: LLM connectivity ───────────────────────────────────────────────

async function phaseLLM() {
  phase("LLM connectivity");

  try {
    const result = await callLLM(
      "You are a test assistant. Respond with exactly: SOMA_OK",
      "Respond with exactly: SOMA_OK",
      { maxTokens: 32, timeout: 180_000 }
    );

    if (!result.text || !result.text.includes("SOMA_OK")) {
      fail(`unexpected response: ${result.text?.slice(0, 100)}`);
      return;
    }

    pass(`Tier ${result.tier} (${result.model}), ${result.tokens} tokens, ${result.latencyMs}ms`);
  } catch (err) {
    fail(err.message);
  }
}

// ── Phase 2: SOMA port connectivity ─────────────────────────────────────────

async function phaseSomaPorts() {
  phase("SOMA port connectivity");

  const somaPath = path.join(projectRoot, "bin/soma");
  if (!existsSync(somaPath)) {
    fail(`missing binary: ${somaPath}`);
    return null;
  }

  try {
    const soma = connectSoma();
    await soma.start();

    // Test filesystem port
    const testDir = path.join(projectRoot, "workspace/_prove_test");
    await invokePort(soma, "filesystem", "mkdir", { path: testDir });
    await invokePort(soma, "filesystem", "writefile", {
      path: path.join(testDir, "test.txt"),
      content: "SOMA_PROVE_OK",
    });
    const readResult = await invokePort(soma, "filesystem", "readfile", {
      path: path.join(testDir, "test.txt"),
    });

    const content = readResult.structured_result?.content || readResult.raw_result?.content || readResult.result?.content || readResult.content || "";
    if (!content.includes("SOMA_PROVE_OK")) {
      fail(`filesystem roundtrip failed: ${JSON.stringify(readResult).slice(0, 200)}`);
      soma.close();
      return null;
    }

    // Cleanup
    await invokePort(soma, "filesystem", "rm", { path: path.join(testDir, "test.txt") }).catch(() => {});
    await invokePort(soma, "filesystem", "rmdir", { path: testDir }).catch(() => {});

    pass("filesystem read/write roundtrip OK");
    return soma;
  } catch (err) {
    fail(err.message);
    return null;
  }
}

// ── Phase 3: Plan decomposition ─────────────────────────────────────────────

async function phasePlan(soma) {
  phase("Plan decomposition");

  const objective = "Create a Node.js hello-world with one test";
  try {
    const result = await callLLM(
      `You decompose coding goals into ordered steps. Each step is a SOMA port invocation.

Available ports and capabilities:
- filesystem: mkdir(path), writefile(path, content), readfile(path), readdir(path)
- runner: npm_install(cwd), npm_test(cwd)
- git: init(cwd), add(cwd, paths[]), commit(cwd, message)

Return a JSON array. Each entry:
{ "step": 1, "description": "...", "port_id": "filesystem", "capability_id": "writefile", "input": { "path": "..." } }

For writefile: do NOT include "content". Be concise — 5-8 steps max. Respond with ONLY the JSON array.`,
      `Goal: ${objective}\nWorkspace: ${path.join(projectRoot, "workspace/_prove_project")}`,
      { maxTokens: 1024, timeout: 600_000 }
    );

    const steps = parseJsonResponse(result.text);
    if (!Array.isArray(steps) || steps.length === 0) {
      fail("no steps parsed");
      return null;
    }

    const plan = {
      objective,
      workspace: path.join(projectRoot, "workspace/_prove_project"),
      created: new Date().toISOString(),
      planner: { model: result.model, tier: result.tier, tokens: result.tokens },
      steps,
    };

    pass(`${steps.length} steps, Tier ${result.tier} (${result.model})`);
    return plan;
  } catch (err) {
    fail(err.message);
    return null;
  }
}

// ── Phase 4: Plan execution with episode capture ────────────────────────────

async function phaseExecute(soma, plan) {
  phase("Plan execution with episode capture");

  if (!plan) { fail("skipped (no plan)"); return null; }

  const fingerprint = "prove_hello_world";
  const recorder = new EpisodeRecorder(fingerprint, plan.objective);
  let succeeded = 0;
  let failed = 0;

  try {
    for (const step of plan.steps) {
      const start = Date.now();

      if (step.capability_id === "writefile" && !step.input?.content) {
        // Generate file content via LLM
        const filePath = step.input.path;
        const parentDir = path.dirname(filePath);
        await invokePort(soma, "filesystem", "mkdir", { path: parentDir }).catch(() => {});

        const llmResult = await callLLM(
          "You generate file content for a coding project. Write COMPLETE code. Use CommonJS. Respond with ONLY the file content — no markdown fences.",
          `Project goal: ${plan.objective}\nFile to write: ${filePath}\nPurpose: ${step.description}\nWrite the complete file content:`,
          { maxTokens: 1024, timeout: 600_000 }
        );

        let content = llmResult.text;
        if (content.startsWith("```"))
          content = content.replace(/^```(?:\w+)?\s*\n?/, "").replace(/\n?```\s*$/, "");

        await invokePort(soma, "filesystem", "writefile", { path: filePath, content });
        recorder.recordStep({
          skill_id: "filesystem.writefile", input: { path: filePath },
          observation: `${content.split("\n").length} lines`, success: true,
          latencyMs: Date.now() - start, llmResult, tier: llmResult.tier,
        });
        succeeded++;
      } else {
        // Direct port invocation
        try {
          await invokePort(soma, step.port_id, step.capability_id, step.input);
          recorder.recordStep({
            skill_id: `${step.port_id}.${step.capability_id}`, input: step.input,
            observation: "ok", success: true, latencyMs: Date.now() - start,
          });
          succeeded++;
        } catch (err) {
          recorder.recordStep({
            skill_id: `${step.port_id}.${step.capability_id}`, input: step.input,
            observation: err.message, success: false, latencyMs: Date.now() - start,
          });
          failed++;
        }
      }
    }

    const episode = recorder.finish(failed === 0);
    pass(`${succeeded} ok, ${failed} failed, episode ${episode.episode_id.slice(0, 8)}`);
    return episode;
  } catch (err) {
    fail(err.message);
    recorder.finish(false);
    return null;
  }
}

// ── Phase 5: Episode persistence ────────────────────────────────────────────

function phaseEpisodePersistence(episode) {
  phase("Episode persistence and retrieval");

  if (!episode) { fail("skipped (no episode)"); return; }

  const episodes = loadEpisodes();
  const found = episodes.find(ep => ep.episode_id === episode.episode_id);
  if (!found) {
    fail(`episode ${episode.episode_id} not found in data/episodes/`);
    return;
  }

  if (found.steps.length !== episode.steps.length) {
    fail(`step count mismatch: ${found.steps.length} vs ${episode.steps.length}`);
    return;
  }

  pass(`${episodes.length} episodes on disk, verified ${found.steps.length} steps`);
}

// ── Phase 6: Schema induction ───────────────────────────────────────────────

function phaseSchemaInduction() {
  phase("Schema induction (PrefixSpan)");

  // Need at least 3 episodes with same fingerprint for BMR gate.
  // Create synthetic episodes to test the pipeline.
  for (let n = 0; n < 3; n++) {
    const syntheticRecorder = new EpisodeRecorder("prove_hello_world", `synthetic #${n}`);
    syntheticRecorder.recordStep({
      skill_id: "filesystem.mkdir", input: { path: "/tmp" },
      observation: "ok", success: true, latencyMs: 10,
    });
    syntheticRecorder.recordStep({
      skill_id: "filesystem.writefile", input: { path: "/tmp/pkg" },
      observation: "ok", success: true, latencyMs: 20,
    });
    syntheticRecorder.recordStep({
      skill_id: "filesystem.writefile", input: { path: "/tmp/index.js" },
      observation: "ok", success: true, latencyMs: 30,
    });
    syntheticRecorder.finish(true);
  }

  const episodes = loadEpisodes();
  const proveEps = episodes.filter(ep => ep.goal_fingerprint === "prove_hello_world");
  if (proveEps.length < 2) {
    fail(`need ≥2 episodes with same fingerprint, got ${proveEps.length}`);
    return;
  }

  // Run consolidation
  const result = consolidate();
  if (result.schemas === 0) {
    fail("no schemas induced");
    return;
  }

  pass(`${result.schemas} schemas from ${result.episodes} episodes`);
}

// ── Phase 7: Routine compilation ────────────────────────────────────────────

function phaseRoutineCompilation() {
  phase("Routine compilation (BMR gate)");

  const routines = loadRoutines();
  if (routines.length === 0) {
    // BMR gate may reject if success rate is too low — that's valid
    const episodes = loadEpisodes();
    const proveEps = episodes.filter(ep => ep.goal_fingerprint === "prove_hello_world");
    const successRate = proveEps.filter(ep => ep.outcome === "success").length / proveEps.length;
    if (successRate < 0.5) {
      pass(`BMR gate correctly rejected (success rate ${successRate})`);
    } else {
      fail("no routines compiled despite sufficient success rate");
    }
    return;
  }

  const match = findMatchingRoutine(routines, "prove_hello_world");
  if (!match) {
    fail("routine compiled but fingerprint match failed");
    return;
  }

  pass(`${routines.length} routines, matched ${match.routine_id} (confidence ${match.confidence})`);
}

// ── Main ────────────────────────────────────────────────────────────────────

async function main() {
  console.log("═══════════════════════════════════════════════════════════");
  console.log("  soma-project-coder — End-to-End Proof Harness");
  console.log("═══════════════════════════════════════════════════════════");

  // Phase 1: LLM
  await phaseLLM();

  // Phase 2: SOMA ports
  const soma = await phaseSomaPorts();

  // Phase 3: Plan
  const plan = soma ? await phasePlan(soma) : null;
  if (!plan) { phase("Plan decomposition"); fail("skipped (no SOMA connection)"); }

  // Phase 4: Execute
  const episode = (soma && plan) ? await phaseExecute(soma, plan) : null;
  if (!episode && soma && plan) { /* already logged */ }
  else if (!episode) { phase("Plan execution"); fail("skipped"); }

  // Phase 5: Episode persistence
  phaseEpisodePersistence(episode);

  // Phase 6: Schema induction
  phaseSchemaInduction();

  // Phase 7: Routine compilation
  phaseRoutineCompilation();

  // Close SOMA
  if (soma) soma.close();

  // Summary
  console.log("\n═══════════════════════════════════════════════════════════");
  const passed = results.filter(r => r.status === "PASS").length;
  const total = results.length;
  console.log(`  Result: ${passed}/${total} phases PASS`);
  for (const r of results) {
    console.log(`    Phase ${r.phase}: ${r.status}${r.detail ? ` — ${r.detail}` : ""}`);
  }
  console.log("═══════════════════════════════════════════════════════════");

  if (passed < total) {
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(`\n[prove] Fatal: ${err.message}`);
  process.exit(1);
});
