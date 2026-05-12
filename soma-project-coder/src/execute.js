#!/usr/bin/env node
/**
 * execute.js — Execute a plan.json through SOMA ports.
 *
 * For writefile steps: asks the LLM for file content with cross-file context.
 * For other steps: invokes the port directly.
 * Records every step as an episode for later schema induction.
 *
 * Usage:
 *   node src/execute.js [plan.json]
 */

import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { loadEnv, projectRoot } from "./env.js";
import { callLLM, parseJsonResponse } from "./llm.js";
import { connectSoma, invokePort } from "./mcp.js";
import { EpisodeRecorder } from "./episodes.js";
import { loadRoutines, findMatchingRoutine } from "./routines.js";

// ── Content generation ───────────────────────────────────────────────────────

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
    const snippets = writtenFiles
      .filter(f => !f.path.endsWith(".md") && !f.path.endsWith(".gitignore"))
      .map(f => `--- ${path.basename(f.path)} ---\n${f.content}`);
    context = `\nPreviously written files (use these EXACT exports/APIs):\n${snippets.join("\n\n")}`;
  }

  const layout = writtenFiles.length
    ? `\nProject file layout:\n${writtenFiles.map(f => `  ${f.path}`).join("\n")}\n  ${filePath} (this file)`
    : "";

  const prompt = `Project goal: ${objective}\nFile to write: ${filePath}\nPurpose: ${description}${layout}${context}\n\nWrite the complete file content:`;

  return callLLM(WRITE_SYSTEM, prompt, { maxTokens: 8192 });
}

// ── Fix generation ───────────────────────────────────────────────────────────

const FIX_SYSTEM = `You fix code based on test/compiler errors. Given the error and the file that caused it, provide the COMPLETE corrected file content. No markdown fences, no explanation.`;

async function generateFix(objective, filePath, fileContent, error, writtenFiles) {
  const context = writtenFiles
    .filter(f => f.path !== filePath && !f.path.endsWith(".md"))
    .map(f => `--- ${path.basename(f.path)} ---\n${f.content}`)
    .join("\n\n");

  const prompt = `Project goal: ${objective}
File with error: ${filePath}
Current content:
${fileContent}

Error:
${error}

Other project files:
${context}

Write the complete corrected file content:`;

  return callLLM(FIX_SYSTEM, prompt, { maxTokens: 8192 });
}

// ── Dependency reconciliation ────────────────────────────────────────────────

const BUILTINS = new Set([
  "assert", "buffer", "child_process", "cluster", "crypto", "dgram", "dns",
  "events", "fs", "http", "http2", "https", "net", "os", "path", "perf_hooks",
  "process", "querystring", "readline", "stream", "string_decoder", "timers",
  "tls", "tty", "url", "util", "v8", "vm", "worker_threads", "zlib",
]);

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
      if (!declared.has(mod) && !BUILTINS.has(mod) && !mod.startsWith("node:"))
        missing.add(mod);
    }
  }

  if (missing.size === 0) return;
  const names = [...missing].join(" ");
  console.log(`  [deps] installing: ${names}`);
  try {
    await invokePort(soma, "runner", "exec", {
      cwd: path.dirname(pkgFile.path),
      command: `npm install --save ${names}`,
      timeout_ms: 60000,
    });
  } catch (err) {
    console.log(`  [deps] install failed: ${err.message}`);
  }
}

// ── Fingerprint a plan for routine matching ──────────────────────────────────

function planFingerprint(plan) {
  // Fingerprint by skill bag (unique capabilities), not exact sequence.
  // Plans with different step counts but same capability set match.
  const skillBag = [...new Set(
    plan.steps.map(s => `${s.port_id}.${s.capability_id}`)
  )].sort().join(",");
  let h = 2166136261 >>> 0;
  for (let i = 0; i < skillBag.length; i++) {
    h ^= skillBag.charCodeAt(i);
    h = Math.imul(h, 16777619) >>> 0;
  }
  return `coder_${h.toString(16)}`;
}

// ── Main execution loop ──────────────────────────────────────────────────────

async function main() {
  const planFile = process.argv[2] || path.join(projectRoot, "plan.json");
  if (!existsSync(planFile)) {
    console.error(`Plan not found: ${planFile}\nRun: node src/plan.js "your goal"`);
    process.exit(1);
  }

  const plan = JSON.parse(readFileSync(planFile, "utf8"));
  const fingerprint = planFingerprint(plan);

  console.log(`[exec] Goal: ${plan.objective}`);
  console.log(`[exec] Workspace: ${plan.workspace}`);
  console.log(`[exec] Fingerprint: ${fingerprint}`);
  console.log(`[exec] ${plan.steps.length} steps to execute\n`);

  // Check for matching routine (Tier 1)
  const routines = loadRoutines();
  const routine = findMatchingRoutine(routines, fingerprint);
  if (routine) {
    console.log(`[exec] ROUTINE MATCH: ${routine.routine_id} (confidence: ${routine.confidence})`);
    console.log(`[exec] Using Tier 1 (compiled routine) — skipping plan deliberation\n`);
  }

  const soma = connectSoma();
  await soma.start();
  console.log("[exec] SOMA connected.\n");

  const recorder = new EpisodeRecorder(fingerprint, plan.objective);
  let succeeded = 0;
  let failed = 0;
  const writtenFiles = [];
  let depsReconciled = false;

  try {
    for (let i = 0; i < plan.steps.length; i++) {
      const step = plan.steps[i];
      const label = `[${i + 1}/${plan.steps.length}]`;
      const start = Date.now();

      // Reconcile deps before test
      if (!depsReconciled && step.capability_id === "npm_test" && writtenFiles.length > 0) {
        depsReconciled = true;
        await reconcileDeps(soma, plan.workspace, writtenFiles);
      }

      // ── writefile: generate content via LLM ──
      if (step.capability_id === "writefile" && !step.input?.content) {
        const filePath = step.input.path;
        process.stdout.write(`${label} generating ${path.basename(filePath)}...`);

        try {
          const parentDir = path.dirname(filePath);
          await invokePort(soma, "filesystem", "mkdir", { path: parentDir }).catch(() => {});

          const llmResult = await generateFileContent(
            plan.objective, filePath, step.description, writtenFiles
          );
          let content = llmResult.text;
          if (content.startsWith("```"))
            content = content.replace(/^```(?:\w+)?\s*\n?/, "").replace(/\n?```\s*$/, "");

          const result = await invokePort(soma, "filesystem", "writefile", {
            path: filePath, content,
          });
          const record = result.result || result;

          if (record.success === false) {
            console.log(` FAIL`);
            recorder.recordStep({
              skill_id: `filesystem.writefile`, input: { path: filePath },
              observation: record.error, success: false,
              latencyMs: Date.now() - start, llmResult, tier: llmResult.tier,
            });
            failed++;
          } else {
            console.log(` ok (${content.split("\n").length} lines, Tier ${llmResult.tier})`);
            writtenFiles.push({ path: filePath, content });
            recorder.recordStep({
              skill_id: `filesystem.writefile`, input: { path: filePath },
              observation: `${content.split("\n").length} lines`, success: true,
              latencyMs: Date.now() - start, llmResult, tier: llmResult.tier,
            });
            succeeded++;
          }
        } catch (err) {
          console.log(` ERROR: ${err.message}`);
          recorder.recordStep({
            skill_id: `filesystem.writefile`, input: { path: step.input.path },
            observation: err.message, success: false, latencyMs: Date.now() - start,
          });
          failed++;
        }
        continue;
      }

      // ── All other steps: invoke port directly ──
      process.stdout.write(`${label} ${step.description}...`);

      try {
        const result = await invokePort(soma, step.port_id, step.capability_id, step.input);
        const record = result.result || result;
        const inner = record.structured_result || record.raw_result || record;
        const exitCode = inner.exit_code ?? record.exit_code;
        const isSuccess = inner.success ?? record.success;
        const isFailure = isSuccess === false || (exitCode !== undefined && exitCode !== 0);

        if (isFailure) {
          const errMsg = inner.error || inner.stderr || inner.stdout || record.error || JSON.stringify(inner);
          console.log(` FAIL (exit ${exitCode})`);

          // Error recovery: try to fix failing tests
          if (step.capability_id === "npm_test" && writtenFiles.length > 0) {
            console.log(`  [fix] attempting error recovery...`);
            const fixAttempt = await attemptFix(
              soma, plan, writtenFiles, String(errMsg).slice(0, 2000), recorder
            );
            if (fixAttempt) {
              console.log(`  [fix] retrying tests...`);
              const retryResult = await invokePort(soma, "runner", "npm_test", step.input);
              const retryInner = retryResult.result?.structured_result || retryResult.result || retryResult;
              if ((retryInner.exit_code ?? 0) === 0) {
                console.log(`  [fix] tests pass after fix`);
                recorder.recordStep({
                  skill_id: `runner.npm_test`, input: step.input,
                  observation: "pass after fix", success: true,
                  latencyMs: Date.now() - start,
                });
                succeeded++;
                continue;
              }
            }
          }

          recorder.recordStep({
            skill_id: `${step.port_id}.${step.capability_id}`, input: step.input,
            observation: String(errMsg).slice(0, 500), success: false,
            latencyMs: Date.now() - start,
          });
          failed++;
        } else {
          console.log(` ok`);
          if (inner.stdout && step.capability_id.startsWith("npm_")) {
            const lines = String(inner.stdout).trim().split("\n");
            console.log(`       ${lines.slice(-3).join("\n       ")}`);
          }
          recorder.recordStep({
            skill_id: `${step.port_id}.${step.capability_id}`, input: step.input,
            observation: "ok", success: true, latencyMs: Date.now() - start,
          });
          succeeded++;
        }
      } catch (err) {
        console.log(` ERROR: ${err.message}`);
        recorder.recordStep({
          skill_id: `${step.port_id}.${step.capability_id}`, input: step.input,
          observation: err.message, success: false, latencyMs: Date.now() - start,
        });
        failed++;
      }
    }

    // Finalize episode
    const episode = recorder.finish(succeeded / (succeeded + failed) >= 0.8);
    console.log(`\n[exec] Done: ${succeeded} ok, ${failed} failed / ${plan.steps.length} total`);
    console.log(`[exec] Episode: ${episode.episode_id}`);
    console.log(`[exec] Tiers: T1=${episode.tier_usage.tier1} T2=${episode.tier_usage.tier2} T3=${episode.tier_usage.tier3}`);
    console.log(`[exec] Tokens: ${episode.total_tokens}, Latency: ${episode.total_latency_ms}ms`);

  } finally {
    soma.close();
  }
}

// ── Error recovery: identify the broken file and regenerate ──────────────────

async function attemptFix(soma, plan, writtenFiles, error, recorder) {
  // Find which file the error points to
  const filePattern = /(?:at |in |from )(?:Object\.<anonymous> \()?([^\s:)]+\.js)/;
  const match = error.match(filePattern);
  if (!match) return false;

  const errorFile = writtenFiles.find(f => f.path.endsWith(match[1]) || f.path.includes(match[1]));
  if (!errorFile) return false;

  console.log(`  [fix] regenerating ${path.basename(errorFile.path)}...`);
  const start = Date.now();

  try {
    const llmResult = await generateFix(
      plan.objective, errorFile.path, errorFile.content, error, writtenFiles
    );
    let content = llmResult.text;
    if (content.startsWith("```"))
      content = content.replace(/^```(?:\w+)?\s*\n?/, "").replace(/\n?```\s*$/, "");

    await invokePort(soma, "filesystem", "writefile", { path: errorFile.path, content });
    errorFile.content = content;

    recorder.recordStep({
      skill_id: "coder.fix_error",
      input: { path: errorFile.path, error: error.slice(0, 200) },
      observation: `regenerated ${content.split("\n").length} lines`,
      success: true,
      latencyMs: Date.now() - start,
      llmResult,
      tier: llmResult.tier,
    });

    console.log(`  [fix] ${path.basename(errorFile.path)} regenerated (Tier ${llmResult.tier})`);
    return true;
  } catch (err) {
    console.log(`  [fix] failed: ${err.message}`);
    return false;
  }
}

main().catch((err) => {
  console.error(`[exec] Fatal: ${err.message}`);
  process.exit(1);
});
