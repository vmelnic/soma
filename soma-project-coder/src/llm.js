/**
 * Multi-tier LLM client.
 *
 * Tier 2: Qwen3-32B on Ollama (Windows RTX 3090) — local, free, fast.
 * Tier 3: Claude API — fallback for complex reasoning.
 *
 * The caller never picks tiers manually. callLLM() tries local first,
 * falls back on timeout/error, and records which tier handled the request.
 */

import { loadEnv } from "./env.js";

// ── Ollama (Tier 2) ─────────────────────────────────────────────────────────

async function callOllama(system, prompt, opts = {}) {
  const env = await loadEnv();
  const host = env.OLLAMA_HOST || "http://127.0.0.1:11434";
  const model = env.OLLAMA_MODEL || "dieKeule/qwen3.6_27b:latest";
  const thinking = !!opts.thinking;
  const timeout = opts.timeout ?? (thinking ? 600_000 : 300_000);

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);

  try {
    const body = {
      model,
      system,
      prompt,
      stream: false,
      think: thinking,
      options: {
        temperature: opts.temperature ?? 0.3,
        num_predict: thinking ? Math.max(opts.maxTokens ?? 4096, 8192) : (opts.maxTokens ?? 4096),
        num_ctx: opts.numCtx ?? 4096,
      },
    };

    const res = await fetch(`${host}/api/generate`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
      signal: controller.signal,
    });

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`Ollama ${res.status}: ${text.slice(0, 200)}`);
    }

    const json = await res.json();
    const evalCount = json.eval_count || 0;
    const evalDuration = json.eval_duration || 1;
    const tokPerSec = evalCount / (evalDuration / 1e9);

    // Qwen3.6 with think:true puts reasoning in `thinking` field, answer in `response`.
    // If response is empty but thinking has content, the model ran out of tokens on reasoning.
    let text = json.response || "";
    if (!text && json.thinking) {
      text = json.thinking;
    }

    return {
      text,
      tier: 2,
      model,
      tokens: evalCount,
      tokPerSec: Math.round(tokPerSec * 10) / 10,
      latencyMs: json.total_duration ? Math.round(json.total_duration / 1e6) : 0,
    };
  } finally {
    clearTimeout(timer);
  }
}

// ── OpenAI (Tier 3) ─────────────────────────────────────────────────────────

async function callOpenAI(system, prompt, opts = {}) {
  const env = await loadEnv();
  const apiKey = env.OPENAI_API_KEY;
  if (!apiKey) throw new Error("OPENAI_API_KEY not set — cannot escalate");
  const model = env.OPENAI_MODEL || "gpt-4o-mini";

  const start = Date.now();
  const res = await fetch("https://api.openai.com/v1/chat/completions", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model,
      max_tokens: opts.maxTokens ?? 8192,
      temperature: opts.temperature ?? 0.3,
      messages: [
        { role: "system", content: system },
        { role: "user", content: prompt },
      ],
    }),
  });

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`OpenAI ${res.status}: ${text.slice(0, 200)}`);
  }

  const json = await res.json();
  const text = json.choices[0].message.content;
  const outputTokens = json.usage?.completion_tokens || 0;
  const inputTokens = json.usage?.prompt_tokens || 0;

  return {
    text,
    tier: 3,
    model,
    tokens: outputTokens,
    inputTokens,
    latencyMs: Date.now() - start,
  };
}

// ── Multi-tier dispatcher ────────────────────────────────────────────────────

const STALL_PATTERNS = [
  /i (?:cannot|can't|don't know how to)/i,
  /as an ai/i,
  /i'm not sure/i,
  /```\s*```/,          // empty code block
];

function looksStalled(text) {
  if (!text || text.trim().length === 0) return true;
  return STALL_PATTERNS.some(p => p.test(text));
}

/**
 * Call the LLM with automatic tier selection.
 *
 * @param {string} system  - System prompt
 * @param {string} prompt  - User prompt
 * @param {object} opts    - { temperature, maxTokens, thinking, forceCloud, timeout }
 * @returns {{ text, tier, model, tokens, latencyMs, escalated? }}
 */
export async function callLLM(system, prompt, opts = {}) {
  if (opts.forceCloud) {
    return callOpenAI(system, prompt, opts);
  }

  // Try Ollama first
  try {
    const result = await callOllama(system, prompt, opts);

    if (!looksStalled(result.text)) return result;

    // Stall detected — escalate
    console.log(`  [llm] stall detected on Tier 2, escalating to Tier 3...`);
  } catch (err) {
    console.log(`  [llm] Tier 2 failed: ${err.message}, escalating...`);
  }

  // Fallback to Anthropic
  try {
    const result = await callOpenAI(system, prompt, opts);
    result.escalated = true;
    return result;
  } catch (err) {
    throw new Error(`All tiers failed. Tier 3: ${err.message}`);
  }
}

/**
 * Parse a JSON response from LLM output, handling markdown fences and prose.
 */
export function parseJsonResponse(text) {
  let cleaned = text.trim();
  if (cleaned.startsWith("```"))
    cleaned = cleaned.replace(/^```(?:json)?\s*/, "").replace(/\s*```$/, "");
  if (!cleaned.startsWith("[") && !cleaned.startsWith("{")) {
    const arrMatch = cleaned.match(/\[[\s\S]*\]/);
    const objMatch = cleaned.match(/\{[\s\S]*\}/);
    if (arrMatch) cleaned = arrMatch[0];
    else if (objMatch) cleaned = objMatch[0];
  }
  return JSON.parse(cleaned);
}
