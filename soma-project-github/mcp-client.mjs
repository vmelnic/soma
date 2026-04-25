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
  const positionals = [];

  for (let i = 0; i < rest.length; i += 1) {
    const part = rest[i];
    if (!part.startsWith("--")) {
      positionals.push(part);
      continue;
    }

    const key = part.slice(2);
    const next = rest[i + 1];
    if (!next || next.startsWith("--")) {
      options[key] = true;
      continue;
    }
    options[key] = next;
    i += 1;
  }

  return { command, options, positionals };
}

function required(value, message) {
  if (!value) {
    throw new Error(message);
  }
  return value;
}

function pretty(value) {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

function resolveRepo(env) {
  const owner = env.SOMA_GITHUB_OWNER || "vmelnic";
  const repo = env.SOMA_GITHUB_REPO || "soma";
  return { owner, repo };
}

function parseToolResult(response) {
  if (response && Array.isArray(response.content) && response.content.length > 0) {
    const text = response.content[0].text;
    if (typeof text === "string") {
      try {
        return JSON.parse(text);
      } catch {
        return { success: true, raw_text: text };
      }
    }
  }
  return response;
}

async function invokePort(client, portId, capabilityId, input) {
  const raw = await client.callTool("invoke_port", {
    port_id: portId,
    capability_id: capabilityId,
    input,
  });
  return parseToolResult(raw);
}

async function run() {
  if (!existsSync(runMcpScript)) {
    throw new Error("missing scripts/run-mcp.sh");
  }

  const env = await loadEnv();
  const { command, options } = parseArgs(process.argv.slice(2));
  const client = new StdioMcpClient(runMcpScript, []);
  await client.start();

  try {
    switch (command) {
      case "skills": {
        const result = await client.request("tools/list", {});
        pretty(result);
        break;
      }
      case "ports": {
        const result = await client.request("ports/list", {});
        pretty(result);
        break;
      }
      case "branches": {
        const { owner, repo } = resolveRepo(env);
        const result = await invokePort(client, "github", "repo.list_branches", {
          owner,
          repo,
          per_page: Number(options.per_page || 30),
        });
        pretty(result);
        break;
      }
      case "read-file": {
        const { owner, repo } = resolveRepo(env);
        const filePath = required(options.path, "missing --path (e.g. README.md)");
        const result = await invokePort(client, "github", "repo.read_file", {
          owner,
          repo,
          path: filePath,
          ref: options.ref || "HEAD",
        });
        pretty(result);
        break;
      }
      case "issues": {
        const { owner, repo } = resolveRepo(env);
        const result = await invokePort(client, "github", "issue.list", {
          owner,
          repo,
          state: options.state || "open",
          per_page: Number(options.per_page || 10),
        });
        pretty(result);
        break;
      }
      case "issue-get": {
        const { owner, repo } = resolveRepo(env);
        const issueNumber = required(
          options.number,
          "missing --number (issue number)"
        );
        const result = await invokePort(client, "github", "issue.get", {
          owner,
          repo,
          issue_number: Number(issueNumber),
        });
        pretty(result);
        break;
      }
      case "prs": {
        const { owner, repo } = resolveRepo(env);
        const result = await invokePort(client, "github", "pr.list", {
          owner,
          repo,
          state: options.state || "open",
          per_page: Number(options.per_page || 10),
        });
        pretty(result);
        break;
      }
      case "pr-get": {
        const { owner, repo } = resolveRepo(env);
        const pullNumber = required(
          options.number,
          "missing --number (PR number)"
        );
        const result = await invokePort(client, "github", "pr.get", {
          owner,
          repo,
          pull_number: Number(pullNumber),
        });
        pretty(result);
        break;
      }
      case "smoke": {
        const { owner, repo } = resolveRepo(env);
        console.error(`Running smoke tests against ${owner}/${repo} ...`);

        // 1. List branches — should return at least one branch
        const branches = await invokePort(client, "github", "repo.list_branches", {
          owner,
          repo,
          per_page: 10,
        });
        if (!branches.success) {
          throw new Error(`repo.list_branches failed: ${JSON.stringify(branches)}`);
        }
        const branchList = branches.structured_result ?? [];
        if (!Array.isArray(branchList) || branchList.length === 0) {
          throw new Error("repo.list_branches returned empty array");
        }
        console.error(`✓ repo.list_branches: ${branchList.length} branch(s) found`);

        // 2. Read README.md — should contain "SOMA"
        const readme = await invokePort(client, "github", "repo.read_file", {
          owner,
          repo,
          path: "README.md",
        });
        if (!readme.success) {
          throw new Error(`repo.read_file failed: ${JSON.stringify(readme)}`);
        }
        const decoded = readme.structured_result?.decoded_content || "";
        if (!decoded.includes("SOMA")) {
          throw new Error("repo.read_file README.md does not contain 'SOMA'");
        }
        console.error(`✓ repo.read_file: README.md decoded (${decoded.length} chars)`);

        // 3. List issues — should return an array
        const issues = await invokePort(client, "github", "issue.list", {
          owner,
          repo,
          state: "all",
          per_page: 5,
        });
        if (!issues.success) {
          throw new Error(`issue.list failed: ${JSON.stringify(issues)}`);
        }
        const issueList = issues.structured_result ?? [];
        if (!Array.isArray(issueList)) {
          throw new Error("issue.list did not return an array");
        }
        console.error(`✓ issue.list: ${issueList.length} issue(s) found`);

        // 4. List PRs — should return an array
        const prs = await invokePort(client, "github", "pr.list", {
          owner,
          repo,
          state: "all",
          per_page: 5,
        });
        if (!prs.success) {
          throw new Error(`pr.list failed: ${JSON.stringify(prs)}`);
        }
        const prList = prs.structured_result ?? [];
        if (!Array.isArray(prList)) {
          throw new Error("pr.list did not return an array");
        }
        console.error(`✓ pr.list: ${prList.length} PR(s) found`);

        pretty({
          branches: branchList.map((b) => b.name),
          readme_snippet: decoded.slice(0, 200),
          issues: issueList.map((i) => ({ number: i.number, title: i.title })),
          prs: prList.map((p) => ({ number: p.number, title: p.title })),
        });
        break;
      }
      default:
        throw new Error(
          "usage: node mcp-client.mjs <skills|ports|branches|read-file|issues|issue-get|prs|pr-get|smoke> [--flags]"
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
