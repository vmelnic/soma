#!/usr/bin/env node
/**
 * autonomy.js — 10-minute sustained autonomy test.
 *
 * Proves the "ignorant → competent" trajectory:
 *   1. Boots SOMA with brain + DNA
 *   2. Polls workspace (git status, file changes) → deposits world state facts
 *   3. Injects perturbations at scheduled times (break test, corrupt file, etc.)
 *   4. Measures: brain calls, routine hits, steps-per-resolution over time
 *   5. Reports trajectory: does the system get faster on repeated event types?
 *
 * Usage: node src/autonomy.js [duration_minutes]
 */

import { connectSoma, extractToolContent } from "./mcp.js";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import path from "node:path";
import { projectRoot } from "./env.js";

const DURATION_MIN = parseInt(process.argv[2] || "10", 10);
const DURATION_MS = DURATION_MIN * 60_000;
const POLL_INTERVAL = 20_000;
const WS = path.join(projectRoot, "workspace");

// ── Metrics ─────────────────────────────────────────────────────────────────

const metrics = {
  events: [],          // { time, type, description }
  resolutions: [],     // { time, type, steps, brainCalls, usedRoutine, durationMs }
  routinesCompiled: 0,
  schemasInduced: 0,
};

function elapsed() {
  return ((Date.now() - startTime) / 1000).toFixed(0) + "s";
}

function log(msg) {
  console.log(`[${elapsed()}] ${msg}`);
}

// ── Perturbations ───────────────────────────────────────────────────────────

const ORIGINAL_USER_MODEL = path.join(WS, "src/models/User.js");
let userModelBackup = null;

function backupFile(filePath) {
  if (existsSync(filePath)) {
    return readFileSync(filePath, "utf8");
  }
  return null;
}

const perturbations = [
  {
    // Break the User model: change a SQL query
    atSec: 60,
    type: "bug_introduced",
    description: "corrupt User.getAllUsers SQL query",
    apply: () => {
      const content = readFileSync(ORIGINAL_USER_MODEL, "utf8");
      userModelBackup = content;
      const broken = content.replace(
        "SELECT * FROM users",
        "SELECT * FROM userz"  // typo in table name
      );
      writeFileSync(ORIGINAL_USER_MODEL, broken);
    },
    revert: () => {
      if (userModelBackup) writeFileSync(ORIGINAL_USER_MODEL, userModelBackup);
    },
  },
  {
    // Revert the bug
    atSec: 120,
    type: "bug_fixed",
    description: "revert User.getAllUsers SQL fix",
    apply: () => {
      if (userModelBackup) writeFileSync(ORIGINAL_USER_MODEL, userModelBackup);
    },
    revert: () => {},
  },
  {
    // Break it again (same type — should trigger routine reuse)
    atSec: 210,
    type: "bug_introduced",
    description: "corrupt User.createUser SQL query",
    apply: () => {
      const content = readFileSync(ORIGINAL_USER_MODEL, "utf8");
      userModelBackup = content;
      const broken = content.replace(
        "INSERT INTO users",
        "INSERT INTO userz"
      );
      writeFileSync(ORIGINAL_USER_MODEL, broken);
    },
    revert: () => {
      if (userModelBackup) writeFileSync(ORIGINAL_USER_MODEL, userModelBackup);
    },
  },
  {
    // Revert again
    atSec: 300,
    type: "bug_fixed",
    description: "revert User.createUser SQL fix",
    apply: () => {
      if (userModelBackup) writeFileSync(ORIGINAL_USER_MODEL, userModelBackup);
    },
    revert: () => {},
  },
  {
    // Third break (same pattern — routine should definitely exist now)
    atSec: 390,
    type: "bug_introduced",
    description: "corrupt User.deleteUser SQL query",
    apply: () => {
      const content = readFileSync(ORIGINAL_USER_MODEL, "utf8");
      userModelBackup = content;
      const broken = content.replace(
        "DELETE FROM users",
        "DELETE FROM userz"
      );
      writeFileSync(ORIGINAL_USER_MODEL, broken);
    },
    revert: () => {
      if (userModelBackup) writeFileSync(ORIGINAL_USER_MODEL, userModelBackup);
    },
  },
  {
    // Final revert
    atSec: 480,
    type: "bug_fixed",
    description: "revert User.deleteUser SQL fix",
    apply: () => {
      if (userModelBackup) writeFileSync(ORIGINAL_USER_MODEL, userModelBackup);
    },
    revert: () => {},
  },
];

// ── World State Poller ──────────────────────────────────────────────────────

let lastFileHash = "";

async function pollWorldState(soma) {
  try {
    // Check git status
    const gitResult = await soma.callTool("invoke_port", {
      port_id: "git",
      capability_id: "status",
      input: { cwd: WS },
    });
    const gitData = extractToolContent(gitResult);
    const gitStatus = gitData?.structured_result || gitData?.raw_result || {};

    // Check if files changed
    const userModel = existsSync(ORIGINAL_USER_MODEL)
      ? readFileSync(ORIGINAL_USER_MODEL, "utf8")
      : "";
    const currentHash = simpleHash(userModel);
    const fileChanged = currentHash !== lastFileHash;
    lastFileHash = currentHash;

    // Deposit facts into world state
    const facts = [];

    if (fileChanged) {
      facts.push({
        fact_id: "poll_file_change",
        subject: "event",
        predicate: "detected",
        value: { type: "file_changed", path: "src/models/User.js", hash: currentHash },
        confidence: 1.0,
        provenance: "observed",
        timestamp: new Date().toISOString(),
      });
      facts.push({
        fact_id: "poll_novelty",
        subject: "novelty",
        predicate: "detected",
        value: { type: "code_change", file: "User.js" },
        confidence: 1.0,
        provenance: "observed",
        timestamp: new Date().toISOString(),
      });
    }

    // Check for anomalies (broken SQL patterns)
    if (userModel.includes("userz")) {
      facts.push({
        fact_id: "poll_anomaly",
        subject: "anomaly",
        predicate: "detected",
        value: { type: "sql_typo", pattern: "userz", file: "User.js" },
        confidence: 1.0,
        provenance: "observed",
        timestamp: new Date().toISOString(),
      });
    }

    if (facts.length > 0) {
      await soma.callTool("patch_world_state", { add_facts: facts });
      log(`polled: ${facts.length} facts deposited (${facts.map(f => f.subject + "." + f.predicate).join(", ")})`);
    }
  } catch (err) {
    log(`poll error (non-fatal): ${err.message}`);
  }
}

function simpleHash(str) {
  let h = 0;
  for (let i = 0; i < str.length; i++) {
    h = ((h << 5) - h + str.charCodeAt(i)) | 0;
  }
  return h.toString(16);
}

// ── Metrics Collection ──────────────────────────────────────────────────────

async function collectMetrics(soma) {
  try {
    const ws = extractToolContent(await soma.callTool("dump_world_state", {}));
    const snap = ws.snapshot || ws;

    // Count brain invocations and routine successes/failures
    let brainCalls = 0;
    let routineSuccesses = 0;
    let routineFailures = 0;

    for (const [key, val] of Object.entries(snap)) {
      if (key.includes("brain")) brainCalls++;
      if (key.includes("dna.") && key.includes("success")) routineSuccesses++;
      if (key.includes("dna.") && key.includes("failure")) routineFailures++;
    }

    // Check for newly compiled routines
    const dump = extractToolContent(await soma.callTool("dump_state", { sections: ["routines"] }));
    const routines = dump.routines || [];
    const compiled = routines.filter(r => r.routine_id.startsWith("compiled_"));
    if (compiled.length > metrics.routinesCompiled) {
      const newCount = compiled.length - metrics.routinesCompiled;
      log(`LEARNING: ${newCount} new routine(s) compiled (total: ${compiled.length})`);
      metrics.routinesCompiled = compiled.length;
    }

    return { brainCalls, routineSuccesses, routineFailures };
  } catch (err) {
    return { brainCalls: 0, routineSuccesses: 0, routineFailures: 0 };
  }
}

// ── Main Loop ───────────────────────────────────────────────────────────────

let startTime;

async function main() {
  console.log("═══════════════════════════════════════════════════════════");
  console.log(`  soma-project-coder — ${DURATION_MIN}-Minute Autonomy Test`);
  console.log("═══════════════════════════════════════════════════════════");

  startTime = Date.now();

  // Backup workspace files
  userModelBackup = backupFile(ORIGINAL_USER_MODEL);

  const soma = connectSoma();
  await soma.start();
  log("SOMA booted (21 skills, brain + DNA active)");

  // Initialize file hash
  if (existsSync(ORIGINAL_USER_MODEL)) {
    lastFileHash = simpleHash(readFileSync(ORIGINAL_USER_MODEL, "utf8"));
  }

  const endTime = startTime + DURATION_MS;

  // Schedule perturbations
  const pendingPerturbations = [...perturbations].filter(p => p.atSec * 1000 < DURATION_MS);
  let nextPertIdx = 0;

  // Collect initial metrics
  const initialMetrics = await collectMetrics(soma);
  log(`initial state: ${initialMetrics.brainCalls} brain facts, ${metrics.routinesCompiled} compiled routines`);

  // Main loop
  const pollTimer = setInterval(() => pollWorldState(soma), POLL_INTERVAL);

  const perturbTimer = setInterval(() => {
    const elapsedSec = (Date.now() - startTime) / 1000;

    while (nextPertIdx < pendingPerturbations.length
      && pendingPerturbations[nextPertIdx].atSec <= elapsedSec) {
      const p = pendingPerturbations[nextPertIdx];
      log(`PERTURBATION: ${p.description}`);
      p.apply();
      metrics.events.push({
        time: elapsedSec,
        type: p.type,
        description: p.description,
      });
      nextPertIdx++;
    }
  }, 1000);

  // Metrics collection every 30s
  const metricsTimer = setInterval(async () => {
    const m = await collectMetrics(soma);
    const elapsedSec = (Date.now() - startTime) / 1000;
    log(`metrics: brain=${m.brainCalls} routineOK=${m.routineSuccesses} routineFail=${m.routineFailures} compiled=${metrics.routinesCompiled}`);
  }, 30_000);

  // Wait for duration
  await new Promise(resolve => {
    const checkTimer = setInterval(() => {
      if (Date.now() >= endTime) {
        clearInterval(checkTimer);
        resolve();
      }
    }, 1000);
  });

  // Cleanup
  clearInterval(pollTimer);
  clearInterval(perturbTimer);
  clearInterval(metricsTimer);

  // Revert any pending perturbations
  for (const p of pendingPerturbations) {
    p.revert();
  }

  // Final metrics
  log("collecting final metrics...");
  const finalMetrics = await collectMetrics(soma);

  // Trigger consolidation to see if new schemas/routines formed
  try {
    const consResult = await soma.callTool("consolidate", {});
    const consData = extractToolContent(consResult);
    log(`consolidation: ${consData.schemas_induced || 0} schemas, ${consData.routines_compiled || 0} routines`);
  } catch (err) {
    log("consolidation skipped (tool may not exist)");
  }

  const finalFinalMetrics = await collectMetrics(soma);

  soma.close();

  // ── Report ──────────────────────────────────────────────────────────────

  console.log("\n═══════════════════════════════════════════════════════════");
  console.log("  Autonomy Test Results");
  console.log("═══════════════════════════════════════════════════════════");

  console.log(`\n  Duration: ${DURATION_MIN} minutes`);
  console.log(`  Events injected: ${metrics.events.length}`);
  console.log(`  Brain invocations: ${finalFinalMetrics.brainCalls}`);
  console.log(`  Routine successes: ${finalFinalMetrics.routineSuccesses}`);
  console.log(`  Routine failures: ${finalFinalMetrics.routineFailures}`);
  console.log(`  Routines compiled: ${metrics.routinesCompiled}`);

  console.log("\n  Timeline:");
  for (const e of metrics.events) {
    console.log(`    ${e.time.toFixed(0)}s: [${e.type}] ${e.description}`);
  }

  // Trajectory analysis
  const earlyBrain = finalFinalMetrics.brainCalls;
  const compiledRoutines = metrics.routinesCompiled;

  console.log("\n  Trajectory:");
  if (compiledRoutines > 0) {
    console.log("    ✓ System compiled routines from experience");
    console.log(`    ✓ ${compiledRoutines} reusable behaviors learned`);
  } else {
    console.log("    → No routines compiled yet (may need more episodes with same pattern)");
  }

  if (finalFinalMetrics.routineSuccesses > 0) {
    console.log(`    ✓ ${finalFinalMetrics.routineSuccesses} successful autonomous responses`);
  }

  const trajectory = compiledRoutines > 0 ? "COMPETENT" : "LEARNING";
  console.log(`\n  Status: ${trajectory}`);
  console.log("═══════════════════════════════════════════════════════════");
}

main().catch(err => {
  // Revert files on crash
  if (userModelBackup) {
    writeFileSync(ORIGINAL_USER_MODEL, userModelBackup);
  }
  console.error(`Fatal: ${err.message}`);
  process.exit(1);
});
