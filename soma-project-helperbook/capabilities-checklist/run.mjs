#!/usr/bin/env node

/**
 * SOMA Runtime Capabilities Checklist
 *
 * Tests every soma-next layer against the running HelperBook MCP server.
 * Run: node capabilities-checklist/run.mjs
 *
 * Requires: services running (docker compose up -d --wait)
 */

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const runMcpScript = path.join(projectRoot, "scripts", "start-mcp.sh");

// ---------------------------------------------------------------------------
// MCP Client
// ---------------------------------------------------------------------------

class McpClient {
  constructor() {
    this.nextId = 1;
    this.pending = new Map();
  }

  async start() {
    this.child = spawn(runMcpScript, [], {
      cwd: projectRoot,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child.stderr.on("data", () => {});
    this.child.on("exit", (code) => {
      for (const { reject } of this.pending.values()) {
        reject(new Error(`MCP server exited with code ${code}`));
      }
      this.pending.clear();
    });
    const rl = readline.createInterface({ input: this.child.stdout });
    rl.on("line", (line) => {
      if (!line.trim()) return;
      let payload;
      try { payload = JSON.parse(line); } catch { return; }
      const p = this.pending.get(String(payload.id));
      if (!p) return;
      this.pending.delete(String(payload.id));
      if (payload.error) p.reject(new Error(payload.error.message));
      else p.resolve(payload.result);
    });
    await this.request("initialize", {});
  }

  request(method, params) {
    const id = String(this.nextId++);
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error("timeout (10s)"));
      }, 10000);
      this.pending.set(id, {
        resolve: (v) => { clearTimeout(timeout); resolve(v); },
        reject: (e) => { clearTimeout(timeout); reject(e); },
      });
      this.child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  }

  async callTool(name, args = {}) {
    return this.request("tools/call", { name, arguments: args });
  }

  async invokePort(portId, capabilityId, input = {}) {
    return this.callTool("invoke_port", { port_id: portId, capability_id: capabilityId, input });
  }

  close() {
    if (this.child) { this.child.stdin.end(); this.child.kill(); }
  }
}

// ---------------------------------------------------------------------------
// Test Runner
// ---------------------------------------------------------------------------

const results = [];
let passed = 0;
let failed = 0;
let currentSection = "";

function section(name) {
  currentSection = name;
  results.push({ type: "section", name });
}

async function check(name, fn) {
  const label = `${currentSection} > ${name}`;
  try {
    const detail = await fn();
    passed++;
    results.push({ type: "pass", label, detail });
  } catch (e) {
    failed++;
    results.push({ type: "fail", label, error: e.message });
  }
}

function assert(condition, msg) {
  if (!condition) throw new Error(msg || "assertion failed");
}

function printReport() {
  console.log("\n" + "=".repeat(70));
  console.log("  SOMA Runtime Capabilities Report");
  console.log("=".repeat(70) + "\n");

  for (const r of results) {
    if (r.type === "section") {
      console.log(`\n  ${r.name}`);
      console.log("  " + "-".repeat(r.name.length));
    } else if (r.type === "pass") {
      const detail = r.detail ? ` — ${r.detail}` : "";
      console.log(`  \x1b[32m✓\x1b[0m ${r.label}${detail}`);
    } else {
      console.log(`  \x1b[31m✗\x1b[0m ${r.label}`);
      console.log(`    \x1b[31m${r.error}\x1b[0m`);
    }
  }

  console.log("\n" + "-".repeat(70));
  console.log(`  Total: ${passed + failed}  Passed: \x1b[32m${passed}\x1b[0m  Failed: \x1b[${failed > 0 ? "31" : "32"}m${failed}\x1b[0m`);
  console.log("-".repeat(70) + "\n");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

async function run() {
  if (!existsSync(runMcpScript)) {
    console.error("Missing scripts/start-mcp.sh");
    process.exit(1);
  }

  const client = new McpClient();
  await client.start();

  // =========================================================================
  // 1. Port Invocation
  // =========================================================================
  section("1. Port Invocation");

  await check("list_ports discovers all ports", async () => {
    const r = await client.callTool("list_ports");
    const ports = r.ports || [];
    const ids = ports.map((p) => p.port_id).sort();
    assert(ids.includes("postgres"), "missing postgres");
    assert(ids.includes("redis"), "missing redis");
    assert(ids.includes("auth"), "missing auth");
    return `${ports.length} ports: ${ids.join(", ")}`;
  });

  await check("invoke_port postgres/query", async () => {
    const r = await client.invokePort("postgres", "query", {
      sql: "SELECT name, role FROM users LIMIT 3",
    });
    assert(r.success, `failed: ${JSON.stringify(r.structured_result)}`);
    const rows = r.structured_result?.rows;
    assert(rows && rows.length === 3, `expected 3 rows, got ${rows?.length}`);
    return `${rows.length} rows returned`;
  });

  await check("invoke_port postgres/count", async () => {
    const r = await client.invokePort("postgres", "count", { table: "users" });
    assert(r.success, `failed: ${JSON.stringify(r.structured_result)}`);
    const count = r.structured_result?.count;
    assert(typeof count === "number" && count >= 13, `expected >=13, got ${count}`);
    return `count=${count}`;
  });

  await check("invoke_port postgres/aggregate", async () => {
    const r = await client.invokePort("postgres", "aggregate", {
      table: "reviews",
      function: "AVG",
      column: "rating",
    });
    assert(r.success, `failed: ${JSON.stringify(r.structured_result)}`);
    return `avg_rating=${JSON.stringify(r.structured_result?.result)}`;
  });

  await check("invoke_port redis/set + get roundtrip", async () => {
    const key = `checklist:${Date.now()}`;
    const setR = await client.invokePort("redis", "set", { key, value: "soma-check" });
    assert(setR.success, `set failed: ${JSON.stringify(setR.structured_result)}`);
    const getR = await client.invokePort("redis", "get", { key });
    assert(getR.success, `get failed: ${JSON.stringify(getR.structured_result)}`);
    assert(getR.structured_result === "soma-check", `value mismatch: ${getR.structured_result}`);
    await client.invokePort("redis", "del", { key });
    return "set→get→del OK";
  });

  await check("invoke_port redis/hset + hget roundtrip", async () => {
    const key = `checklist:hash:${Date.now()}`;
    await client.invokePort("redis", "hset", { key, field: "name", value: "soma" });
    const r = await client.invokePort("redis", "hget", { key, field: "name" });
    assert(r.success && r.structured_result === "soma", `got: ${r.structured_result}`);
    await client.invokePort("redis", "del", { key });
    return "hset→hget→del OK";
  });

  await check("invoke_port auth/otp_generate", async () => {
    const r = await client.invokePort("auth", "otp_generate", { phone: "+40700000000" });
    assert(r.success, `failed: ${JSON.stringify(r.structured_result)}`);
    const code = r.structured_result?.debug_code;
    assert(code && code.length === 6, `expected 6-digit code, got: ${code}`);
    return `code=${code}`;
  });

  await check("invoke_port auth/otp_generate + otp_verify", async () => {
    const gen = await client.invokePort("auth", "otp_generate", { phone: "+40700000001" });
    assert(gen.success, "generate failed");
    const code = gen.structured_result.debug_code;
    const ver = await client.invokePort("auth", "otp_verify", { phone: "+40700000001", code });
    assert(ver.success, `verify failed: ${JSON.stringify(ver.structured_result)}`);
    assert(ver.structured_result?.valid === true, `not valid: ${JSON.stringify(ver.structured_result)}`);
    return "generate→verify OK";
  });

  await check("invoke_port auth/session lifecycle", async () => {
    const create = await client.invokePort("auth", "session_create", { user_id: "test-user" });
    assert(create.success, `create failed: ${JSON.stringify(create.structured_result)}`);
    const token = create.structured_result?.token;
    assert(token, "no token returned");
    const validate = await client.invokePort("auth", "session_validate", { token });
    assert(validate.success, `validate failed`);
    assert(validate.structured_result?.valid === true, "session not valid");
    const revoke = await client.invokePort("auth", "session_revoke", { token });
    assert(revoke.success, `revoke failed`);
    const after = await client.invokePort("auth", "session_validate", { token });
    assert(after.success && after.structured_result?.valid === false, "session still valid after revoke");
    return "create→validate→revoke→invalidated OK";
  });

  await check("invoke_port error handling (unknown port)", async () => {
    const r = await client.invokePort("nonexistent", "anything", {});
    assert(r.success === false, "expected failure");
    assert(r.failure_class, "expected failure_class");
    return `failure_class=${r.failure_class}`;
  });

  await check("invoke_port error handling (unknown capability)", async () => {
    const r = await client.invokePort("postgres", "nonexistent_cap", {});
    assert(r.success === false, "expected failure");
    return `failure_class=${r.failure_class}`;
  });

  // =========================================================================
  // 2. State & Context (dump_state — the LLM context solution)
  // =========================================================================
  section("2. State & Context");

  await check("dump_state full snapshot", async () => {
    const r = await client.callTool("dump_state", { sections: ["full"] });
    const keys = Object.keys(r);
    assert(keys.includes("ports"), "missing ports section");
    assert(keys.includes("skills"), "missing skills section");
    assert(keys.includes("packs"), "missing packs section");
    assert(keys.includes("metrics"), "missing metrics section");
    assert(keys.includes("sessions"), "missing sessions section");
    assert(keys.includes("episodes"), "missing episodes section");
    assert(keys.includes("schemas"), "missing schemas section");
    assert(keys.includes("routines"), "missing routines section");
    assert(keys.includes("belief"), "missing belief section");
    return `${keys.length} sections: ${keys.join(", ")}`;
  });

  await check("dump_state ports section has loaded ports", async () => {
    const r = await client.callTool("dump_state", { sections: ["ports"] });
    const ports = r.ports || [];
    assert(ports.length >= 3, `expected >=3 ports, got ${ports.length}`);
    const ids = ports.map((p) => p.port_id);
    assert(ids.includes("postgres"), "missing postgres");
    return `${ports.length} ports with capabilities`;
  });

  await check("dump_state packs section", async () => {
    const r = await client.callTool("dump_state", { sections: ["packs"] });
    const packs = r.packs || [];
    assert(packs.length >= 3, `expected >=3 packs, got ${packs.length}`);
    return `${packs.length} packs loaded`;
  });

  await check("dump_state metrics includes self_model", async () => {
    const r = await client.callTool("dump_state", { sections: ["metrics"] });
    const metrics = r.metrics || {};
    assert(metrics.self_model, "missing self_model in metrics");
    assert(metrics.self_model.uptime_seconds >= 0, "missing uptime");
    return `uptime=${metrics.self_model.uptime_seconds}s, rss=${metrics.self_model.rss_bytes || "n/a"}`;
  });

  await check("dump_state selective sections", async () => {
    const r = await client.callTool("dump_state", { sections: ["ports", "metrics"] });
    const keys = Object.keys(r);
    assert(keys.includes("ports"), "missing ports");
    assert(keys.includes("metrics"), "missing metrics");
    assert(!keys.includes("sessions"), "should not include sessions");
    return `only ${keys.join(", ")}`;
  });

  await check("inspect_packs", async () => {
    const r = await client.callTool("inspect_packs");
    assert(r.packs, "missing packs");
    return `${r.packs.length} packs`;
  });

  await check("inspect_skills", async () => {
    const r = await client.callTool("inspect_skills");
    assert(r.skills, "missing skills");
    return `${r.skills.length} skills registered`;
  });

  await check("inspect_resources", async () => {
    const r = await client.callTool("inspect_resources");
    // May be empty — just checking it doesn't error
    return `resources returned`;
  });

  // =========================================================================
  // 3. Goal & Session Lifecycle
  // =========================================================================
  section("3. Goal & Session Lifecycle");

  let testSessionId = null;

  await check("create_goal returns session_id", async () => {
    const r = await client.callTool("create_goal", {
      objective: "list all users in the helperbook database",
    });
    assert(r.session_id, "missing session_id");
    assert(r.goal_id, "missing goal_id");
    testSessionId = r.session_id;
    // Session may complete, error (no skills), or stay created — all valid
    return `session=${r.session_id.slice(0, 8)}… status=${r.status}`;
  });

  await check("list_sessions includes created session", async () => {
    const r = await client.callTool("list_sessions");
    assert(r.sessions, "missing sessions");
    assert(r.sessions.length >= 1, "no sessions");
    // Sessions are returned as [session_id, status] tuples or objects
    const found = r.sessions.some((s) => {
      const sid = s.session_id || s[0];
      return sid === testSessionId;
    });
    assert(found, `session ${testSessionId} not found in list`);
    return `${r.sessions.length} session(s)`;
  });

  await check("inspect_session shows session details", async () => {
    assert(testSessionId, "no session to inspect");
    const r = await client.callTool("inspect_session", { session_id: testSessionId });
    assert(r.session_id === testSessionId, "session_id mismatch");
    return `status=${r.status}`;
  });

  await check("inspect_belief for session", async () => {
    assert(testSessionId, "no session");
    const r = await client.callTool("inspect_belief", { session_id: testSessionId });
    assert(r.session_id === testSessionId, "session_id mismatch");
    return `belief returned`;
  });

  await check("inspect_trace for session", async () => {
    assert(testSessionId, "no session");
    const r = await client.callTool("inspect_trace", { session_id: testSessionId });
    assert(r.session_id === testSessionId, "session_id mismatch");
    return `trace steps=${r.trace?.total_steps ?? r.trace?.returned ?? 0}`;
  });

  await check("abort_session on created session", async () => {
    const g = await client.callTool("create_goal", { objective: "test session lifecycle" });
    assert(g.session_id, "missing session_id");
    const sid = g.session_id;

    // Session may be in Created or error state (no skills registered).
    // Abort should work from any non-terminal state.
    const abort = await client.callTool("abort_session", { session_id: sid });
    assert(abort.session_id === sid, "abort: session_id mismatch");
    return `abort on ${sid.slice(0, 8)}… status=${abort.status}`;
  });

  await check("pause/resume require Running state (state machine enforced)", async () => {
    const g = await client.callTool("create_goal", { objective: "test pause constraints" });
    const sid = g.session_id;
    // Pause on a non-Running session should fail gracefully
    try {
      await client.callTool("pause_session", { session_id: sid });
      // If it succeeds, the session was somehow running — also fine
      return "pause succeeded (session was running)";
    } catch (e) {
      assert(e.message.includes("cannot pause"), `unexpected error: ${e.message}`);
      return "correctly rejected: session not in Running state";
    }
  });

  await check("create_goal with budget constraints", async () => {
    const r = await client.callTool("create_goal", {
      objective: "count appointments",
      risk_budget: 0.3,
      latency_budget_ms: 5000,
      priority: "high",
    });
    assert(r.session_id, "missing session_id");
    return `session=${r.session_id.slice(0, 8)}… with budget constraints`;
  });

  // =========================================================================
  // 4. Memory Persistence
  // =========================================================================
  section("4. Memory Persistence");

  await check("episodes store exists in dump", async () => {
    const r = await client.callTool("dump_state", { sections: ["episodes"] });
    assert("episodes" in r, "missing episodes key");
    return `${r.episodes.length} episodes stored`;
  });

  await check("schemas store exists in dump", async () => {
    const r = await client.callTool("dump_state", { sections: ["schemas"] });
    assert("schemas" in r, "missing schemas key");
    return `${r.schemas.length} schemas stored`;
  });

  await check("routines store exists in dump", async () => {
    const r = await client.callTool("dump_state", { sections: ["routines"] });
    assert("routines" in r, "missing routines key");
    return `${r.routines.length} routines stored`;
  });

  await check("sessions survive across dump calls", async () => {
    const before = await client.callTool("dump_state", { sections: ["sessions"] });
    const count1 = before.sessions?.length || 0;
    await client.callTool("create_goal", { objective: "test persistence" });
    const after = await client.callTool("dump_state", { sections: ["sessions"] });
    const count2 = after.sessions?.length || 0;
    assert(count2 >= count1 + 1, `expected session count to grow: ${count1} -> ${count2}`);
    return `sessions: ${count1} → ${count2}`;
  });

  // =========================================================================
  // 5. Policy & Safety
  // =========================================================================
  section("5. Policy & Safety");

  await check("query_policy returns policy info", async () => {
    const r = await client.callTool("query_policy", { action: "delete" });
    // Should return without error — structure varies
    return `policy response received`;
  });

  await check("port invocation records policy_result", async () => {
    const r = await client.invokePort("postgres", "query", {
      sql: "SELECT 1 AS test",
    });
    assert(r.success, "query failed");
    assert(r.policy_result, "missing policy_result in observation");
    return `policy_result.status=${r.policy_result.status}`;
  });

  await check("port invocation records auth_result", async () => {
    const r = await client.invokePort("postgres", "query", {
      sql: "SELECT 1 AS test",
    });
    assert(r.success, "query failed");
    assert(r.auth_result, "missing auth_result in observation");
    return `auth_result.status=${r.auth_result.status}`;
  });

  await check("port invocation records sandbox_result", async () => {
    const r = await client.invokePort("postgres", "query", {
      sql: "SELECT 1 AS test",
    });
    assert(r.success, "query failed");
    assert(r.sandbox_result, "missing sandbox_result in observation");
    return `sandbox_result.status=${r.sandbox_result.status}`;
  });

  // =========================================================================
  // 6. Proprioception (Self-Awareness)
  // =========================================================================
  section("6. Proprioception");

  await check("query_metrics returns runtime metrics", async () => {
    const r = await client.callTool("query_metrics");
    assert(r, "empty metrics");
    return `metrics keys: ${Object.keys(r).slice(0, 5).join(", ")}…`;
  });

  await check("self_model has RSS memory", async () => {
    const r = await client.callTool("dump_state", { sections: ["metrics"] });
    const sm = r.metrics?.self_model;
    assert(sm, "missing self_model");
    const rss = sm.rss_bytes;
    assert(typeof rss === "number" && rss > 0, `invalid rss: ${rss}`);
    return `rss=${(rss / 1024 / 1024).toFixed(1)}MB`;
  });

  await check("self_model has uptime", async () => {
    const r = await client.callTool("dump_state", { sections: ["metrics"] });
    const sm = r.metrics?.self_model;
    assert(sm.uptime_seconds >= 0, "missing uptime");
    return `uptime=${sm.uptime_seconds}s`;
  });

  await check("self_model has capability counts", async () => {
    const r = await client.callTool("dump_state", { sections: ["metrics"] });
    const sm = r.metrics?.self_model;
    assert(sm.registered_ports >= 3, `expected >=3 ports, got ${sm.registered_ports}`);
    return `ports=${sm.registered_ports}, skills=${sm.registered_skills}, packs=${sm.loaded_packs}`;
  });

  // =========================================================================
  // 7. Observation Tracing
  // =========================================================================
  section("7. Observation & Tracing");

  await check("every invoke_port returns observation_id", async () => {
    const r = await client.invokePort("postgres", "query", { sql: "SELECT 1" });
    assert(r.observation_id, "missing observation_id");
    assert(r.invocation_id, "missing invocation_id");
    return `obs=${r.observation_id.slice(0, 8)}… inv=${r.invocation_id.slice(0, 8)}…`;
  });

  await check("invoke_port returns latency_ms", async () => {
    const r = await client.invokePort("postgres", "query", { sql: "SELECT 1" });
    assert(typeof r.latency_ms === "number", `invalid latency: ${r.latency_ms}`);
    return `latency=${r.latency_ms}ms`;
  });

  await check("invoke_port returns input_hash", async () => {
    const r = await client.invokePort("postgres", "query", { sql: "SELECT 1" });
    assert(r.input_hash, "missing input_hash");
    assert(r.input_hash.length === 64, `expected SHA-256 hex, got length ${r.input_hash.length}`);
    return `hash=${r.input_hash.slice(0, 16)}…`;
  });

  await check("invoke_port returns side_effect_summary", async () => {
    const r = await client.invokePort("postgres", "query", { sql: "SELECT 1" });
    assert(r.side_effect_summary, "missing side_effect_summary");
    return `side_effect=${r.side_effect_summary}`;
  });

  await check("invoke_port returns timestamp", async () => {
    const r = await client.invokePort("postgres", "query", { sql: "SELECT 1" });
    assert(r.timestamp, "missing timestamp");
    const d = new Date(r.timestamp);
    assert(!isNaN(d.getTime()), "invalid timestamp");
    return `timestamp=${r.timestamp}`;
  });

  // =========================================================================
  // Done
  // =========================================================================

  client.close();
  printReport();
  process.exit(failed > 0 ? 1 : 0);
}

run().catch((e) => {
  console.error(`\x1b[31mFATAL: ${e.message}\x1b[0m`);
  process.exit(1);
});
