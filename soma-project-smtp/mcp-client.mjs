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

function required(value, message) {
  if (!value) {
    throw new Error(message);
  }
  return value;
}

function pretty(value) {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

async function invokePort(client, capabilityId, input) {
  return client.callTool("invoke_port", {
    port_id: "smtp",
    capability_id: capabilityId,
    input,
  });
}

async function run() {
  if (!existsSync(runMcpScript)) {
    throw new Error("missing scripts/run-mcp.sh");
  }

  const env = await loadEnv();
  const { command, options } = parseArgs(process.argv.slice(2));
  const client = new StdioMcpClient(runMcpScript, []);
  await client.start();

  const recipient = options.to || env.SOMA_SMTP_TO;
  const attachmentPath = path.join(projectRoot, "samples", "attachment.txt");

  try {
    switch (command) {
      case "skills": {
        const result = await client.callTool("inspect_skills", {});
        pretty(result);
        break;
      }
      case "send-plain": {
        const result = await invokePort(client, "send_plain", {
          to: required(recipient, "missing recipient; set SOMA_SMTP_TO in .env or pass --to"),
          subject: options.subject || "SOMA SMTP plain text test",
          body: options.body || "Plain text email sent through soma MCP.",
        });
        pretty(result);
        break;
      }
      case "send-html": {
        const result = await invokePort(client, "send_html", {
          to: required(recipient, "missing recipient; set SOMA_SMTP_TO in .env or pass --to"),
          subject: options.subject || "SOMA SMTP HTML test",
          body:
            options.body ||
            "<html><body><h1>SOMA SMTP</h1><p>HTML email sent through soma MCP.</p></body></html>",
        });
        pretty(result);
        break;
      }
      case "send-attachment": {
        const attachmentData = await readFile(attachmentPath);
        const result = await invokePort(client, "send_attachment", {
          to: required(recipient, "missing recipient; set SOMA_SMTP_TO in .env or pass --to"),
          subject: options.subject || "SOMA SMTP attachment test",
          body: options.body || "Attachment email sent through soma MCP.",
          attachment_name: path.basename(attachmentPath),
          attachment_data: attachmentData.toString("base64"),
        });
        pretty(result);
        break;
      }
      case "smoke": {
        const to = required(recipient, "missing recipient; set SOMA_SMTP_TO in .env");
        const attachmentData = await readFile(attachmentPath);

        const sendPlain = await invokePort(client, "send_plain", {
          to,
          subject: "SOMA SMTP plain text smoke",
          body: "Plain text email sent through soma MCP smoke test.",
        });
        if (!sendPlain.success) throw new Error(JSON.stringify(sendPlain, null, 2));

        const sendHtml = await invokePort(client, "send_html", {
          to,
          subject: "SOMA SMTP HTML smoke",
          body: "<html><body><h1>SOMA SMTP</h1><p>HTML smoke test.</p></body></html>",
        });
        if (!sendHtml.success) throw new Error(JSON.stringify(sendHtml, null, 2));

        const sendAttachment = await invokePort(client, "send_attachment", {
          to,
          subject: "SOMA SMTP attachment smoke",
          body: "Attachment smoke test sent through soma MCP.",
          attachment_name: path.basename(attachmentPath),
          attachment_data: attachmentData.toString("base64"),
        });
        if (!sendAttachment.success) throw new Error(JSON.stringify(sendAttachment, null, 2));

        pretty({
          send_plain: sendPlain,
          send_html: sendHtml,
          send_attachment: sendAttachment,
        });
        break;
      }
      default:
        throw new Error(
          "usage: node mcp-client.mjs <skills|send-plain|send-html|send-attachment|smoke> [--flags]",
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
