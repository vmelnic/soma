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

// -- DB schema context for LLM -------------------------------------------------

const DB_SCHEMA = `HelperBook database (PostgreSQL). Key tables:

users (id UUID PK, phone, name, photo_url, bio, location_lat, location_lon,
       role ['client','provider','both'], subscription_plan, is_verified, slug, locale, currency, created_at)

connections (id UUID PK, requester_id FK users, recipient_id FK users,
             status ['pending','accepted','declined','blocked'], message, created_at)

chats (id UUID PK, type ['direct','group'], name, created_by FK users, created_at)
chat_members (chat_id FK chats, user_id FK users, role, joined_at, muted_until)

messages (id UUID PK, chat_id FK chats, sender_id FK users,
          type ['text','photo','video','voice','document','location','contact_card','appointment_card','service_card'],
          content, media_url, status ['sent','delivered','read'], reply_to_id, created_at)

appointments (id UUID PK, chat_id, creator_id, client_id FK users, provider_id FK users,
              service TEXT, start_time, end_time, location, rate_amount DECIMAL, rate_currency,
              rate_type ['hourly','fixed','negotiable'],
              status ['proposed','confirmed','in_progress','completed','dismissed','cancelled','no_show'],
              notes, created_at)

reviews (id UUID PK, appointment_id FK appointments, reviewer_id FK users, reviewed_id FK users,
         rating INT 1-5, feedback TEXT, tags TEXT[], created_at)

provider_profiles (user_id UUID PK FK users, bio_extended, certifications TEXT[],
                   working_schedule JSONB, service_area_radius INT, communication_languages TEXT[],
                   response_rate, avg_response_time)

services_history (id UUID PK, appointment_id FK, services TEXT[], hours, rate, total_amount,
                  confirmed_by_client, confirmed_by_provider, disputed, created_at)

notifications (id UUID PK, user_id FK users, type, title, body, data JSONB, read BOOLEAN, created_at)

contact_notes (user_id, contact_id, note_text, updated_at)`;

const SQL_SYSTEM = `You are a PostgreSQL query generator for the HelperBook service marketplace.
You receive a question from a user and the database schema. Return ONLY a single SQL SELECT query.
No explanation, no markdown fences, no comments — just the raw SQL ending with a semicolon.

${DB_SCHEMA}`;

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
  // Strip markdown fences if present despite instructions.
  let sql = text.trim();
  if (sql.startsWith("```")) {
    sql = sql.replace(/^```(?:sql)?\n?/, "").replace(/\n?```$/, "");
  }
  return sql.trim();
}

async function askQuestion(somaClient, question, role) {
  const scenario = SCENARIOS[role];
  const contextHint = scenario
    ? `The current user is ${scenario.label} (id: ${scenario.userId}).`
    : "";

  // Step 1: LLM generates SQL.
  const sqlPrompt = `${contextHint}\n\nQuestion: ${question}`;
  process.stdout.write(`\n  Question: ${question}\n`);
  process.stdout.write("  Generating SQL...");

  const rawSql = await ollamaGenerate(sqlPrompt, SQL_SYSTEM);
  const sql = extractSql(rawSql);
  process.stdout.write(` done.\n  SQL: ${sql}\n`);

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

      process.stdout.write("3. Querying DB via SOMA... ");
      const users = await somaClient.callTool("invoke_port", {
        port_id: "postgres",
        capability_id: "query",
        input: { sql: "SELECT id, name, role FROM users ORDER BY name LIMIT 3" },
      });
      if (!users.success) throw new Error(JSON.stringify(users));
      process.stdout.write("OK\n");

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
