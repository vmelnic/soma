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

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const FRONTEND_DIR = resolvePath(__dirname, "..", "frontend");
const BIN_DIR = resolvePath(__dirname, "..", "bin");
const PORT = Number(process.env.BACKEND_PORT ?? 8765);

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
  const auth = createAuth(soma);
  const contexts = createContexts(soma);
  const messages = createMessages(soma);

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
        let reply;
        try {
          reply = await runChatTurn({
            systemPrompt: buildSystemPrompt(
              ctx.context,
              soma.getPortCatalogSummary(),
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
        return sendJson(res, 201, {
          status: "ok",
          user_message: userAppend.message,
          assistant_message: assistantAppend.message,
          model: reply.model,
          tool_calls: reply.tool_calls,
        });
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
