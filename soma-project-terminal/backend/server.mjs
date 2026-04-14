// soma-project-terminal — Node HTTP gateway.
//
// Conversation-first architecture:
//
//   Frontend:       streaming-ish chat UI. One <div> for transcript,
//                   one <input> for composition, one mic button for
//                   voice input. No per-pack UI, no skills grid, no
//                   runtime panel, no view DSL.
//
//   Backend:        this file. Routes the operator's chat turns to
//                   the chat brain with OpenAI tool calling bound to
//                   soma-next's MCP catalog. The brain handles every
//                   operator request by calling the right port via
//                   MCP tools — no pack generation, no templates, no
//                   LLM-produced artifacts we maintain.
//
//   SOMA:           the one master pack
//                   (packs/platform/manifest.json) the backend loads
//                   into soma-next as its child process. Every
//                   context shares this pack; the pack grows when WE
//                   (humans) add new ports, not when the LLM
//                   generates anything.
//
// Routes:
//   POST /api/auth/request-link
//   GET  /api/auth/verify
//   GET  /api/me
//   POST /api/auth/logout
//
//   GET    /api/contexts            — list operator's contexts
//   POST   /api/contexts            — create a new context
//   GET    /api/contexts/:id        — load one
//   DELETE /api/contexts/:id        — delete one
//
//   GET    /api/contexts/:id/messages — read the transcript
//   POST   /api/contexts/:id/messages — append a user turn, run the
//                                       tool-calling chat brain loop,
//                                       append the assistant reply
//
//   POST /api/transcribe            — Whisper voice input
//   GET  /api/health
//
//   PUT    /api/webhooks/:name/instruction — set brain instruction
//   DELETE /api/webhooks/:name/instruction — remove brain instruction
//   GET    /api/webhooks/:name/instruction — get current instruction
//   POST   /api/webhooks/:name            — receive webhook, optionally
//                                            route through brain
//
// Zero runtime node_modules. All side effects route through
// SomaMcpClient.invokePort (postgres, smtp, crypto). OpenAI is the
// one network hop that doesn't go through SOMA — for now. Future
// commit: soma-ports/llm dylib (see docs/terminal-multi-tenancy.md).

import http from "node:http";
import { readFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { extname, normalize, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";

import { SomaMcpClient } from "./soma-mcp.mjs";
import { createAuth } from "./auth.mjs";
import { createContexts } from "./contexts.mjs";
import { createMessages } from "./messages.mjs";
import {
  runChatTurn,
  transcribeAudio,
  buildSystemPrompt,
} from "./brain.mjs";
import { DEFAULT_CHAT_TOOLS, makeInvokeTool } from "./mcp-tools.mjs";
import { hasTable, extractTableNames, introspectTable, formatSchemaCache, loadFromPostgres } from "./schema-cache.mjs";
import {
  secretsEnabled, ensureSecretsTable, storeSecret,
  getSecret, deleteSecret, listSecrets,
} from "./secrets.mjs";
import { contextNamespace } from "./brain.mjs";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const FRONTEND_DIR = resolvePath(__dirname, "..", "frontend");
const BIN_DIR = resolvePath(__dirname, "..", "bin");
const PORT = Number(process.env.BACKEND_PORT ?? 8765);

// Per-webhook-name brain instructions. Set via PUT /api/webhooks/:name/instruction.
// When set, incoming webhooks are routed through the brain for interpretation.
const webhookInstructions = new Map();

// ------------------------------------------------------------------
// startup preflight — verify the native binaries exist. wasm assets
// no longer exist (deleted in this commit).
// ------------------------------------------------------------------

function preflight() {
  const nativeRequired = [
    { path: resolvePath(BIN_DIR, "soma"), name: "soma-next binary" },
    {
      path: resolvePath(BIN_DIR, "libsoma_port_crypto.dylib"),
      name: "crypto port dylib",
    },
    {
      path: resolvePath(BIN_DIR, "libsoma_port_postgres.dylib"),
      name: "postgres port dylib",
    },
    {
      path: resolvePath(BIN_DIR, "libsoma_port_smtp.dylib"),
      name: "smtp port dylib",
    },
  ];
  const missing = nativeRequired.filter((r) => !existsSync(r.path));
  if (missing.length === 0) return;

  process.stderr.write(
    "error: missing required binaries:\n" +
      missing.map((m) => `  - ${m.name}: ${m.path}`).join("\n") +
      "\n\n" +
      "Run ./scripts/copy-binaries.sh to populate bin/ from soma-next\n" +
      "and soma-ports (both must be built in release mode first).\n",
  );
  process.exit(1);
}

// ------------------------------------------------------------------
// helpers
// ------------------------------------------------------------------

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};

function send(res, status, body, headers = {}) {
  const isString = typeof body === "string";
  const payload = isString ? Buffer.from(body, "utf8") : body;
  res.writeHead(status, {
    "Content-Length": payload.length,
    ...headers,
  });
  res.end(payload);
}

function sendJson(res, status, obj, headers = {}) {
  send(res, status, JSON.stringify(obj), {
    "Content-Type": "application/json; charset=utf-8",
    ...headers,
  });
}

function sendRedirect(res, location) {
  res.writeHead(302, { Location: location });
  res.end();
}

async function readBody(req, max = 64 * 1024) {
  const chunks = [];
  let total = 0;
  for await (const chunk of req) {
    total += chunk.length;
    if (total > max) {
      const err = new Error("request body too large");
      err.statusCode = 413;
      throw err;
    }
    chunks.push(chunk);
  }
  const raw = Buffer.concat(chunks).toString("utf8");
  if (raw === "") return {};
  try {
    return JSON.parse(raw);
  } catch {
    const err = new Error("invalid JSON body");
    err.statusCode = 400;
    throw err;
  }
}

// Raw-body reader for binary uploads (Whisper audio). Returns a
// Buffer, not a parsed JSON object, with a 10 MB cap.
async function readRawBody(req, max = 10 * 1024 * 1024) {
  const chunks = [];
  let total = 0;
  for await (const chunk of req) {
    total += chunk.length;
    if (total > max) {
      const err = new Error("request body too large");
      err.statusCode = 413;
      throw err;
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function readCookie(req, name) {
  const header = req.headers.cookie;
  if (!header) return null;
  for (const part of header.split(";")) {
    const [k, ...rest] = part.trim().split("=");
    if (k === name) return decodeURIComponent(rest.join("="));
  }
  return null;
}

function sessionCookieHeader(token, expiresAt) {
  const secure = (process.env.PUBLIC_BASE_URL || "").startsWith("https://")
    ? "Secure; "
    : "";
  return (
    `soma_session=${encodeURIComponent(token)}; ` +
    `Path=/; ` +
    `HttpOnly; ` +
    `${secure}` +
    `SameSite=Lax; ` +
    `Expires=${new Date(expiresAt).toUTCString()}`
  );
}

function clearSessionCookie() {
  return "soma_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0";
}

function getSessionToken(req) {
  // Authorization header wins over the cookie — an explicit bearer
  // token is the caller saying "use THIS session, not whatever is
  // in the jar." Cookie-first would let a stale browser session
  // silently override an intentional API call, which breaks
  // multi-tenant scope isolation tests in particular.
  const h = req.headers.authorization;
  if (h && h.toLowerCase().startsWith("bearer ")) {
    return h.slice(7).trim();
  }
  const cookie = readCookie(req, "soma_session");
  if (cookie) return cookie;
  return null;
}

// ------------------------------------------------------------------
// static file serving
// ------------------------------------------------------------------

async function serveStatic(req, res, urlPath) {
  let relPath = urlPath === "/" ? "/index.html" : urlPath;
  const q = relPath.indexOf("?");
  if (q !== -1) relPath = relPath.slice(0, q);
  const joined = resolvePath(FRONTEND_DIR, "." + normalize(relPath));
  if (!joined.startsWith(FRONTEND_DIR)) {
    return send(res, 403, "forbidden");
  }
  try {
    const s = await stat(joined);
    if (!s.isFile()) return send(res, 404, "not found");
    const ext = extname(joined).toLowerCase();
    const body = await readFile(joined);
    return send(res, 200, body, {
      "Content-Type": MIME[ext] || "application/octet-stream",
      "Cache-Control": "no-cache",
    });
  } catch (e) {
    if (e.code === "ENOENT") return send(res, 404, "not found");
    throw e;
  }
}

// ------------------------------------------------------------------
// main
// ------------------------------------------------------------------

async function main() {
  preflight();

  const soma = new SomaMcpClient();
  await soma.start();

  // Ensure collaborator table exists.
  try {
    await soma.invokePort("postgres", "execute", {
      sql: `CREATE TABLE IF NOT EXISTS context_collaborators (
        context_id UUID NOT NULL,
        user_id UUID NOT NULL,
        added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        PRIMARY KEY (context_id, user_id)
      )`,
    });
  } catch {}

  const auth = createAuth(soma);
  const contexts = createContexts(soma);
  const messages = createMessages(soma);

  // Cache routine summary at startup (ports + routines are static
  // for the life of the backend).
  const cachedRoutineSummary = await soma.getRoutineSummary();

  // The chat brain's tool bindings — a closure over the live
  // SomaMcpClient so every tool call hits the same child process.
  const invokeTool = makeInvokeTool(soma);

  async function requireUser(req, res) {
    const token = getSessionToken(req);
    const user = await auth.currentUser(token);
    if (!user) {
      sendJson(res, 401, { status: "unauthenticated" });
      return null;
    }
    return user;
  }

  const handle = async (req, res) => {
    const url = new URL(req.url, `http://${req.headers.host}`);
    const path = url.pathname;
    const method = req.method;

    try {
      // ---- auth ----
      if (method === "POST" && path === "/api/auth/request-link") {
        const body = await readBody(req);
        const email = String(body.email ?? "").trim();
        const result = await auth.requestMagicLink(email);
        if (result.ok) {
          return sendJson(res, 200, { status: "dispatched" });
        }
        return sendJson(res, 400, { status: "error", error: result.error });
      }

      if (method === "GET" && path === "/api/auth/verify") {
        const token = url.searchParams.get("token");
        const ua = req.headers["user-agent"] ?? null;
        const result = await auth.verifyMagicToken(token, ua);
        if (!result.ok) {
          return sendJson(res, 401, { status: "error", error: result.error });
        }
        res.setHeader(
          "Set-Cookie",
          sessionCookieHeader(result.session_token, result.expires_at),
        );
        const accept = req.headers.accept ?? "";
        if (accept.includes("application/json")) {
          return sendJson(res, 200, {
            status: "ok",
            session_token: result.session_token,
            user: result.user,
            expires_at: result.expires_at,
          });
        }
        return sendRedirect(res, "/");
      }

      if (method === "GET" && path === "/api/me") {
        const token = getSessionToken(req);
        const user = await auth.currentUser(token);
        if (!user) {
          return sendJson(res, 401, { status: "unauthenticated" });
        }
        return sendJson(res, 200, { status: "ok", user });
      }

      if (method === "POST" && path === "/api/auth/logout") {
        const token = getSessionToken(req);
        await auth.logout(token);
        res.setHeader("Set-Cookie", clearSessionCookie());
        return sendJson(res, 200, { status: "ok" });
      }

      // ---- contexts ----
      if (method === "GET" && path === "/api/contexts") {
        const user = await requireUser(req, res);
        if (!user) return;
        const result = await contexts.listForUser(user.id);
        if (!result.ok) {
          return sendJson(res, 400, {
            status: "error",
            error: result.error,
          });
        }
        return sendJson(res, 200, {
          status: "ok",
          contexts: result.contexts,
        });
      }

      if (method === "POST" && path === "/api/contexts") {
        const user = await requireUser(req, res);
        if (!user) return;
        const body = await readBody(req);
        const result = await contexts.createContext(user.id, body);
        if (!result.ok) {
          return sendJson(res, 400, {
            status: "error",
            error: result.error,
          });
        }
        return sendJson(res, 201, {
          status: "ok",
          context: result.context,
        });
      }

      // /api/contexts/:id/messages — transcript + chat turn orchestrator
      const msgMatch = path.match(
        /^\/api\/contexts\/([^/]+)\/messages$/,
      );
      if (msgMatch && (method === "GET" || method === "POST")) {
        const contextId = msgMatch[1];
        const user = await requireUser(req, res);
        if (!user) return;

        if (method === "GET") {
          const result = await messages.listForContext(user.id, contextId);
          if (!result.ok) {
            return sendJson(res, 404, {
              status: "error",
              error: result.error,
            });
          }
          return sendJson(res, 200, {
            status: "ok",
            messages: result.messages,
          });
        }

        // POST — body: { content: "..." }
        const body = await readBody(req);
        const content = String(body?.content ?? "");

        // Append the user turn first so a brain failure still
        // leaves the operator's message on the transcript.
        const userAppend = await messages.append(
          user.id,
          contextId,
          "user",
          content,
        );
        if (!userAppend.ok) {
          const code = userAppend.error === "not found" ? 404 : 400;
          return sendJson(res, code, {
            status: "error",
            error: userAppend.error,
          });
        }

        // Load the context for its system-prompt fields + full
        // transcript (including the turn we just appended).
        const ctx = await contexts.loadContext(user.id, contextId);
        if (!ctx.ok) {
          return sendJson(res, 404, {
            status: "error",
            error: "context vanished mid-request",
          });
        }
        const hist = await messages.historyFor(user.id, contextId);
        if (!hist.ok) {
          return sendJson(res, 500, {
            status: "error",
            error: hist.error,
          });
        }

        // Run the tool-calling chat turn. The brain may execute
        // multiple tool calls against soma-next before it emits a
        // final text reply. Tool results are NOT stored in the
        // transcript — only the user turn and the assistant's
        // final content land in `messages`. Tool-call traces are
        // returned in the response for debugging and can be
        // surfaced in a future commit (e.g. a "show tools used"
        // toggle on each message).
        // Lazy load persisted schema cache for this context
        if (formatSchemaCache(contextId) === "(no tables discovered yet)") {
          await loadFromPostgres(soma, contextId);
        }

        // Build runtime briefing — compact state for the brain's
        // "working memory". This replaces deep conversation history
        // with structured pointers into the runtime's actual state.
        const briefingParts = [];

        // Active schedules
        try {
          const schedRaw = await soma.callTool("list_schedules", {});
          const scheds = soma.unwrap(schedRaw);
          const schedList = scheds?.schedules || [];
          if (schedList.length > 0) {
            const labels = schedList.map(s => {
              const type = s.interval_ms ? `every ${s.interval_ms}ms` : "one-shot";
              return `${s.label} (${type})`;
            }).join(", ");
            briefingParts.push(`Active schedules (${schedList.length}): ${labels}`);
          }
        } catch {}

        // Recent webhooks from world state — the brain can reference
        // "that lead" or "the payment that just came in".
        try {
          const wsRaw = await soma.callTool("dump_world_state", {});
          const wsData = soma.unwrap(wsRaw);
          const webhookFacts = (wsData?.facts || []).filter(f => f.subject === "webhook");
          if (webhookFacts.length > 0) {
            const lines = webhookFacts.slice(-5).map(f => {
              const val = typeof f.value === "object" ? JSON.stringify(f.value) : String(f.value);
              return `[${f.predicate}] ${val}`;
            });
            briefingParts.push(`Recent webhooks:\n${lines.join("\n")}`);
          }
        } catch {}

        const runtimeBriefing = briefingParts.length > 0
          ? briefingParts.join("\n")
          : "(no background activity)";

        // Fetch world state for the brain's system prompt.
        let worldStateSummary = "(no world state facts)";
        try {
          const wsRaw = await soma.callTool("dump_world_state", {});
          const ws = soma.unwrap(wsRaw);
          if (ws?.facts?.length > 0) {
            worldStateSummary = ws.facts.map(f => `${f.subject}.${f.predicate} = ${JSON.stringify(f.value)} (confidence: ${f.confidence})`).join("\n");
          }
        } catch {}

        let reply;
        try {
          reply = await runChatTurn({
            systemPrompt: buildSystemPrompt(
              ctx.context,
              soma.getPortCatalogSummary(),
              formatSchemaCache(contextId),
              cachedRoutineSummary,
              runtimeBriefing,
              worldStateSummary,
            ),
            history: hist.history,
            tools: DEFAULT_CHAT_TOOLS,
            invokeTool,
          });
        } catch (e) {
          console.error(`[brain] runChatTurn failed:`, e.message);
          return sendJson(res, 502, {
            status: "error",
            error: `brain failed: ${e.message}`,
            user_message: userAppend.message,
          });
        }

        const assistantContent =
          reply?.content && reply.content.trim() !== ""
            ? reply.content
            : "(the brain returned no content for this turn)";
        const assistantAppend = await messages.append(
          user.id,
          contextId,
          "assistant",
          assistantContent,
        );
        if (!assistantAppend.ok) {
          return sendJson(res, 500, {
            status: "error",
            error: assistantAppend.error,
            user_message: userAppend.message,
          });
        }
        // Lazy schema introspection: inspect tool calls for SQL that
        // touched tables we haven't cached yet. Fire-and-forget — the
        // schema will be available in the system prompt for the NEXT turn.
        if (reply.tool_calls?.length > 0) {
          for (const tc of reply.tool_calls) {
            if (tc.name === "invoke_port" && tc.result?.ok) {
              const capId = tc.args?.capability_id;
              if (capId === "query" || capId === "execute") {
                const tables = extractTableNames(tc.args?.input?.sql);
                for (const t of tables) {
                  if (!hasTable(contextId, t)) {
                    introspectTable(soma, contextId, t).catch(() => {});
                  }
                }
              }
            }
          }
        }

        return sendJson(res, 201, {
          status: "ok",
          user_message: userAppend.message,
          assistant_message: assistantAppend.message,
          model: reply.model,
          tool_calls: reply.tool_calls,
        });
      }

      // ---- SSE: real-time events (scheduler, webhook, email) ----
      const sseMatch = path.match(/^\/api\/contexts\/([^/]+)\/events$/);
      if (sseMatch && method === "GET") {
        const contextId = sseMatch[1];
        const user = await requireUser(req, res);
        if (!user) return;
        const ctx = await contexts.loadContext(user.id, contextId);
        if (!ctx.ok) return sendJson(res, 404, { status: "error", error: "not found" });

        res.writeHead(200, {
          "Content-Type": "text/event-stream",
          "Cache-Control": "no-cache",
          Connection: "keep-alive",
        });
        res.write(`data: ${JSON.stringify({ type: "connected" })}\n\n`);

        const onEvent = (evt) => {
          res.write(`data: ${JSON.stringify(evt)}\n\n`);
        };
        if (soma.events) {
          soma.events.on("scheduler", onEvent);
          soma.events.on("scheduler_brain", onEvent);
          soma.events.on("webhook", onEvent);
          soma.events.on("email", onEvent);
          soma.events.on("reactive", onEvent);
        }
        req.on("close", () => {
          if (soma.events) {
            soma.events.removeListener("scheduler", onEvent);
            soma.events.removeListener("scheduler_brain", onEvent);
            soma.events.removeListener("webhook", onEvent);
            soma.events.removeListener("email", onEvent);
            soma.events.removeListener("reactive", onEvent);
          }
        });
        return;
      }

      // ---- file upload (text files via filesystem port) ----
      const uploadMatch = path.match(/^\/api\/contexts\/([^/]+)\/upload$/);
      if (uploadMatch && method === "POST") {
        const contextId = uploadMatch[1];
        const user = await requireUser(req, res);
        if (!user) return;
        const ctx = await contexts.loadContext(user.id, contextId);
        if (!ctx.ok) return sendJson(res, 404, { status: "error", error: "not found" });

        const filename = url.searchParams.get("filename") || "upload.txt";
        const safeName = filename.replace(/[/\\]/g, "_");
        let raw;
        try {
          raw = await readRawBody(req, 2 * 1024 * 1024);
        } catch (e) {
          return sendJson(res, e.statusCode ?? 400, { status: "error", error: e.message });
        }
        if (!raw || raw.length === 0) return sendJson(res, 400, { status: "error", error: "empty file body" });
        const content = raw.toString("utf8");
        const namespace = contextNamespace(contextId);
        const dirPath = `/tmp/soma_uploads/${namespace}`;
        const uploadPath = `${dirPath}/${safeName}`;
        try { await soma.invokePort("filesystem", "mkdir", { path: dirPath }); } catch {}
        try {
          await soma.invokePort("filesystem", "writefile", { path: uploadPath, content });
        } catch (err) {
          return sendJson(res, 500, { status: "error", error: `write failed: ${err.message}` });
        }
        if (soma.events) {
          soma.events.emit("webhook", { _webhook_event: true, name: "file-upload", payload: { filename: safeName, path: uploadPath, size: content.length }, received_at: new Date().toISOString() });
        }
        return sendJson(res, 200, { status: "ok", filename: safeName, path: uploadPath, size: content.length });
      }

      // ---- secrets vault ----
      const secretNameMatch = path.match(/^\/api\/contexts\/([^/]+)\/secrets\/([^/]+)$/);
      if (secretNameMatch && (method === "GET" || method === "DELETE")) {
        if (!secretsEnabled()) return sendJson(res, 501, { status: "error", error: "SOMA_SECRETS_KEY not configured" });
        const contextId = secretNameMatch[1];
        const secretName = decodeURIComponent(secretNameMatch[2]);
        const user = await requireUser(req, res);
        if (!user) return;
        const ctx = await contexts.loadContext(user.id, contextId);
        if (!ctx.ok) return sendJson(res, 404, { status: "error", error: "not found" });
        const namespace = contextNamespace(contextId);
        if (method === "GET") {
          const value = await getSecret(soma, namespace, secretName);
          return sendJson(res, value !== null ? 200 : 404, value !== null ? { status: "ok", name: secretName, value } : { status: "error", error: "secret not found" });
        }
        await deleteSecret(soma, namespace, secretName);
        return sendJson(res, 200, { status: "ok", deleted: secretName });
      }

      const secretsMatch = path.match(/^\/api\/contexts\/([^/]+)\/secrets$/);
      if (secretsMatch && (method === "GET" || method === "POST")) {
        if (!secretsEnabled()) return sendJson(res, 501, { status: "error", error: "SOMA_SECRETS_KEY not configured" });
        const contextId = secretsMatch[1];
        const user = await requireUser(req, res);
        if (!user) return;
        const ctx = await contexts.loadContext(user.id, contextId);
        if (!ctx.ok) return sendJson(res, 404, { status: "error", error: "not found" });
        const namespace = contextNamespace(contextId);
        await ensureSecretsTable(soma, namespace);
        if (method === "GET") {
          const secrets = await listSecrets(soma, namespace);
          return sendJson(res, 200, { status: "ok", secrets });
        }
        const body = JSON.parse(await readBody(req));
        if (!body.name || !body.value) return sendJson(res, 400, { status: "error", error: "name and value required" });
        await storeSecret(soma, namespace, body.name, body.value);
        return sendJson(res, 200, { status: "ok", stored: body.name });
      }

      // ---- collaborators ----
      const collabMatch = path.match(/^\/api\/contexts\/([^/]+)\/collaborators(?:\/([^/]+))?$/);
      if (collabMatch) {
        const contextId = collabMatch[1];
        const targetUserId = collabMatch[2] ? decodeURIComponent(collabMatch[2]) : null;
        const user = await requireUser(req, res);
        if (!user) return;

        if (method === "GET" && !targetUserId) {
          const ctx = await contexts.loadContext(user.id, contextId);
          if (!ctx.ok) return sendJson(res, 404, { status: "error", error: "not found" });
          const result = await soma.invokePort("postgres", "query", {
            sql: "SELECT cc.user_id, u.email, cc.added_at FROM context_collaborators cc JOIN users u ON u.id = cc.user_id WHERE cc.context_id = $1::text::uuid",
            params: [contextId],
          });
          return sendJson(res, 200, { status: "ok", collaborators: result.rows || [] });
        }

        if (method === "POST" && !targetUserId) {
          const ctx = await contexts.loadContext(user.id, contextId);
          if (!ctx.ok) return sendJson(res, 404, { status: "error", error: "not found" });
          const body = JSON.parse(await readBody(req));
          if (!body.email) return sendJson(res, 400, { status: "error", error: "email required" });
          const userResult = await soma.invokePort("postgres", "query", { sql: "SELECT id FROM users WHERE email = $1", params: [body.email] });
          if (!userResult.rows?.length) return sendJson(res, 404, { status: "error", error: "user not found" });
          const collabId = userResult.rows[0].id;
          await soma.invokePort("postgres", "execute", {
            sql: "INSERT INTO context_collaborators (context_id, user_id) VALUES ($1::text::uuid, $2::text::uuid) ON CONFLICT DO NOTHING",
            params: [contextId, collabId],
          });
          return sendJson(res, 200, { status: "ok", added: body.email });
        }

        if (method === "DELETE" && targetUserId) {
          const ctx = await contexts.loadContext(user.id, contextId);
          if (!ctx.ok) return sendJson(res, 404, { status: "error", error: "not found" });
          await soma.invokePort("postgres", "execute", {
            sql: "DELETE FROM context_collaborators WHERE context_id = $1::text::uuid AND user_id = $2::text::uuid",
            params: [contextId, targetUserId],
          });
          return sendJson(res, 200, { status: "ok", removed: targetUserId });
        }
      }

      // /api/contexts/:id — GET loads, DELETE removes.
      if (
        (method === "GET" || method === "DELETE") &&
        path.startsWith("/api/contexts/")
      ) {
        const contextId = path.slice("/api/contexts/".length);
        if (!contextId || contextId.includes("/")) {
          return sendJson(res, 404, {
            status: "error",
            error: "not found",
          });
        }
        const user = await requireUser(req, res);
        if (!user) return;

        if (method === "GET") {
          const result = await contexts.loadContext(user.id, contextId);
          if (!result.ok) {
            return sendJson(res, 404, {
              status: "error",
              error: result.error,
            });
          }
          return sendJson(res, 200, {
            status: "ok",
            context: result.context,
          });
        }

        // DELETE
        const result = await contexts.deleteContext(user.id, contextId);
        if (!result.ok) {
          return sendJson(res, 404, {
            status: "error",
            error: result.error,
          });
        }
        return sendJson(res, 200, { status: "ok" });
      }

      // ---- voice transcription ----
      if (method === "POST" && path === "/api/transcribe") {
        const user = await requireUser(req, res);
        if (!user) return;

        let audio;
        try {
          audio = await readRawBody(req);
        } catch (e) {
          const code = e.statusCode ?? 400;
          return sendJson(res, code, {
            status: "error",
            error: e.message,
          });
        }
        if (!audio || audio.length === 0) {
          return sendJson(res, 400, {
            status: "error",
            error: "empty audio body",
          });
        }
        const rawCt = req.headers["content-type"] || "";
        if (rawCt.startsWith("application/json")) {
          return sendJson(res, 400, {
            status: "error",
            error: "content-type must be audio/*",
          });
        }
        const result = await transcribeAudio({
          audioBuffer: audio,
          mimeType: rawCt || "audio/webm",
        });
        if (!result.ok) {
          return sendJson(res, 502, {
            status: "error",
            error: result.error,
          });
        }
        return sendJson(res, 200, {
          status: "ok",
          text: result.text,
          model: result.model,
        });
      }

      // ---- webhook instruction management ----
      const webhookInstrMatch = path.match(/^\/api\/webhooks\/([a-zA-Z0-9_-]+)\/instruction$/);
      if (webhookInstrMatch) {
        const hookName = webhookInstrMatch[1];

        if (method === "PUT") {
          const body = await readBody(req);
          const { instruction } = body;
          if (!instruction) return sendJson(res, 400, { error: "instruction required" });
          webhookInstructions.set(hookName, instruction);
          return sendJson(res, 200, { status: "ok", hook: hookName, instruction });
        }

        if (method === "DELETE") {
          webhookInstructions.delete(hookName);
          return sendJson(res, 200, { status: "ok", hook: hookName, instruction: null });
        }

        if (method === "GET") {
          return sendJson(res, 200, { hook: hookName, instruction: webhookInstructions.get(hookName) || null });
        }
      }

      // ---- webhook receiver (proxy to soma-next's HTTP listener) ----
      // Webhook receive + world state patching happens in soma-next.
      // The terminal just proxies the request. The webhook event arrives
      // back via stderr JSON → SSE, same as scheduler events.
      const webhookMatch = path.match(/^\/api\/webhooks\/([a-zA-Z0-9_-]+)$/);
      if (webhookMatch && method === "POST") {
        const hookName = webhookMatch[1];
        const rawBody = await readBody(req);

        // Proxy to soma-next's webhook listener.
        try {
          const bodyStr = typeof rawBody === "string" ? rawBody : JSON.stringify(rawBody);
          const proxyRes = await fetch(`http://${soma.webhookAddr}/${hookName}`, {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
              "Content-Length": String(Buffer.byteLength(bodyStr)),
            },
            body: bodyStr,
          });
          const proxyBody = await proxyRes.json();
          sendJson(res, proxyRes.status, proxyBody);
        } catch (err) {
          sendJson(res, 502, { status: "error", error: `webhook proxy failed: ${err.message}` });
        }

        // Brain instruction processing (terminal-level — brain logic).
        const instruction = webhookInstructions.get(hookName);
        if (instruction) {
          let parsedBody = {};
          try { parsedBody = JSON.parse(rawBody); } catch {}
          (async () => {
            try {
              const briefPrompt = `You are SOMA processing an inbound webhook event.

Webhook: "${hookName}"
Payload: ${JSON.stringify(parsedBody)}

Operator instruction: ${instruction}

Follow the instruction. Use invoke_port if you need to take action (send email, update database, etc.). Keep your response concise.`;

              const reply = await runChatTurn({
                systemPrompt: briefPrompt,
                history: [],
                tools: DEFAULT_CHAT_TOOLS,
                invokeTool,
                temperature: 0.3,
                maxTokens: 400,
              });
              console.log(
                `[webhook] ${hookName}:brain → ${reply.content?.slice(0, 200)}` +
                  (reply.tool_calls?.length ? ` (${reply.tool_calls.length} tool calls)` : ""),
              );
            } catch (err) {
              console.error(`[webhook] ${hookName}:brain-error → ${err.message}`);
            }
          })();
        }

        return;
      }

      // ---- health check ----
      if (method === "GET" && path === "/api/health") {
        return sendJson(res, 200, {
          status: "ok",
          generation: "conversation-first",
          soma_mcp_ready: soma.ready,
          brain_fake: String(process.env.BRAIN_FAKE || "") === "1",
        });
      }

      // ---- static fallback ----
      if (method === "GET" || method === "HEAD") {
        return await serveStatic(req, res, path);
      }

      return sendJson(res, 404, { status: "error", error: "not found" });
    } catch (err) {
      const code = err.statusCode ?? 500;
      console.error(`[http] ${method} ${path} → ${code}:`, err.message);
      return sendJson(res, code, {
        status: "error",
        error: err.message || "internal error",
      });
    }
  };

  const server = http.createServer(handle);
  server.listen(PORT, "127.0.0.1", () => {
    console.log(`[http] soma-project-terminal listening on http://127.0.0.1:${PORT}`);
    console.log(`[http] SOMA_POSTGRES_URL=${process.env.SOMA_POSTGRES_URL}`);
    console.log(`[http] SOMA_SMTP=${process.env.SOMA_SMTP_HOST}:${process.env.SOMA_SMTP_PORT}`);
    console.log(`[http] PUBLIC_BASE_URL=${process.env.PUBLIC_BASE_URL}`);
  });

  const shutdown = async (signal) => {
    console.log(`\n[http] ${signal} received, shutting down`);
    server.close();
    await soma.close();
    process.exit(0);
  };
  process.on("SIGINT", () => shutdown("SIGINT"));
  process.on("SIGTERM", () => shutdown("SIGTERM"));
}

main().catch((err) => {
  console.error("[http] fatal startup error:", err);
  process.exit(1);
});
