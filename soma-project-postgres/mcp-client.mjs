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

async function loadEnv() {
  const content = await readFile(envFilePath, "utf8");
  return { ...parseDotEnv(content), ...process.env };
}

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
    const request = {
      jsonrpc: "2.0",
      id,
      method,
      params,
    };

    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(`${JSON.stringify(request)}\n`);
    });
  }

  async callTool(name, argumentsObject) {
    return this.request("tools/call", {
      name,
      arguments: argumentsObject,
    });
  }

  async close() {
    if (!this.child) return;
    this.child.stdin.end();
    this.child.kill();
  }
}

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

async function invokePort(client, capabilityId, input) {
  return client.callTool("invoke_port", {
    port_id: "postgres",
    capability_id: capabilityId,
    input,
  });
}

async function run() {
  if (!existsSync(runMcpScript)) {
    throw new Error("missing scripts/run-mcp.sh");
  }

  await loadEnv();
  const { command, options } = parseArgs(process.argv.slice(2));
  const client = new StdioMcpClient(runMcpScript, []);
  await client.start();

  try {
    switch (command) {
      case "skills": {
        const result = await client.callTool("inspect_skills", {});
        pretty(result);
        break;
      }
      case "query": {
        if (!options.sql) throw new Error("missing --sql");
        const result = await invokePort(client, "query", { sql: options.sql });
        pretty(result);
        break;
      }
      case "execute": {
        if (!options.sql) throw new Error("missing --sql");
        const result = await invokePort(client, "execute", { sql: options.sql });
        pretty(result);
        break;
      }
      case "find": {
        if (!options.table || !options.id) throw new Error("missing --table and --id");
        const result = await invokePort(client, "find", { table: options.table, id: options.id });
        pretty(result);
        break;
      }
      case "find-many": {
        if (!options.table) throw new Error("missing --table");
        const input = { table: options.table };
        if (options.limit) input.limit = Number(options.limit);
        if (options.filter) input.filter = JSON.parse(options.filter);
        const result = await invokePort(client, "find_many", input);
        pretty(result);
        break;
      }
      case "count": {
        if (!options.table) throw new Error("missing --table");
        const input = { table: options.table };
        if (options.filter) input.filter = JSON.parse(options.filter);
        const result = await invokePort(client, "count", input);
        pretty(result);
        break;
      }
      case "smoke": {
        // 1. Count users
        const userCount = await invokePort(client, "count", { table: "users" });
        if (!userCount.success) throw new Error(JSON.stringify(userCount, null, 2));
        const count = userCount.structured_result?.count;
        if (typeof count !== "number" || count < 1) {
          throw new Error(`expected users, got count=${count}`);
        }

        // 2. Query users
        const users = await invokePort(client, "query", {
          sql: "SELECT id, name, role FROM users ORDER BY name LIMIT 5",
        });
        if (!users.success) throw new Error(JSON.stringify(users, null, 2));

        // 3. Find a specific user by ID (use query for UUID columns)
        const find = await invokePort(client, "query", {
          sql: "SELECT id, name, phone, role FROM users WHERE id = '00000000-0000-0000-0000-000000000001'",
        });
        if (!find.success) throw new Error(JSON.stringify(find, null, 2));

        // 4. Count appointments
        const apptCount = await invokePort(client, "count", { table: "appointments" });
        if (!apptCount.success) throw new Error(JSON.stringify(apptCount, null, 2));

        // 5. Query appointments with join
        const appointments = await invokePort(client, "query", {
          sql: "SELECT a.service, a.status, a.rate_amount, u.name AS provider FROM appointments a JOIN users u ON a.provider_id = u.id ORDER BY a.start_time LIMIT 5",
        });
        if (!appointments.success) throw new Error(JSON.stringify(appointments, null, 2));

        // 6. Aggregate average rating
        const avgRating = await invokePort(client, "aggregate", {
          table: "reviews",
          function: "AVG",
          column: "rating",
        });
        if (!avgRating.success) throw new Error(JSON.stringify(avgRating, null, 2));

        // 7. Find messages in a chat
        const messages = await invokePort(client, "find_many", {
          table: "messages",
          filter: { chat_id: "c0000000-0000-0000-0000-000000000001" },
          limit: 5,
        });
        if (!messages.success) throw new Error(JSON.stringify(messages, null, 2));

        pretty({
          user_count: userCount.structured_result,
          users: users.structured_result,
          found_user: find.structured_result,
          appointment_count: apptCount.structured_result,
          appointments: appointments.structured_result,
          avg_rating: avgRating.structured_result,
          messages: messages.structured_result,
        });
        break;
      }
      default:
        throw new Error(
          "usage: node mcp-client.mjs <skills|query|execute|find|find-many|count|smoke> [--flags]",
        );
    }
  } finally {
    await client.close();
  }
}

run().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
