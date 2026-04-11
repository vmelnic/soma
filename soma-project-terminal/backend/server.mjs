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
// Every database read/write goes through postgres.execute / query.
// Every email goes through smtp.send_plain. Every random token and
// sha256 hash goes through crypto.random_string / crypto.sha256.
// The only direct Node deps are node:http, node:fs, node:path — no
// pg, no nodemailer, nothing in node_modules.
//
// Commit 2+ adds /api/invoke_port, /api/brain, /api/contexts.

import http from "node:http";
import { readFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { extname, normalize, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";

import { SomaMcpClient } from "./soma-mcp.mjs";
import { createAuth } from "./auth.mjs";

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
  const cookie = readCookie(req, "soma_session");
  if (cookie) return cookie;
  const h = req.headers.authorization;
  if (h && h.toLowerCase().startsWith("bearer ")) {
    return h.slice(7).trim();
  }
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

      // ---- health check ----
      if (method === "GET" && path === "/api/health") {
        return sendJson(res, 200, {
          status: "ok",
          commit: 1,
          soma_mcp_ready: soma.ready,
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
