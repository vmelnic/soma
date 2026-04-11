// soma-project-terminal — Node HTTP gateway.
//
// Commit 1 responsibilities:
//   - Spawn soma-next in --mcp mode with the platform pack (crypto
//     + postgres + smtp ports) and hold the MCP stdio handle.
//   - Serve static files from ../frontend/ (the Fallout terminal UI).
//   - Expose auth endpoints backed by SOMA port invocations:
//       POST /api/auth/request-link
//       GET  /api/auth/verify
//       GET  /api/me
//       POST /api/auth/logout
//
// Commit 2 adds the context registry:
//       GET    /api/contexts
//       POST   /api/contexts
//       GET    /api/contexts/:id
//       DELETE /api/contexts/:id
//
// Commit 3 adds the per-context chat brain:
//       GET  /api/contexts/:id/messages
//       POST /api/contexts/:id/messages   (append user + call brain)
//
// Commit 4 adds the dynamic pack loader:
//       PUT  /api/contexts/:id/pack
// which validates + stores a manifest on the context row so the
// browser-side wasm runtime boots that pack next time the context
// is opened (see frontend/runtime.mjs for the hot-swap).
//
// Commit 5 adds per-context memory isolation:
//       GET    /api/contexts/:id/memory
//       POST   /api/contexts/:id/memory/episodes
//       POST   /api/contexts/:id/memory/schemas
//       POST   /api/contexts/:id/memory/routines
//       DELETE /api/contexts/:id/memory
// Episodes / schemas / routines each live in their own table with
// ownership enforced by joining contexts. One context's memory is
// opaque to every other — a leak here would silently break the
// whole multi-tenant story, so it's tested explicitly.
//
// Every database read/write goes through postgres.execute / query.
// Every email goes through smtp.send_plain. Every random token and
// sha256 hash goes through crypto.random_string / crypto.sha256.
// The only direct Node deps are node:http, node:fs, node:path — no
// pg, no nodemailer, nothing in node_modules. The OpenAI brain uses
// the global `fetch` introduced in Node 18+ (zero-dep wrapper in
// backend/brain.mjs).
//
// Commit 4+ adds dynamic pack loading; commit 6 hits
// brain.reasoningCompletion for LLM-to-PackSpec.

import http from "node:http";
import { readFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { extname, normalize, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";

import { SomaMcpClient } from "./soma-mcp.mjs";
import { createAuth } from "./auth.mjs";
import { createContexts } from "./contexts.mjs";
import { createMessages } from "./messages.mjs";
import { createMemory } from "./memory.mjs";
import { chatCompletion, buildSystemPrompt } from "./brain.mjs";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const FRONTEND_DIR = resolvePath(__dirname, "..", "frontend");
const BIN_DIR = resolvePath(__dirname, "..", "bin");
const PORT = Number(process.env.BACKEND_PORT ?? 8765);

// ------------------------------------------------------------------
// startup preflight — verify the binaries we need actually exist
// ------------------------------------------------------------------

function preflight() {
  const required = [
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
  const missing = required.filter((r) => !existsSync(r.path));
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
// small helpers
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
  // application/wasm is required — browsers refuse to stream-
  // instantiate wasm modules served with the wrong Content-Type.
  ".wasm": "application/wasm",
  ".ts": "text/plain; charset=utf-8",
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
  // silently override an intentional API call, which is exactly
  // the kind of confused-deputy failure that breaks multi-tenant
  // scope isolation.
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
  const memory = createMemory(soma);

  // Helper — every context route requires a valid session. Returns
  // the user record on success, or sends a 401 and returns null.
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
      // ---- auth endpoints ----
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
      // Listing, creating, loading, and deleting the operator's
      // contexts. Every route is session-scoped via `requireUser`
      // and every SQL query filters by `user_id` so one operator's
      // id lookup for another operator's context returns "not found"
      // rather than leaking existence across tenants.
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

      // /api/contexts/:id/memory[...] — list all three tiers, append
      // to one tier, or clear the whole memory for a context. The
      // ownership check lives in backend/memory.mjs via a contexts
      // join, so an id probe from another tenant gets "not found".
      const memoryMatch = path.match(
        /^\/api\/contexts\/([^/]+)\/memory(?:\/(episodes|schemas|routines))?$/,
      );
      if (memoryMatch) {
        const contextId = memoryMatch[1];
        const category = memoryMatch[2] || null;
        const user = await requireUser(req, res);
        if (!user) return;

        // GET /memory → full snapshot of all three tiers.
        if (!category && method === "GET") {
          const result = await memory.listMemory(user.id, contextId);
          if (!result.ok) {
            return sendJson(res, 404, {
              status: "error",
              error: result.error,
            });
          }
          return sendJson(res, 200, {
            status: "ok",
            memory: result.memory,
          });
        }

        // DELETE /memory → clear all three tiers.
        if (!category && method === "DELETE") {
          const result = await memory.clearMemory(user.id, contextId);
          if (!result.ok) {
            return sendJson(res, 404, {
              status: "error",
              error: result.error,
            });
          }
          return sendJson(res, 200, { status: "ok" });
        }

        // POST /memory/<episodes|schemas|routines> → append one row.
        if (category && method === "POST") {
          const body = await readBody(req);
          const payload = body?.payload ?? body;
          const name = body?.name ?? null;

          let result;
          if (category === "episodes") {
            result = await memory.appendEpisode(
              user.id,
              contextId,
              payload,
            );
          } else if (category === "schemas") {
            result = await memory.appendSchema(
              user.id,
              contextId,
              name,
              payload,
            );
          } else {
            result = await memory.appendRoutine(
              user.id,
              contextId,
              name,
              payload,
            );
          }

          if (!result.ok) {
            const code = result.error === "not found" ? 404 : 400;
            return sendJson(res, code, {
              status: "error",
              error: result.error,
            });
          }
          return sendJson(res, 201, {
            status: "ok",
            category,
            row: result.row,
          });
        }

        return sendJson(res, 405, {
          status: "error",
          error: "method not allowed",
        });
      }

      // /api/contexts/:id/pack — PUT stores the compiled PackSpec.
      // Matched before the single-context handler so the /pack
      // suffix doesn't get mis-parsed as part of the context id.
      const packMatch = path.match(/^\/api\/contexts\/([^/]+)\/pack$/);
      if (packMatch && method === "PUT") {
        const contextId = packMatch[1];
        const user = await requireUser(req, res);
        if (!user) return;
        const body = await readBody(req);
        // Accept { pack: {...} } (the documented shape) or a bare
        // manifest object — the latter makes curl debugging easier.
        const packInput =
          body && typeof body === "object" && "pack" in body
            ? body.pack
            : body;
        const result = await contexts.setPackSpec(
          user.id,
          contextId,
          packInput,
        );
        if (!result.ok) {
          const code = result.error === "not found" ? 404 : 400;
          return sendJson(res, code, {
            status: "error",
            error: result.error,
          });
        }
        return sendJson(res, 200, {
          status: "ok",
          context: result.context,
        });
      }

      // /api/contexts/:id/messages — GET lists the transcript, POST
      // appends a user message, calls the brain, appends the reply,
      // and returns both in one response. Matched before the single-
      // context handler so the /messages suffix doesn't get mis-
      // parsed as part of the context id.
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

        // The user turn lands first so that a brain failure still
        // leaves the operator's message on the transcript. Otherwise
        // a crash mid-brain would swallow what they typed.
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

        // Pull the full history (which now ends with the user turn
        // we just appended) and the context record for its system
        // prompt.
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

        let reply;
        try {
          reply = await chatCompletion({
            systemPrompt: buildSystemPrompt(ctx.context),
            messages: hist.history,
          });
        } catch (e) {
          console.error(`[brain] chatCompletion failed:`, e.message);
          return sendJson(res, 502, {
            status: "error",
            error: `brain failed: ${e.message}`,
            user_message: userAppend.message,
          });
        }

        const assistantAppend = await messages.append(
          user.id,
          contextId,
          "assistant",
          reply.content,
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
        });
      }

      // /api/contexts/:id — GET loads, DELETE removes. Matching the
      // path with startsWith rather than a router keeps the gateway
      // a flat file; commit 3+ will grow an actual router if the
      // route table gets unwieldy.
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

      // ---- health check ----
      if (method === "GET" && path === "/api/health") {
        return sendJson(res, 200, {
          status: "ok",
          commit: 5,
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

  // Clean shutdown — close the soma-next subprocess before we exit.
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
