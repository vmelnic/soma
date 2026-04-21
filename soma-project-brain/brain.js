#!/usr/bin/env node
/**
 * soma-project-brain — External brain for SOMA via MCP.
 *
 * Spawns SOMA as a child process (stdio MCP), submits a goal, and acts as the
 * brain: polls for WaitingForInput, reads pending_input_request + belief
 * projection, calls an LLM (Claude Haiku by default) to compose the missing
 * bindings, and provides them via provide_session_input.
 *
 * The body does the orchestration. The brain answers narrow questions.
 *
 * Usage:
 *   node brain.js "create a users table and insert Alice and Bob"
 */

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const projectRoot = path.dirname(fileURLToPath(import.meta.url));
const bodyRoot = path.resolve(projectRoot, "../soma-project-body");

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
  const paths = [
    path.join(projectRoot, ".env"),
    path.join(bodyRoot, ".env"),
  ];
  const entries = {};
  for (const p of paths) {
    if (existsSync(p)) {
      Object.assign(entries, parseDotEnv(await readFile(p, "utf8")));
    }
  }
  envCache = { ...entries, ...process.env };
  return envCache;
}

// ---------------------------------------------------------------------------
// Stdio MCP client
// ---------------------------------------------------------------------------

class StdioMcpClient {
  constructor(command, args, cwd) {
    this.command = command;
    this.args = args;
    this.cwd = cwd;
    this.nextId = 1;
    this.pending = new Map();
    this.notificationHandlers = new Map();
  }

  onNotification(method, handler) {
    let list = this.notificationHandlers.get(method);
    if (!list) {
      list = [];
      this.notificationHandlers.set(method, list);
    }
    list.push(handler);
  }

  async start() {
    this.child = spawn(this.command, this.args, {
      cwd: this.cwd,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child.stderr.on("data", (chunk) => process.stderr.write(chunk));
    this.child.on("exit", (code) => {
      for (const { reject } of this.pending.values())
        reject(new Error(`MCP server exited with code ${code}`));
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
      // JSON-RPC notification: has method, no id
      if (payload.method && payload.id === undefined) {
        const handlers = this.notificationHandlers.get(payload.method);
        if (handlers) {
          for (const h of handlers) h(payload.params);
        }
        return;
      }
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
// LLM client (Anthropic Messages API)
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
      max_tokens: 1024,
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
// Brain logic
// ---------------------------------------------------------------------------

const SYSTEM_PROMPT = `You are a brain for SOMA, a goal-driven runtime. The body (SOMA) runs a control loop that selects skills and executes them via ports. When it can't resolve an input, it pauses and asks you.

You will receive:
- The goal objective
- The skill that was selected
- The missing input slots (name + JSON schema)
- The projected belief state (TOON-encoded, minimal)

Respond with ONLY a valid JSON object mapping slot names to values. No explanation, no markdown fences, no commentary. Just the JSON object.

If the selected skill is WRONG for this goal, include "_redirect_skill": "<correct_skill_id>" in your response to override the body's selection. Only do this when the skill clearly doesn't match the goal.

Examples:
- Missing slot: sql (string, "SQL SELECT statement") → {"sql": "SELECT * FROM users"}
- Missing slots: table (string), columns (string) → {"table": "users", "columns": "id INTEGER PRIMARY KEY, name TEXT NOT NULL"}
- Wrong skill selected: {"_redirect_skill": "soma.crypto.sha256", "data": "hello world"}`;

function buildUserPrompt(objective, pendingRequest, beliefProjection, currentSubGoal) {
  const lines = [`Goal: ${objective}`, ""];

  if (currentSubGoal) {
    lines.push(`Current step: ${currentSubGoal}`);
    lines.push("");
  }

  if (pendingRequest) {
    lines.push(`Skill: ${pendingRequest.skill_id}`);
    lines.push("Missing slots:");
    for (const slot of pendingRequest.missing_slots) {
      const desc = slot.schema?.description || slot.schema?.type || "any";
      lines.push(`  - ${slot.name} (${desc})`);
    }
  }

  if (beliefProjection) {
    lines.push("");
    lines.push(`Belief context: ${beliefProjection}`);
  }

  return lines.join("\n");
}

function parseJsonResponse(text) {
  let cleaned = text.trim();
  if (cleaned.startsWith("```")) {
    cleaned = cleaned.replace(/^```(?:json)?\s*/, "").replace(/\s*```$/, "");
  }
  return JSON.parse(cleaned);
}

// ---------------------------------------------------------------------------
// Orchestrator — goal decomposition + routine chaining
// ---------------------------------------------------------------------------

function isComplexGoal(objective) {
  const markers = [" then ", " and then ", ", then ", ";"];
  const lower = objective.toLowerCase();
  return markers.some((m) => lower.includes(m));
}

let cachedSkills = null;
async function getSkills(soma) {
  if (cachedSkills) return cachedSkills;
  const skillsResult = await soma.callTool("inspect_skills", {});
  const skillsContent = extractToolContent(skillsResult);
  cachedSkills = skillsContent.skills || [];
  return cachedSkills;
}

async function decomposeAndMap(objective, skills) {
  const skillSummaries = skills.map(
    (s) => `${s.skill_id}: ${s.name} — ${s.description || ""} [${(s.tags || []).join(", ")}]`
  ).join("\n");

  const prompt = `You decompose goals into ordered sub-goals and map each to an available SOMA skill.

Given a goal and a list of available skills, return a JSON array where each entry has:
- "description": what this sub-goal achieves (short, imperative)
- "skill_id": the exact skill_id from the list below that best matches

IMPORTANT: Only use skill_ids from this list. Prefer sqlite skills for database operations.

Available skills:
${skillSummaries}

Respond with ONLY the JSON array. No explanation.

Examples:
Goal: "create a users table, insert Alice and Bob, list all users"
[
  {"description": "create users table", "skill_id": "soma.ports.sqlite.create_table"},
  {"description": "insert Alice", "skill_id": "soma.ports.sqlite.insert"},
  {"description": "insert Bob", "skill_id": "soma.ports.sqlite.insert"},
  {"description": "list all users", "skill_id": "soma.ports.sqlite.query"}
]`;

  const response = await callLLM(prompt, objective);
  return parseJsonResponse(response);
}

async function precomputePlan(soma, objective) {
  // Check for existing routines
  const routineResult = await soma.callTool("find_routines", {
    query: objective,
    limit: 3,
  });
  const routineContent = extractToolContent(routineResult);
  if (routineContent.routines?.length > 0) {
    const top = routineContent.routines[0];
    if (top.similarity > 0.8 && top.steps > 0) {
      console.log(`[orchestrator] Found matching routine: ${top.routine_id} (sim=${top.similarity.toFixed(2)})`);
      return { steps: [{ type: "sub_routine", routine_id: top.routine_id }], descriptions: [] };
    }
  }

  // Single LLM call: decompose + map to skills
  const skills = await getSkills(soma);
  const skillIds = new Set(skills.map((s) => s.skill_id));
  console.log("[orchestrator] Decomposing goal and mapping skills (single LLM call)...");
  const subgoals = await decomposeAndMap(objective, skills);
  console.log(`[orchestrator] ${subgoals.length} sub-goals: ${subgoals.map((s) => s.description).join(" → ")}`);

  const steps = [];
  const descriptions = [];
  for (const sg of subgoals) {
    const subResult = await soma.callTool("find_routines", {
      query: sg.description,
      limit: 1,
    });
    const subContent = extractToolContent(subResult);
    if (subContent.routines?.length > 0 && subContent.routines[0].similarity > 0.8) {
      const r = subContent.routines[0];
      console.log(`[orchestrator]   "${sg.description}" → routine ${r.routine_id}`);
      steps.push({ type: "sub_routine", routine_id: r.routine_id });
    } else if (sg.skill_id && skillIds.has(sg.skill_id)) {
      console.log(`[orchestrator]   "${sg.description}" → skill ${sg.skill_id}`);
      steps.push({ type: "skill", skill_id: sg.skill_id });
    } else {
      console.log(`[orchestrator]   "${sg.description}" → skill_id "${sg.skill_id}" not found, skipping`);
      continue;
    }
    descriptions.push(sg.description);
  }
  return { steps, descriptions };
}


// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  const objective = process.argv[2];
  if (!objective) {
    console.error("Usage: node brain.js <goal-objective>");
    process.exit(1);
  }

  const maxSteps = parseInt(process.argv[3] || "10", 10);
  const env = await loadEnv();

  // Build SOMA launch command (reuse soma-project-body's layout)
  const somaPath = path.join(bodyRoot, "bin/soma");
  if (!existsSync(somaPath)) {
    console.error(`Missing SOMA binary: ${somaPath}`);
    process.exit(1);
  }

  const packArgs = [];
  const packsDir = path.join(bodyRoot, "packs");
  if (existsSync(packsDir)) {
    const { readdirSync } = await import("node:fs");
    for (const entry of readdirSync(packsDir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        const manifest = path.join(packsDir, entry.name, "manifest.json");
        if (existsSync(manifest)) {
          packArgs.push("--pack", manifest);
        }
      }
    }
  }

  console.log(`[brain] Starting SOMA with ${packArgs.length / 2} packs...`);
  const soma = new StdioMcpClient(somaPath, ["--mcp", ...packArgs], bodyRoot);
  await soma.start();
  console.log("[brain] SOMA connected via MCP.");

  try {
    // 1a. Pre-compute plan for complex goals BEFORE creating the goal
    let precomputedPlan = null;
    let stepDescriptions = [];
    if (isComplexGoal(objective)) {
      try {
        const result = await precomputePlan(soma, objective);
        precomputedPlan = result.steps;
        stepDescriptions = result.descriptions;
      } catch (err) {
        console.error(`[brain] Plan precompute failed: ${err.message}, will use body deliberation.`);
      }
    }

    // 1b. Create async goal
    console.log(`[brain] Goal: "${objective}" (max_steps: ${maxSteps})`);
    const goalResult = await soma.callTool("create_goal_async", {
      objective,
      max_steps: maxSteps,
    });
    const goalContent = extractToolContent(goalResult);
    const goalId = goalContent.goal_id;
    const sessionId = goalContent.session_id;
    console.log(`[brain] Goal created: ${goalId} (session: ${sessionId})`);

    // 1c. Inject pre-computed plan immediately (before background thread progresses)
    if (precomputedPlan && precomputedPlan.length > 0) {
      try {
        const injectResult = await soma.callTool("inject_plan", {
          session_id: sessionId,
          steps: precomputedPlan,
        });
        const injected = extractToolContent(injectResult);
        console.log(`[brain] Plan injected: ${injected.steps_injected} steps.`);
      } catch (err) {
        console.error(`[brain] Plan injection failed: ${err.message}`);
      }
    }

    // 2. Push-notification listener (alongside poll fallback)
    let lastPushedStep = -1;
    soma.onNotification("notifications/goal/trace_step", (params) => {
      if (params.goal_id !== goalId) return;
      const ev = params.event;
      if (ev) {
        const ok = ev.observation_success ? "ok" : "FAIL";
        console.log(
          `[push] step ${ev.step_index}: ${ev.selected_skill} [${ok}] Δ${ev.progress_delta}`
        );
        lastPushedStep = ev.step_index;
      }
      if (params.terminal) {
        console.log(`[push] goal terminal — status: ${params.status}`);
      }
    });

    // Poll loop (still needed for WaitingForInput reaction; push gives live trace)
    let brainCalls = 0;
    const maxBrainCalls = 20;

    while (brainCalls < maxBrainCalls) {
      await sleep(500);

      const statusResult = await soma.callTool("get_goal_status", {
        goal_id: goalId,
      });
      const status = extractToolContent(statusResult);

      if (status.status === "completed" || status.status === "failed") {
        console.log(
          `[brain] Goal ${status.status} after ${status.steps} steps.`
        );
        if (status.error) console.log(`[brain] Last error: ${status.error}`);
        break;
      }

      if (status.status !== "waiting_for_input") {
        continue;
      }

      // 3. Session is waiting — act as the brain
      brainCalls++;
      console.log(
        `[brain] WaitingForInput detected (brain call #${brainCalls})`
      );

      // Read session state
      const sessionResult = await soma.callTool("inspect_session", {
        session_id: sessionId,
      });
      const session = extractToolContent(sessionResult);
      const pendingRequest = session.working_memory?.pending_input_request;

      if (!pendingRequest) {
        // Check if this is a policy confirmation request
        if (status.error && status.error.includes("confirm")) {
          console.log(`[brain] Policy confirmation needed: ${status.error}`);
          try {
            await soma.callTool("provide_session_input", {
              session_id: sessionId,
              bindings: { confirmed: true },
            });
            console.log("[brain] Policy confirmed.");
          } catch (err) {
            console.error(`[brain] confirm failed: ${err.message}`);
            break;
          }
          continue;
        }
        console.log("[brain] No pending_input_request — resuming without input.");
        await soma.callTool("resume_session", { session_id: sessionId });
        continue;
      }

      // Read belief projection
      let beliefToon = null;
      try {
        const projResult = await soma.callTool("inspect_belief_projection", {
          session_id: sessionId,
        });
        const proj = extractToolContent(projResult);
        beliefToon = proj.toon_encoded;
      } catch {
        // projection not critical
      }

      // 4. Ask LLM to compose the missing bindings
      const planStep = session.working_memory?.plan_step ?? 0;
      const currentSubGoal = stepDescriptions[planStep] || null;
      const userPrompt = buildUserPrompt(
        objective,
        pendingRequest,
        beliefToon,
        currentSubGoal
      );
      console.log(`[brain] Skill: ${pendingRequest.skill_id} — missing: ${pendingRequest.missing_slots.map((s) => s.name).join(", ")}`);

      let bindings;
      try {
        const llmResponse = await callLLM(SYSTEM_PROMPT, userPrompt);
        console.log(`[brain] LLM response: ${llmResponse.slice(0, 200)}`);
        bindings = parseJsonResponse(llmResponse);
      } catch (err) {
        console.error(`[brain] LLM error: ${err.message}`);
        break;
      }

      // 5. Provide the bindings (with optional skill redirect)
      const redirectSkill = bindings._redirect_skill;
      if (redirectSkill) {
        delete bindings._redirect_skill;
        console.log(`[brain] Redirecting skill → ${redirectSkill}`);
      }
      try {
        const args = {
          session_id: sessionId,
          bindings,
        };
        if (redirectSkill) args.redirect_skill_id = redirectSkill;
        const provideResult = await soma.callTool("provide_session_input", args);
        const provided = extractToolContent(provideResult);
        const redir = provided.redirected_to ? `, redirected → ${provided.redirected_to}` : "";
        console.log(
          `[brain] Injected ${provided.bindings_injected} binding(s), status: ${provided.status}${redir}`
        );
      } catch (err) {
        console.error(`[brain] provide_session_input failed: ${err.message}`);
        break;
      }
    }

    if (brainCalls >= maxBrainCalls) {
      console.log(`[brain] Reached max brain calls (${maxBrainCalls}).`);
    }

    // Final status
    const finalResult = await soma.callTool("get_goal_status", {
      goal_id: goalId,
    });
    const finalStatus = extractToolContent(finalResult);
    console.log(`[brain] Final: ${JSON.stringify(finalStatus, null, 2)}`);
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

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

main().catch((err) => {
  console.error(`[brain] Fatal: ${err.message}`);
  process.exit(1);
});
