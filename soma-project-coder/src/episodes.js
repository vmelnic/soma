/**
 * Episode capture — records coding sessions as SOMA-compatible episodes.
 *
 * Each session becomes an Episode with steps, observations, and a goal
 * fingerprint. Episodes are stored as JSON in data/episodes/ and used
 * by consolidate.js for schema induction + routine compilation.
 */

import { existsSync, mkdirSync, writeFileSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { projectRoot } from "./env.js";

const EPISODES_DIR = path.join(projectRoot, "data/episodes");

// FNV-1a 128-dim hash embedder (matches soma-next's HashEmbedder)
export function hashEmbed(text) {
  const dim = 128;
  const vec = new Float32Array(dim);
  let h = 2166136261 >>> 0;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 16777619) >>> 0;
  }
  for (let i = 0; i < dim; i++) {
    h ^= (h >>> 13);
    h = Math.imul(h, 0x5bd1e995) >>> 0;
    h ^= (h >>> 15);
    vec[i] = ((h & 0xff) / 127.5) - 1.0;
  }
  return Array.from(vec);
}

export class EpisodeRecorder {
  constructor(goalFingerprint, objective) {
    this.episode = {
      episode_id: crypto.randomUUID(),
      goal_fingerprint: goalFingerprint,
      objective,
      initial_belief_summary: {},
      steps: [],
      outcome: null,
      tags: ["coder", goalFingerprint],
      embedding: hashEmbed(goalFingerprint),
      salience: 1.0,
      world_state_context: {},
      started_at: new Date().toISOString(),
      finished_at: null,
      tier_usage: { tier1: 0, tier2: 0, tier3: 0 },
      total_tokens: 0,
      total_latency_ms: 0,
    };
  }

  recordStep(step) {
    // step: { skill_id, input, observation, success, latencyMs, llmResult?, tier? }
    const tier = step.tier || 2;
    const tokens = step.llmResult?.tokens || 0;

    this.episode.steps.push({
      step_index: this.episode.steps.length,
      skill_id: step.skill_id,
      input_summary: summarizeInput(step.input),
      observation: {
        success: step.success,
        latency_ms: step.latencyMs || 0,
        output_summary: step.observation ? String(step.observation).slice(0, 500) : null,
      },
      tier,
      tokens,
      timestamp: new Date().toISOString(),
    });

    this.episode.tier_usage[`tier${tier}`]++;
    this.episode.total_tokens += tokens;
    this.episode.total_latency_ms += step.latencyMs || 0;
  }

  finish(success) {
    this.episode.outcome = success ? "success" : "failure";
    this.episode.finished_at = new Date().toISOString();
    this.save();
    return this.episode;
  }

  save() {
    mkdirSync(EPISODES_DIR, { recursive: true });
    const filename = `${this.episode.started_at.replace(/[:.]/g, "-")}_${this.episode.episode_id.slice(0, 8)}_${this.episode.goal_fingerprint}.json`;
    writeFileSync(
      path.join(EPISODES_DIR, filename),
      JSON.stringify(this.episode, null, 2)
    );
  }

  /** Skill sequence for this episode (used by PrefixSpan). */
  skillSequence() {
    return this.episode.steps.map(s => s.skill_id);
  }
}

function summarizeInput(input) {
  if (!input) return null;
  const s = JSON.stringify(input);
  return s.length > 200 ? s.slice(0, 200) + "..." : s;
}

/** Load all stored episodes. */
export function loadEpisodes() {
  if (!existsSync(EPISODES_DIR)) return [];
  return readdirSync(EPISODES_DIR)
    .filter(f => f.endsWith(".json"))
    .map(f => JSON.parse(readFileSync(path.join(EPISODES_DIR, f), "utf8")))
    .sort((a, b) => a.started_at.localeCompare(b.started_at));
}

/** Group episodes by goal_fingerprint. */
export function groupByFingerprint(episodes) {
  const groups = {};
  for (const ep of episodes) {
    const fp = ep.goal_fingerprint;
    if (!groups[fp]) groups[fp] = [];
    groups[fp].push(ep);
  }
  return groups;
}
