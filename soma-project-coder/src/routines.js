/**
 * Routine pipeline — schema induction + routine compilation.
 *
 * Episodes (raw traces) → PrefixSpan (frequent subsequences) →
 * Schemas (canonical skill sequences) → BMR gate → Compiled Routines.
 *
 * Compiled routines execute at Tier 1 (no LLM) by replaying the
 * canonical step sequence with variable bindings from the goal.
 */

import { existsSync, mkdirSync, writeFileSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { projectRoot } from "./env.js";
import { loadEpisodes, groupByFingerprint, hashEmbed } from "./episodes.js";

const ROUTINES_DIR = path.join(projectRoot, "data/routines");
const SCHEMAS_DIR = path.join(projectRoot, "data/schemas");

// ── PrefixSpan — mine frequent skill subsequences ───────────────────────────

function prefixSpan(sequences, minSupport = 2) {
  const patterns = [];

  function project(prefix, projected) {
    const freqItems = new Map();
    for (const { seq, start } of projected) {
      const seen = new Set();
      for (let i = start; i < seq.length; i++) {
        if (!seen.has(seq[i])) {
          seen.add(seq[i]);
          freqItems.set(seq[i], (freqItems.get(seq[i]) || 0) + 1);
        }
      }
    }

    for (const [item, support] of freqItems) {
      if (support < minSupport) continue;
      const newPrefix = [...prefix, item];
      patterns.push({ pattern: newPrefix, support });

      const newProjected = [];
      for (const { seq, start } of projected) {
        for (let i = start; i < seq.length; i++) {
          if (seq[i] === item) {
            newProjected.push({ seq, start: i + 1 });
            break;
          }
        }
      }
      if (newProjected.length >= minSupport) {
        project(newPrefix, newProjected);
      }
    }
  }

  const initial = sequences.map(seq => ({ seq, start: 0 }));
  project([], initial);

  // Return longest patterns first (most informative)
  patterns.sort((a, b) => b.pattern.length - a.pattern.length || b.support - a.support);
  return patterns;
}

// ── Schema induction ────────────────────────────────────────────────────────

function induceSchemas(episodes) {
  const groups = groupByFingerprint(episodes);
  const schemas = [];

  for (const [fingerprint, group] of Object.entries(groups)) {
    if (group.length < 2) continue;

    const sequences = group.map(ep =>
      ep.steps.map(s => s.skill_id)
    );

    const patterns = prefixSpan(sequences, Math.max(2, Math.floor(group.length * 0.5)));
    if (patterns.length === 0) continue;

    // Pick the longest pattern with highest support as canonical
    const canonical = patterns[0];

    const successRate = group.filter(ep => ep.outcome === "success").length / group.length;
    const avgLatency = group.reduce((s, ep) => s + ep.total_latency_ms, 0) / group.length;
    const avgTokens = group.reduce((s, ep) => s + ep.total_tokens, 0) / group.length;

    schemas.push({
      schema_id: `schema_${fingerprint}`,
      fingerprint,
      canonical_sequence: canonical.pattern,
      support: canonical.support,
      episode_count: group.length,
      success_rate: Math.round(successRate * 1000) / 1000,
      avg_latency_ms: Math.round(avgLatency),
      avg_tokens: Math.round(avgTokens),
      patterns: patterns.slice(0, 10),
      embedding: hashEmbed(fingerprint),
      induced_at: new Date().toISOString(),
    });
  }

  return schemas;
}

// ── BMR gate — Bayesian Model Reduction ─────────────────────────────────────
//
// A routine is worth compiling if the accuracy gain justifies its complexity.
// accuracy_loss + 0.1 * complexity > 0.5 → reject (too lossy or too complex)

function bmrGate(schema, episodes) {
  const group = episodes.filter(ep => ep.goal_fingerprint === schema.fingerprint);
  if (group.length < 3) return { pass: false, reason: "insufficient episodes" };

  // Accuracy: success rate of episodes matching this schema
  const accuracyLoss = 1 - schema.success_rate;

  // Complexity: normalized sequence length (longer = more complex)
  const maxSteps = Math.max(...group.map(ep => ep.steps.length));
  const complexity = schema.canonical_sequence.length / Math.max(maxSteps, 1);

  const score = accuracyLoss + 0.1 * complexity;
  const pass = score <= 0.5;

  return {
    pass,
    score: Math.round(score * 1000) / 1000,
    accuracy_loss: Math.round(accuracyLoss * 1000) / 1000,
    complexity: Math.round(complexity * 1000) / 1000,
    reason: pass ? "accepted" : `score ${score.toFixed(3)} > 0.5 threshold`,
  };
}

// ── Compile a schema into a routine ─────────────────────────────────────────

function compileRoutine(schema, episodes) {
  const gate = bmrGate(schema, episodes);
  if (!gate.pass) return null;

  const group = episodes.filter(ep => ep.goal_fingerprint === schema.fingerprint);
  const successEps = group.filter(ep => ep.outcome === "success");

  // Extract variable bindings from successful episodes
  const bindings = {};
  for (const ep of successEps) {
    for (const step of ep.steps) {
      if (step.input_summary) {
        const key = step.skill_id;
        if (!bindings[key]) bindings[key] = [];
        bindings[key].push(step.input_summary);
      }
    }
  }

  // Build compiled steps from canonical sequence
  const steps = schema.canonical_sequence.map((skillId, i) => {
    const [portId, capabilityId] = skillId.split(".");
    return {
      step_index: i,
      skill_id: skillId,
      port_id: portId,
      capability_id: capabilityId,
      binding_template: bindings[skillId]?.[0] || null,
    };
  });

  return {
    routine_id: `routine_${schema.fingerprint}`,
    schema_id: schema.schema_id,
    fingerprint: schema.fingerprint,
    confidence: schema.success_rate,
    steps,
    bmr_gate: gate,
    episode_count: schema.episode_count,
    embedding: schema.embedding,
    compiled_at: new Date().toISOString(),
  };
}

// ── Load / save ─────────────────────────────────────────────────────────────

export function loadRoutines() {
  if (!existsSync(ROUTINES_DIR)) return [];
  return readdirSync(ROUTINES_DIR)
    .filter(f => f.endsWith(".json"))
    .map(f => JSON.parse(readFileSync(path.join(ROUTINES_DIR, f), "utf8")));
}

function loadSchemas() {
  if (!existsSync(SCHEMAS_DIR)) return [];
  return readdirSync(SCHEMAS_DIR)
    .filter(f => f.endsWith(".json"))
    .map(f => JSON.parse(readFileSync(path.join(SCHEMAS_DIR, f), "utf8")));
}

function saveSchema(schema) {
  mkdirSync(SCHEMAS_DIR, { recursive: true });
  writeFileSync(
    path.join(SCHEMAS_DIR, `${schema.schema_id}.json`),
    JSON.stringify(schema, null, 2)
  );
}

function saveRoutine(routine) {
  mkdirSync(ROUTINES_DIR, { recursive: true });
  writeFileSync(
    path.join(ROUTINES_DIR, `${routine.routine_id}.json`),
    JSON.stringify(routine, null, 2)
  );
}

// ── Fingerprint matching ────────────────────────────────────────────────────

export function findMatchingRoutine(routines, fingerprint) {
  // Exact match first
  const exact = routines.find(r => r.fingerprint === fingerprint);
  if (exact) return exact;

  // Prefix match (e.g., "coder_api_express" matches "coder_api_express_sqlite")
  const prefixMatches = routines.filter(r =>
    fingerprint.startsWith(r.fingerprint) || r.fingerprint.startsWith(fingerprint)
  );
  if (prefixMatches.length > 0) {
    return prefixMatches.sort((a, b) => b.confidence - a.confidence)[0];
  }

  return null;
}

// ── Full consolidation pipeline ─────────────────────────────────────────────

export function consolidate() {
  const episodes = loadEpisodes();
  if (episodes.length === 0) {
    return { episodes: 0, schemas: 0, routines: 0, message: "no episodes to consolidate" };
  }

  console.log(`[consolidate] ${episodes.length} episodes loaded`);

  // Induce schemas
  const schemas = induceSchemas(episodes);
  console.log(`[consolidate] ${schemas.length} schemas induced`);
  for (const schema of schemas) {
    saveSchema(schema);
    console.log(`  ${schema.schema_id}: ${schema.canonical_sequence.length} steps, ${schema.success_rate} success, ${schema.episode_count} episodes`);
  }

  // Compile routines
  let compiled = 0;
  let rejected = 0;
  for (const schema of schemas) {
    const routine = compileRoutine(schema, episodes);
    if (routine) {
      saveRoutine(routine);
      compiled++;
      console.log(`  ${routine.routine_id}: ${routine.steps.length} steps, confidence ${routine.confidence} [COMPILED]`);
    } else {
      rejected++;
      const gate = bmrGate(schema, episodes);
      console.log(`  ${schema.schema_id}: REJECTED (${gate.reason})`);
    }
  }

  // Tier usage summary
  const tierTotals = { tier1: 0, tier2: 0, tier3: 0 };
  let totalTokens = 0;
  let totalLatency = 0;
  for (const ep of episodes) {
    tierTotals.tier1 += ep.tier_usage?.tier1 || 0;
    tierTotals.tier2 += ep.tier_usage?.tier2 || 0;
    tierTotals.tier3 += ep.tier_usage?.tier3 || 0;
    totalTokens += ep.total_tokens || 0;
    totalLatency += ep.total_latency_ms || 0;
  }

  console.log(`[consolidate] Tier usage: T1=${tierTotals.tier1} T2=${tierTotals.tier2} T3=${tierTotals.tier3}`);
  console.log(`[consolidate] Total tokens: ${totalTokens}, Total latency: ${totalLatency}ms`);

  return {
    episodes: episodes.length,
    schemas: schemas.length,
    routines: compiled,
    rejected,
    tier_usage: tierTotals,
    total_tokens: totalTokens,
    total_latency_ms: totalLatency,
  };
}
