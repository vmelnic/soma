// Thin MCP stdio client against a long-lived soma-next subprocess.
//
// We spawn `soma --mcp --pack auto` per backend process on startup,
// keep it alive for the life of the server, and send tools/call
// messages for every `invoke_port`. Auto mode discovers all port
// dylibs from SOMA_PORTS_PLUGIN_PATH without needing a manifest.
// Line-delimited JSON-RPC 2.0 — same wire format the phase 1g
// brain-proxy uses, same format soma-project-postgres/mcp-client.mjs
// uses against the postgres pack.
//
// We do not ship a pool / restart-on-crash here — if soma-next dies
// the whole backend should surface that loudly, not paper over it.
// Commits 2+ can add heartbeats + auto-restart once the happy path
// is trusted.

import { spawn } from "child_process";
import readline from "readline";
import { dirname, resolve as resolvePath } from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const PROJECT_ROOT = resolvePath(__dirname, "..");

export class SomaMcpClient {
  constructor(opts = {}) {
    this.projectRoot = opts.projectRoot || PROJECT_ROOT;
    this.binPath = opts.binPath || resolvePath(this.projectRoot, "bin", "soma");
    this.binDir = dirname(this.binPath);
    this.nextId = 1;
    this.pending = new Map();
    this.child = null;
    this.ready = false;
  }

  async start() {
    this.child = spawn(this.binPath, ["--mcp", "--pack", "auto"], {
      cwd: this.projectRoot,
      env: {
        ...process.env,
        // Plugin search path — soma-next auto-discovers all
        // libsoma_port_*.dylib / .so in these directories.
        SOMA_PORTS_PLUGIN_PATH: this.binDir,
        SOMA_PORTS_REQUIRE_SIGNATURES: "false",
      },
      stdio: ["pipe", "pipe", "pipe"],
    });

    // Forward soma-next's stderr to our own so pack load errors
    // surface immediately in the backend log.
    this.child.stderr.on("data", (chunk) => {
      process.stderr.write(`[soma-mcp] ${chunk}`);
    });

    this.child.on("exit", (code, signal) => {
      this.ready = false;
      const reason = `soma-next exited code=${code} signal=${signal ?? ""}`;
      console.error(`[soma-mcp] ${reason}`);
      for (const { reject } of this.pending.values()) {
        reject(new Error(reason));
      }
      this.pending.clear();
    });

    // Line-delimited JSON-RPC responses come in over stdout.
    const rl = readline.createInterface({ input: this.child.stdout });
    rl.on("line", (line) => {
      if (!line.trim()) return;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch (e) {
        console.warn("[soma-mcp] ignoring non-JSON stdout line:", line.slice(0, 80));
        return;
      }
      const id = String(msg.id);
      const pending = this.pending.get(id);
      if (!pending) return;
      this.pending.delete(id);
      if (msg.error) {
        pending.reject(
          new Error(msg.error.message || `soma MCP error code ${msg.error.code}`),
        );
      } else {
        pending.resolve(msg.result);
      }
    });

    // MCP spec: client must send `initialize` before any tools/call.
    await this.request("initialize", {});
    this.ready = true;

    // Cache the port catalog once, right after init. The chat
    // brain's system prompt embeds a compact summary of this so
    // it doesn't need to call list_ports every turn just to know
    // what's available. Ports don't change between turns for the
    // life of the backend — they're baked into packs/platform at
    // startup — so a one-shot snapshot is correct.
    try {
      const raw = await this.callTool("list_ports", {});
      this.portCatalog = this.unwrap(raw);
    } catch (err) {
      console.warn(
        `[soma-mcp] list_ports at startup failed: ${err.message}`,
      );
      this.portCatalog = null;
    }

    // Build a short-name → full port_id map so callers can use simple
    // names ("postgres", "smtp", "crypto") regardless of the port's
    // internal namespaced ID ("soma.ports.postgres", "soma.smtp").
    // For each port, register: exact port_id, last dotted segment,
    // and name (lowercased). First match wins on collisions.
    this.portAliases = new Map();
    if (this.portCatalog) {
      const ports = Array.isArray(this.portCatalog)
        ? this.portCatalog
        : this.portCatalog?.ports || [];
      for (const p of ports) {
        const id = p.port_id;
        if (!id) continue;
        // Exact match always wins.
        if (!this.portAliases.has(id)) this.portAliases.set(id, id);
        // Short name: last segment after the last dot.
        const short = id.includes(".") ? id.split(".").pop() : null;
        if (short && !this.portAliases.has(short)) {
          this.portAliases.set(short, id);
        }
        // Name (lowercased) as fallback alias.
        const name = (p.name || "").toLowerCase();
        if (name && !this.portAliases.has(name)) {
          this.portAliases.set(name, id);
        }
      }
    }

    console.log("[soma-mcp] soma-next MCP server ready");
  }

  // Return a compact text catalog of the loaded ports + their
  // capabilities, suitable for embedding in a system prompt.
  //
  // For each capability we surface:
  //   - capability_id
  //   - a risk tag (read / write / destructive) derived from
  //     effect_class + risk_class, so the brain can tell which
  //     calls are safe to issue freely and which need operator
  //     confirmation
  //   - the `purpose` string from the port metadata — this is the
  //     one-line human description the port author wrote, and
  //     it's the closest thing to real documentation the brain
  //     gets (input_schema from list_ports is generic
  //     {type: "object"} and carries no field info)
  //
  // Tries to handle varied catalog shapes: {ports: [...]} or a
  // bare array, capability entries as strings or objects, etc.
  getPortCatalogSummary() {
    const cat = this.portCatalog;
    if (!cat) return "(port catalog unavailable)";

    const portArray = Array.isArray(cat)
      ? cat
      : Array.isArray(cat?.ports)
        ? cat.ports
        : null;
    if (!portArray) {
      return JSON.stringify(cat).slice(0, 3000);
    }

    const lines = [];
    for (const p of portArray) {
      if (!p || typeof p !== "object") continue;
      const id = p.port_id || p.id || p.name || "(unknown)";
      const desc = p.description ? ` — ${p.description}` : "";
      lines.push(`${id}${desc}`);

      const caps = Array.isArray(p.capabilities)
        ? p.capabilities
        : Array.isArray(p.skills)
          ? p.skills
          : [];

      // First pass: compute the max capability id width so the
      // rendered table aligns visually (helps the model parse).
      let maxId = 0;
      for (const c of caps) {
        const capId =
          typeof c === "string"
            ? c
            : c?.capability_id || c?.id || c?.name || null;
        if (typeof capId === "string" && capId.length > maxId) {
          maxId = capId.length;
        }
      }

      for (const c of caps) {
        if (typeof c === "string") {
          lines.push(`  ${c.padEnd(maxId)}`);
          continue;
        }
        const capId = c?.capability_id || c?.id || c?.name;
        if (!capId) continue;

        // Map (effect_class, risk_class) → a short tag the model
        // can reason about at a glance.
        const eff = String(c?.effect_class || "").toLowerCase();
        const risk = String(c?.risk_class || "").toLowerCase();
        let tag = "read";
        if (eff === "destructive" || risk === "critical") {
          tag = "destructive";
        } else if (eff.includes("mutation") || eff.includes("external")) {
          tag = "write";
        } else if (eff === "readonly" || eff === "none") {
          tag = "read";
        }

        const purpose = c?.purpose ? ` — ${c.purpose}` : "";
        lines.push(
          `  ${capId.padEnd(maxId)}  [${tag}]${purpose}`,
        );
      }
      lines.push("");
    }
    return lines.length > 0
      ? lines.join("\n").trimEnd()
      : "(no ports loaded)";
  }

  async getRoutineSummary() {
    try {
      const raw = await this.callTool("dump_state", { sections: ["routines"] });
      const data = this.unwrap(raw);
      const routines = data?.routines || [];
      if (routines.length === 0) return "(no routines compiled yet)";
      return routines
        .map((r) => {
          const steps = Array.isArray(r.compiled_skill_path)
            ? r.compiled_skill_path.join(" \u2192 ")
            : "?";
          const conf =
            typeof r.confidence === "number" ? r.confidence.toFixed(2) : "?";
          return `${r.routine_id} (${steps}, confidence: ${conf})`;
        })
        .join("\n");
    } catch {
      return "(routine fetch failed)";
    }
  }

  request(method, params) {
    if (!this.child || this.child.exitCode !== null) {
      return Promise.reject(new Error("soma-next child not running"));
    }
    const id = String(this.nextId++);
    const msg = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(`${JSON.stringify(msg)}\n`);
    });
  }

  async callTool(name, args) {
    return this.request("tools/call", { name, arguments: args });
  }

  // soma-next wraps tools/call results in
  //   { content: [{ type: "text", text: "<JSON>" }] }
  // so clients going through an MCP client library get strings. For
  // invoke_port, the `text` is always a JSON-serialized PortCallRecord.
  unwrap(result) {
    if (
      result &&
      Array.isArray(result.content) &&
      result.content[0]?.type === "text"
    ) {
      const text = result.content[0].text;
      try {
        return JSON.parse(text);
      } catch {
        return text;
      }
    }
    return result;
  }

  // Invoke a port capability. Throws on transport errors, on
  // soma-next-side errors, and on PortCallRecord.success === false
  // with the adapter's failure detail attached.
  //
  // On success returns the parsed `structured_result` (the port's
  // payload), not the full PortCallRecord — callers almost always
  // want the data, not the tracing envelope.
  // Resolve a short port name ("postgres") to the full port_id
  // ("soma.ports.postgres") as reported by the runtime.
  resolvePortId(shortName) {
    return this.portAliases?.get(shortName) ?? shortName;
  }

  async invokePort(portId, capabilityId, input) {
    portId = this.resolvePortId(portId);
    if (process.env.SOMA_MCP_DEBUG === "1") {
      const sql = input?.sql ? ` sql=${input.sql.slice(0, 100)}` : "";
      const params = input?.params
        ? ` params=${JSON.stringify(input.params).slice(0, 200)}`
        : "";
      process.stderr.write(
        `[soma-mcp] → ${portId}.${capabilityId}${sql}${params}\n`,
      );
    }
    const raw = await this.callTool("invoke_port", {
      port_id: portId,
      capability_id: capabilityId,
      input,
    });
    const record = this.unwrap(raw);
    if (!record || typeof record !== "object") {
      throw new Error(
        `invokePort ${portId}.${capabilityId}: unexpected response shape`,
      );
    }
    if (record.success === false) {
      const detail =
        record.structured_result?.error ??
        record.failure_class ??
        "unknown failure";
      const err = new Error(`${portId}.${capabilityId} failed: ${detail}`);
      err.record = record;
      throw err;
    }
    return record.structured_result ?? {};
  }

  async close() {
    if (!this.child) return;
    try {
      this.child.stdin.end();
    } catch {}
    this.child.kill("SIGTERM");
    await new Promise((resolve) => {
      if (this.child.exitCode !== null) return resolve();
      this.child.once("exit", resolve);
      setTimeout(() => {
        try {
          this.child.kill("SIGKILL");
        } catch {}
        resolve();
      }, 3000).unref();
    });
  }
}
