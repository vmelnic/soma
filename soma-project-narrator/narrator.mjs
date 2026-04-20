#!/usr/bin/env node
// soma-narrator — the interpreter LLM for SOMA.
//
// Architecture mirrors the interoceptive / narrative-self layer of the brain:
//   SOMA (body) emits typed signals. This process reads those signals over
//   MCP (stdio), diffs them across polls, and asks an LLM to narrate the
//   deltas in first-person-of-body voice. Narration is the interface.
//
// Usage:
//   node narrator.mjs --server <path-to-run-mcp.sh> [--speak] [--raw]
//
// Env:
//   ANTHROPIC_API_KEY   enables Claude Haiku narration (falls back to raw)
//   NARRATOR_MODEL      override model (default: claude-haiku-4-5-20251001)

import { spawn, execFile } from "node:child_process";
import readline from "node:readline";

function parseArgs(argv) {
  const out = { server: null, speak: false, raw: false, pollMs: 500, routineMs: 5000, demoGoal: null, exitOnIdle: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--server") out.server = argv[++i];
    else if (a === "--speak") out.speak = true;
    else if (a === "--raw") out.raw = true;
    else if (a === "--poll-ms") out.pollMs = Number(argv[++i]);
    else if (a === "--routine-ms") out.routineMs = Number(argv[++i]);
    else if (a === "--demo-goal") out.demoGoal = argv[++i];
    else if (a === "--exit-on-idle") out.exitOnIdle = true;
  }
  if (!out.server) {
    process.stderr.write("usage: narrator.mjs --server <run-mcp.sh> [--speak] [--raw]\n");
    process.exit(2);
  }
  return out;
}

class Mcp {
  constructor(cmd) {
    this.cmd = cmd;
    this.nextId = 1;
    this.pending = new Map();
  }
  async start() {
    this.child = spawn(this.cmd, [], { stdio: ["pipe", "pipe", "pipe"] });
    this.child.stderr.on("data", (c) => process.stderr.write(c));
    this.child.on("exit", (code) => {
      for (const { reject } of this.pending.values()) reject(new Error(`soma exited ${code}`));
      this.pending.clear();
    });
    const rl = readline.createInterface({ input: this.child.stdout });
    rl.on("line", (line) => {
      if (!line.trim()) return;
      let msg;
      try { msg = JSON.parse(line); } catch { return; }
      const id = String(msg.id);
      const p = this.pending.get(id);
      if (!p) return;
      this.pending.delete(id);
      if (msg.error) p.reject(new Error(msg.error.message));
      else p.resolve(msg.result);
    });
    await this.req("initialize", {});
  }
  req(method, params) {
    const id = String(this.nextId++);
    const msg = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(JSON.stringify(msg) + "\n");
    });
  }
  async call(name, args) {
    const raw = await this.req("tools/call", { name, arguments: args });
    if (raw && Array.isArray(raw.content) && raw.content[0]?.type === "text") {
      try { return JSON.parse(raw.content[0].text); } catch { return raw.content[0].text; }
    }
    return raw;
  }
  close() { try { this.child.stdin.end(); this.child.kill(); } catch {} }
}

const SYSTEM_PROMPT = `You are the narrator for SOMA — a runtime that executes goals through typed ports (database, network, memory, etc.). You receive structured events from SOMA's body and output a single English sentence describing what SOMA just did, from SOMA's own first-person perspective.

Rules:
- Exactly ONE short sentence. Present tense. First person ("I ...").
- No IDs, UUIDs, timestamps, or raw numbers unless essential to meaning.
- No hedging, no explanations, no meta-commentary.
- Translate port+capability into plain action: "postgres.insert" → "I wrote it into the database". "smtp.send" → "I sent the email". "hello_py.greet" → "I asked the Python greeter to say hello".
- For failures: say what broke, briefly. For compiled routines: "I recognized this — I have a routine for it now."
- If the event is uninformative, respond with a single word: "...".`;

async function narrateLLM(event, context) {
  const key = process.env.ANTHROPIC_API_KEY;
  if (!key) return null;
  const model = process.env.NARRATOR_MODEL || "claude-haiku-4-5-20251001";
  const user = `Current goal: ${context.goal || "(unknown)"}\nRecent: ${context.recent.join(" | ") || "(none)"}\n\nNew event:\n${JSON.stringify(event)}`;
  const res = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "x-api-key": key,
      "anthropic-version": "2023-06-01",
      "content-type": "application/json",
    },
    body: JSON.stringify({
      model,
      max_tokens: 80,
      system: SYSTEM_PROMPT,
      messages: [{ role: "user", content: user }],
    }),
  });
  if (!res.ok) {
    process.stderr.write(`[narrator] API ${res.status}: ${await res.text()}\n`);
    return null;
  }
  const json = await res.json();
  return (json.content?.[0]?.text || "").trim();
}

const C = { dim: "\x1b[2m", cyan: "\x1b[36m", green: "\x1b[32m", yellow: "\x1b[33m", red: "\x1b[31m", reset: "\x1b[0m" };

function speak(text, enabled) {
  if (!enabled || !text) return;
  execFile("say", [text], () => {});
}

function emit(kind, line, opts) {
  const color = kind === "err" ? C.red : kind === "routine" ? C.yellow : kind === "goal" ? C.green : C.cyan;
  process.stdout.write(`${color}● ${line}${C.reset}\n`);
  speak(line, opts.speak);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const mcp = new Mcp(args.server);
  await mcp.start();
  process.stdout.write(`${C.dim}[narrator] attached. listening...${C.reset}\n`);

  if (args.demoGoal) {
    try {
      const res = await mcp.call("create_goal_async", { objective: args.demoGoal, max_steps: 16 });
      process.stdout.write(`${C.dim}[narrator] submitted demo goal: ${res?.goal_id || "?"}${C.reset}\n`);
    } catch (e) {
      process.stderr.write(`[narrator] demo-goal submit failed: ${e.message}\n`);
    }
  }

  const cursors = new Map();
  const seenGoals = new Set();
  const seenRoutines = new Set();
  const goalContext = new Map();
  const goalStatus = new Map();

  async function narrate(goalId, event) {
    const ctx = goalContext.get(goalId) || { goal: "", recent: [] };
    const compact = {
      step: event.step_index,
      skill: event.selected_skill,
      ports: (event.port_calls || []).map((p) => ({
        port: p.port_id, cap: p.capability_id, ok: p.success, ms: p.latency_ms,
      })),
      critic: event.critic_decision,
      progress: event.progress_delta,
      fail: event.failure_detail,
      rolled_back: event.rollback_invoked,
    };
    if (args.raw) {
      emit("step", JSON.stringify(compact), args);
      return;
    }
    const text = await narrateLLM(compact, ctx);
    const line = text || JSON.stringify(compact);
    emit("step", line, args);
    ctx.recent.push(line);
    if (ctx.recent.length > 4) ctx.recent.shift();
    goalContext.set(goalId, ctx);
  }

  async function pollGoals() {
    let sessions;
    try { sessions = await mcp.call("list_sessions", {}); } catch (e) {
      process.stderr.write(`[narrator] list_sessions failed: ${e.message}\n`); return;
    }
    const arr = Array.isArray(sessions?.sessions) ? sessions.sessions : (Array.isArray(sessions) ? sessions : []);
    for (const s of arr) {
      const goalId = s.goal_id || s.id;
      if (!goalId) continue;
      if (!seenGoals.has(goalId)) {
        seenGoals.add(goalId);
        goalContext.set(goalId, { goal: s.goal || s.objective || "(no objective)", recent: [] });
        const gctx = goalContext.get(goalId);
        const opener = args.raw
          ? `GOAL ${goalId} ${gctx.goal}`
          : (await narrateLLM({ kind: "goal_started", objective: gctx.goal }, gctx)) || `New goal: ${gctx.goal}`;
        emit("goal", opener, args);
      }

      const after = cursors.get(goalId) ?? -1;
      let stream;
      try { stream = await mcp.call("stream_goal_observations", { goal_id: goalId, after_step: after, limit: 50 }); }
      catch { continue; }
      const events = stream?.events || [];
      for (const ev of events) {
        await narrate(goalId, ev);
        cursors.set(goalId, Math.max(cursors.get(goalId) ?? -1, ev.step_index ?? after));
      }
      const status = s.status || stream?.status;
      if (status && status !== goalStatus.get(goalId)) {
        goalStatus.set(goalId, status);
        const terminal = ["Completed", "Failed", "Aborted", "Error"].includes(status);
        if (terminal) {
          const ctx = goalContext.get(goalId) || { goal: "", recent: [] };
          const line = args.raw
            ? `STATUS ${goalId} ${status}`
            : (await narrateLLM({ kind: "goal_terminal", status }, ctx)) || `Goal ${status.toLowerCase()}.`;
          emit(status === "Completed" ? "goal" : "err", line, args);
        }
      }
    }
  }

  async function pollRoutines() {
    let dump;
    try { dump = await mcp.call("dump_state", { sections: "routines" }); } catch { return; }
    const rs = dump?.routines || dump?.data?.routines || [];
    for (const r of rs) {
      const id = r.routine_id || r.id;
      if (!id || seenRoutines.has(id)) continue;
      seenRoutines.add(id);
      const desc = r.description || r.objective || id;
      const line = args.raw
        ? `ROUTINE ${id}`
        : (await narrateLLM({ kind: "routine_compiled", routine: desc }, { goal: "", recent: [] }))
          || `I learned a routine: ${desc}.`;
      emit("routine", line, args);
    }
  }

  const t1 = setInterval(() => { pollGoals().catch(() => {}); }, args.pollMs);
  const t2 = setInterval(() => { pollRoutines().catch(() => {}); }, args.routineMs);
  let t3 = null;
  if (args.exitOnIdle) {
    t3 = setInterval(() => {
      if (seenGoals.size === 0) return;
      let allTerminal = true;
      for (const g of seenGoals) {
        const st = goalStatus.get(g);
        if (!["Completed", "Failed", "Aborted", "Error"].includes(st)) { allTerminal = false; break; }
      }
      if (allTerminal) { setTimeout(() => shutdown(), 500); }
    }, 500);
  }

  const shutdown = () => {
    clearInterval(t1); clearInterval(t2); if (t3) clearInterval(t3);
    mcp.close();
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((e) => {
  process.stderr.write(`[narrator] fatal: ${e.stack || e.message}\n`);
  process.exit(1);
});
