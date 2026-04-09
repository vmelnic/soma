#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
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

function resolveBucket(cliOptions, env) {
  return cliOptions.bucket || env.SOMA_S3_DEFAULT_BUCKET;
}

async function invokePort(client, portId, capabilityId, input) {
  return client.callTool("invoke_port", {
    port_id: portId,
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

  try {
    switch (command) {
      case "skills": {
        const result = await client.callTool("inspect_skills", {});
        pretty(result);
        break;
      }
      case "put": {
        const bucket = required(
          resolveBucket(options, env),
          "missing bucket; set SOMA_S3_DEFAULT_BUCKET in .env or pass --bucket",
        );
        const key = required(options.key, "missing --key");
        const file = required(options.file, "missing --file");
        const contentType = options["content-type"] || "text/plain";
        const filePath = path.isAbsolute(file) ? file : path.join(projectRoot, file);
        const data = await readFile(filePath, "utf8");
        const result = await invokePort(client, "s3", "put_object", {
          bucket,
          key,
          data,
          content_type: contentType,
        });
        pretty(result);
        break;
      }
      case "get": {
        const bucket = required(
          resolveBucket(options, env),
          "missing bucket; set SOMA_S3_DEFAULT_BUCKET in .env or pass --bucket",
        );
        const key = required(options.key, "missing --key");
        const result = await invokePort(client, "s3", "get_object", { bucket, key });
        if (options.out && result.success) {
          const outPath = path.isAbsolute(options.out)
            ? options.out
            : path.join(projectRoot, options.out);
          await mkdir(path.dirname(outPath), { recursive: true });
          const decoded = Buffer.from(result.structured_result.data, "base64");
          await writeFile(outPath, decoded);
        }
        pretty(result);
        break;
      }
      case "list": {
        const bucket = required(
          resolveBucket(options, env),
          "missing bucket; set SOMA_S3_DEFAULT_BUCKET in .env or pass --bucket",
        );
        const prefix = options.prefix || "";
        const result = await invokePort(client, "s3", "list_objects", { bucket, prefix });
        pretty(result);
        break;
      }
      case "presign": {
        const bucket = required(
          resolveBucket(options, env),
          "missing bucket; set SOMA_S3_DEFAULT_BUCKET in .env or pass --bucket",
        );
        const key = required(options.key, "missing --key");
        const expiresSeconds = Number(options["expires-seconds"] || 300);
        const result = await invokePort(client, "s3", "presign_url", {
          bucket,
          key,
          expires_secs: expiresSeconds,
        });
        pretty(result);
        break;
      }
      case "delete": {
        const bucket = required(
          resolveBucket(options, env),
          "missing bucket; set SOMA_S3_DEFAULT_BUCKET in .env or pass --bucket",
        );
        const key = required(options.key, "missing --key");
        const result = await invokePort(client, "s3", "delete_object", { bucket, key });
        pretty(result);
        break;
      }
      case "smoke": {
        const bucket = required(
          env.SOMA_S3_DEFAULT_BUCKET,
          "missing SOMA_S3_DEFAULT_BUCKET in .env",
        );
        const key = options.key || "mcp-smoke/hello.txt";
        const prefix = options.prefix || "mcp-smoke/";
        const sampleFile = path.join(projectRoot, "samples", "hello.txt");
        const sampleText = await readFile(sampleFile, "utf8");
        const expectedBytes = await readFile(sampleFile);

        const put = await invokePort(client, "s3", "put_object", {
          bucket,
          key,
          data: sampleText,
          content_type: "text/plain",
        });
        if (!put.success) throw new Error(JSON.stringify(put, null, 2));

        const list = await invokePort(client, "s3", "list_objects", { bucket, prefix });
        if (!list.success) throw new Error(JSON.stringify(list, null, 2));
        const listedObjects = list.structured_result?.objects ?? [];
        if (!listedObjects.some((item) => item.key === key)) {
          throw new Error(`list_objects did not include ${key}`);
        }

        const get = await invokePort(client, "s3", "get_object", { bucket, key });
        if (!get.success) throw new Error(JSON.stringify(get, null, 2));
        const downloaded = Buffer.from(get.structured_result.data, "base64");
        if (!downloaded.equals(expectedBytes)) {
          throw new Error("downloaded S3 object does not match sample file");
        }

        const presign = await invokePort(client, "s3", "presign_url", {
          bucket,
          key,
          expires_secs: Number(options["expires-seconds"] || 300),
        });
        if (!presign.success) throw new Error(JSON.stringify(presign, null, 2));
        const presignResponse = await fetch(presign.structured_result.url);
        if (!presignResponse.ok) {
          throw new Error(`presigned URL returned HTTP ${presignResponse.status}`);
        }
        const presignBody = Buffer.from(await presignResponse.arrayBuffer());
        if (!presignBody.equals(expectedBytes)) {
          throw new Error("presigned URL body does not match sample file");
        }

        const del = await invokePort(client, "s3", "delete_object", { bucket, key });
        if (!del.success) throw new Error(JSON.stringify(del, null, 2));

        const listAfterDelete = await invokePort(client, "s3", "list_objects", {
          bucket,
          prefix,
        });
        if (!listAfterDelete.success) {
          throw new Error(JSON.stringify(listAfterDelete, null, 2));
        }
        const remainingObjects = listAfterDelete.structured_result?.objects ?? [];
        if (remainingObjects.some((item) => item.key === key)) {
          throw new Error(`delete_object did not remove ${key}`);
        }

        pretty({
          put,
          list,
          get,
          presign,
          delete: del,
          list_after_delete: listAfterDelete,
        });
        break;
      }
      default:
        throw new Error(
          "usage: node mcp-client.mjs <skills|put|get|list|presign|delete|smoke> [--flags]",
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
