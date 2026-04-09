#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const projectRoot = path.dirname(fileURLToPath(import.meta.url));
const runMcpScript = path.join(projectRoot, "scripts", "run-mcp.sh");
const envFilePath = path.join(projectRoot, ".env");

// -- env -----------------------------------------------------------------------

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
    ) {
      value = value.slice(1, -1);
    }
    env[key] = value;
  }
  return env;
}

let envCache;
async function loadEnv() {
  if (envCache) return envCache;
  const content = await readFile(envFilePath, "utf8");
  envCache = { ...parseDotEnv(content), ...process.env };
  return envCache;
}

// -- SOMA MCP client -----------------------------------------------------------

class StdioMcpClient {
  constructor(command, args) {
    this.command = command;
    this.args = args;
    this.nextId = 1;
    this.pending = new Map();
  }

  async start() {
    this.child = spawn(this.command, this.args, {
      cwd: projectRoot,
      stdio: ["pipe", "pipe", "pipe"],
    });

    this.child.stderr.on("data", (chunk) => {
      process.stderr.write(chunk);
    });

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
      try {
        payload = JSON.parse(line);
      } catch (error) {
        for (const { reject } of this.pending.values()) {
          reject(error);
        }
        this.pending.clear();
        return;
      }
      const pending = this.pending.get(String(payload.id));
      if (!pending) return;
      this.pending.delete(String(payload.id));
      if (payload.error) {
        pending.reject(new Error(payload.error.message));
      } else {
        pending.resolve(payload.result);
      }
    });

    await this.request("initialize", {});
  }

  request(method, params) {
    const id = String(this.nextId++);
    const request = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(`${JSON.stringify(request)}\n`);
    });
  }

  async callTool(name, args) {
    return this.request("tools/call", { name, arguments: args });
  }

  async close() {
    if (!this.child) return;
    this.child.stdin.end();
    this.child.kill();
  }
}

// -- Ollama client -------------------------------------------------------------

async function ollamaGenerate(prompt, system) {
  const env = await loadEnv();
  const host = env.OLLAMA_HOST || "http://localhost:11434";
  const model = env.OLLAMA_MODEL || "gemma4:e2b";

  const body = {
    model,
    prompt,
    system: system || "",
    stream: false,
    options: { temperature: 0.3 },
  };

  const res = await fetch(`${host}/api/generate`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Ollama ${res.status}: ${text}`);
  }

  const json = await res.json();
  return json.response;
}

async function ollamaHealthy() {
  const env = await loadEnv();
  const host = env.OLLAMA_HOST || "http://localhost:11434";
  try {
    const res = await fetch(`${host}/api/tags`);
    return res.ok;
  } catch {
    return false;
  }
}

// -- DB schema discovery (live introspection via SOMA) -------------------------

const SCHEMA_SQL = `
SELECT
  c.table_name,
  c.column_name,
  c.data_type,
  c.udt_name,
  c.is_nullable,
  c.column_default,
  tc.constraint_type,
  ccu.table_name AS fk_table
FROM information_schema.columns c
LEFT JOIN information_schema.key_column_usage kcu
  ON kcu.table_schema = c.table_schema
  AND kcu.table_name = c.table_name
  AND kcu.column_name = c.column_name
LEFT JOIN information_schema.table_constraints tc
  ON tc.constraint_name = kcu.constraint_name
  AND tc.table_schema = kcu.table_schema
  AND tc.constraint_type IN ('PRIMARY KEY', 'FOREIGN KEY')
LEFT JOIN information_schema.constraint_column_usage ccu
  ON ccu.constraint_name = kcu.constraint_name
  AND ccu.table_schema = kcu.table_schema
  AND tc.constraint_type = 'FOREIGN KEY'
WHERE c.table_schema = 'public'
  AND c.table_name NOT LIKE '\\_%'
ORDER BY c.table_name, c.ordinal_position;`;

const CHECK_SQL = `
SELECT
  tc.table_name,
  tc.constraint_name,
  cc.check_clause
FROM information_schema.table_constraints tc
JOIN information_schema.check_constraints cc
  ON cc.constraint_name = tc.constraint_name
  AND cc.constraint_schema = tc.constraint_schema
WHERE tc.table_schema = 'public'
  AND tc.constraint_type = 'CHECK'
  AND cc.check_clause NOT LIKE '%IS NOT NULL%'
ORDER BY tc.table_name;`;

let schemaCache = null;

// Columns with special types that need sample values shown to the LLM.
const SAMPLE_TYPES = new Set(["ARRAY", "jsonb", "USER-DEFINED"]);

async function discoverSchema(somaClient) {
  if (schemaCache) return schemaCache;

  const [colResult, chkResult] = await Promise.all([
    somaClient.callTool("invoke_port", {
      port_id: "postgres",
      capability_id: "query",
      input: { sql: SCHEMA_SQL },
    }),
    somaClient.callTool("invoke_port", {
      port_id: "postgres",
      capability_id: "query",
      input: { sql: CHECK_SQL },
    }),
  ]);

  if (!colResult.success) throw new Error(`Schema query failed: ${JSON.stringify(colResult)}`);

  const rows = colResult.structured_result?.rows || [];
  const checks = chkResult.success ? (chkResult.structured_result?.rows || []) : [];

  // Group check constraints by table.
  const checksByTable = {};
  for (const chk of checks) {
    const t = chk.table_name;
    if (!checksByTable[t]) checksByTable[t] = [];
    checksByTable[t].push(chk.check_clause);
  }

  // Identify columns that need sample values (arrays, jsonb).
  // Map: table → [column_name, ...]
  const sampleCols = {};
  for (const row of rows) {
    if (SAMPLE_TYPES.has(row.data_type)) {
      if (!sampleCols[row.table_name]) sampleCols[row.table_name] = [];
      sampleCols[row.table_name].push(row.column_name);
    }
  }

  // Fetch sample values only for those columns.
  const samplesByTable = {};
  const fetches = Object.entries(sampleCols).map(async ([table, cols]) => {
    const colList = cols.map((c) => `"${c}"`).join(", ");
    const res = await somaClient.callTool("invoke_port", {
      port_id: "postgres",
      capability_id: "query",
      input: { sql: `SELECT DISTINCT ${colList} FROM "${table}" LIMIT 3` },
    });
    if (res.success && res.structured_result?.rows?.length) {
      samplesByTable[table] = res.structured_result.rows;
    }
  });
  await Promise.all(fetches);

  // Group columns by table, format compactly.
  const tables = {};
  for (const row of rows) {
    const t = row.table_name;
    if (!tables[t]) tables[t] = [];
    let dtype = row.data_type;
    if (dtype === "ARRAY" && row.udt_name) {
      dtype = row.udt_name.replace(/^_/, "") + "[]";
    }
    let col = `${row.column_name} ${dtype}`;
    if (row.constraint_type === "PRIMARY KEY") col += " PK";
    if (row.constraint_type === "FOREIGN KEY" && row.fk_table) col += ` FK→${row.fk_table}`;
    if (row.is_nullable === "NO" && row.constraint_type !== "PRIMARY KEY") col += " NOT NULL";
    tables[t].push(col);
  }

  const lines = ["HelperBook database (PostgreSQL).\n"];
  for (const [table, cols] of Object.entries(tables)) {
    lines.push(`${table} (${cols.join(", ")})`);
    const tableChecks = checksByTable[table];
    if (tableChecks) {
      lines.push(`  CHECK: ${tableChecks.join("; ")}`);
    }
    if (samplesByTable[table]) {
      lines.push(`  SAMPLE VALUES: ${JSON.stringify(samplesByTable[table])}`);
    }
    lines.push("");
  }

  schemaCache = lines.join("\n");
  return schemaCache;
}

function buildSqlSystem(schema) {
  return `Output a single PostgreSQL SELECT query. Nothing else — no explanation, no markdown, no comments. Just raw SQL ending with a semicolon.

Rules:
- Always SELECT human-readable columns (name, service, rating, etc.), not just id.
- text[] columns: use 'value' = ANY(column_name). NEVER use LIKE or ILIKE on arrays.
- SAMPLE VALUES show real stored data. Use those exact values (e.g. language code 'fr' not 'French').

${schema}`;
}

const ANSWER_SYSTEM = `You are a helpful assistant for HelperBook, a service marketplace app.
Given a user's question and the query results, provide a clear, concise answer in natural language.
Keep it brief — 2-4 sentences max. Reference specific names, numbers, and dates from the data.`;

// -- Scenario definitions ------------------------------------------------------

const SCENARIOS = {
  consumer: {
    label: "Service Consumer (Alexandru P.)",
    userId: "00000000-0000-0000-0000-000000000001",
    questions: [
      "What are my upcoming appointments this week?",
      "Show me all providers I'm connected with and their services.",
      "What's the average rating of providers I've worked with?",
      "Do I have any unread messages?",
      "How much have I spent on services so far?",
      "Which provider has the highest rating among my connections?",
      "Show me my appointment history with status and cost.",
    ],
  },
  provider: {
    label: "Service Provider (Ana M. — Hair Stylist)",
    userId: "00000000-0000-0000-0000-000000000002",
    questions: [
      "How many clients do I have this month?",
      "What is my average rating from reviews?",
      "Show me all my upcoming confirmed appointments.",
      "Who are the clients that have booked with me?",
      "What's my total earnings from completed appointments?",
      "Do I have any pending connection requests?",
      "Show me the feedback from my latest reviews.",
    ],
  },
};

// -- Core loop: question → SQL → SOMA → answer ---------------------------------

function extractSql(text) {
  let sql = text.trim();
  // Strip markdown fences if present.
  if (sql.startsWith("```")) {
    sql = sql.replace(/^```(?:sql)?\n?/, "").replace(/\n?```$/, "");
  }
  // If the LLM buried a SELECT inside prose, extract it.
  if (!/^\s*SELECT/i.test(sql)) {
    const match = sql.match(/SELECT[\s\S]+?;/i);
    if (match) return match[0].trim();
  }
  return sql.trim();
}

function looksLikeSql(text) {
  return /^\s*SELECT\s/i.test(text);
}

async function askQuestion(somaClient, question, role) {
  const scenario = SCENARIOS[role];
  const contextHint = scenario
    ? `The current user is ${scenario.label} (id: ${scenario.userId}).`
    : "";

  // Step 1: Discover schema from live DB, then ask LLM to generate SQL.
  const schema = await discoverSchema(somaClient);
  const sqlSystem = buildSqlSystem(schema);
  const sqlPrompt = `${contextHint}\n\nQuestion: ${question}`;
  process.stdout.write(`\n  Question: ${question}\n`);
  process.stdout.write("  Generating SQL...");

  let sql = extractSql(await ollamaGenerate(sqlPrompt, sqlSystem));

  // If the LLM returned prose instead of SQL, retry once with a stricter nudge.
  if (!looksLikeSql(sql)) {
    process.stdout.write(" (retrying)...");
    const retry = await ollamaGenerate(
      `${sqlPrompt}\n\nYou MUST reply with ONLY a SQL SELECT query. No text.`,
      sqlSystem,
    );
    sql = extractSql(retry);
  }

  process.stdout.write(` done.\n  SQL: ${sql}\n`);

  if (!looksLikeSql(sql)) {
    process.stdout.write("  Skipped — model did not produce valid SQL.\n");
    return;
  }

  // Step 2: Execute via SOMA postgres port.
  process.stdout.write("  Executing via SOMA...");
  const result = await somaClient.callTool("invoke_port", {
    port_id: "postgres",
    capability_id: "query",
    input: { sql },
  });
  process.stdout.write(" done.\n");

  if (!result.success) {
    process.stdout.write(`  Error: ${JSON.stringify(result)}\n`);
    return;
  }

  const rows = result.structured_result?.rows || [];
  const rowCount = result.structured_result?.row_count ?? rows.length;

  // Step 3: LLM interprets results.
  process.stdout.write("  Interpreting results...");
  const answerPrompt = [
    `Question: ${question}`,
    `Query returned ${rowCount} row(s):`,
    JSON.stringify(rows.slice(0, 20), null, 2),
  ].join("\n");

  const answer = await ollamaGenerate(answerPrompt, ANSWER_SYSTEM);
  process.stdout.write(" done.\n");
  process.stdout.write(`  Answer: ${answer.trim()}\n`);
}

// -- Commands ------------------------------------------------------------------

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let i = 0; i < rest.length; i += 1) {
    const part = rest[i];
    if (!part.startsWith("--")) continue;
    const key = part.slice(2);
    const next = rest[i + 1];
    if (!next || next.startsWith("--")) {
      options[key] = true;
      continue;
    }
    options[key] = next;
    i += 1;
  }
  return { command, options };
}

function pretty(value) {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

async function runScenario(somaClient, role) {
  const scenario = SCENARIOS[role];
  if (!scenario) throw new Error(`Unknown role: ${role}. Use: consumer, provider`);

  process.stdout.write(`\n=== ${scenario.label} ===\n`);

  for (const question of scenario.questions) {
    try {
      await askQuestion(somaClient, question, role);
    } catch (err) {
      process.stdout.write(`  Error: ${err.message}\n`);
    }
  }

  process.stdout.write(`\n=== Done (${scenario.questions.length} questions) ===\n`);
}

async function run() {
  if (!existsSync(runMcpScript)) {
    throw new Error("missing scripts/run-mcp.sh");
  }

  await loadEnv();
  const { command, options } = parseArgs(process.argv.slice(2));
  const somaClient = new StdioMcpClient(runMcpScript, []);

  switch (command) {
    case "smoke": {
      // Verify all three pieces: Ollama, SOMA, and DB.
      process.stdout.write("1. Checking Ollama... ");
      if (!(await ollamaHealthy())) throw new Error("Ollama not reachable");
      process.stdout.write("OK\n");

      process.stdout.write("2. Starting SOMA MCP... ");
      await somaClient.start();
      process.stdout.write("OK\n");

      process.stdout.write("3. Discovering DB schema via SOMA... ");
      const schema = await discoverSchema(somaClient);
      const tableCount = (schema.match(/^[a-z_]+ \(/gm) || []).length;
      process.stdout.write(`${tableCount} tables found\n`);

      process.stdout.write("4. Ollama generate test... ");
      const reply = await ollamaGenerate("Say OK in one word.", "You are a test bot. Reply with one word only.");
      process.stdout.write(`${reply.trim()}\n`);

      process.stdout.write("5. End-to-end: question → SQL → SOMA → answer... ");
      await askQuestion(somaClient, "How many users are in the system?", "consumer");
      process.stdout.write("Smoke test passed.\n");

      await somaClient.close();
      break;
    }

    case "consumer":
    case "provider": {
      await somaClient.start();
      await runScenario(somaClient, command);
      await somaClient.close();
      break;
    }

    case "ask": {
      if (!options.question) throw new Error("missing --question");
      const role = options.role || "consumer";
      await somaClient.start();
      await askQuestion(somaClient, options.question, role);
      await somaClient.close();
      break;
    }

    case "both": {
      await somaClient.start();
      await runScenario(somaClient, "consumer");
      await runScenario(somaClient, "provider");
      await somaClient.close();
      break;
    }

    default:
      throw new Error(
        "usage: node ollama.js <smoke|consumer|provider|both|ask> [--question '...'] [--role consumer|provider]"
      );
  }
}

run().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
