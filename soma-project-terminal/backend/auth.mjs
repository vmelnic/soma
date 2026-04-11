// Magic-link auth flow — SOMA-native.
//
// Every side effect routes through `SomaMcpClient.invokePort`:
//   - `crypto.random_string` generates raw tokens (magic + session)
//   - `crypto.sha256`         hashes tokens for at-rest storage
//   - `postgres.query`        reads
//   - `postgres.execute`      inserts + updates
//   - `smtp.send_plain`       dispatches the magic-link email
//
// No direct pg / nodemailer calls. No session state in any port's
// memory (the auth port IS available but it stores sessions in an
// in-memory HashMap, which would dump every user on process restart
// — session persistence is a database concern, not a port concern).
//
// Timestamp arithmetic happens in SQL (NOW() + INTERVAL ...) rather
// than JS — the postgres port serializes every parameter as a plain
// string with tokio-postgres' default TEXT encoding, which Postgres
// refuses to implicitly cast into TIMESTAMPTZ. SQL-side intervals
// sidestep the issue entirely, and the server's clock becomes
// authoritative for all expiries.

const MAGIC_TOKEN_LEN = 48; // alphanumeric chars — the crypto port's charset
const SESSION_TOKEN_LEN = 48;

function isValidEmail(s) {
  return typeof s === "string" && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(s);
}

// Numeric sanity check before interpolating into SQL. These values
// come from env vars, not user input, so the risk is configuration
// typos, not injection.
function positiveInt(n, fallback) {
  const v = Number(n);
  return Number.isFinite(v) && v > 0 && v < 1_000_000 ? Math.floor(v) : fallback;
}

export function createAuth(soma) {
  const MAGIC_TTL_MIN = positiveInt(process.env.MAGIC_TOKEN_TTL_MINUTES, 15);
  const SESSION_TTL_DAYS = positiveInt(process.env.SESSION_TTL_DAYS, 30);
  const PUBLIC_BASE_URL =
    process.env.PUBLIC_BASE_URL || "http://localhost:8765";

  // Precomputed SQL interval fragments — safe because TTLs are env
  // constants that we validated as positive integers above.
  const MAGIC_INTERVAL = `INTERVAL '${MAGIC_TTL_MIN} minutes'`;
  const SESSION_INTERVAL = `INTERVAL '${SESSION_TTL_DAYS} days'`;

  async function sha256Hex(data) {
    const out = await soma.invokePort("crypto", "sha256", { data });
    return out.hash;
  }

  async function randomToken(length) {
    const out = await soma.invokePort("crypto", "random_string", { length });
    return out.value;
  }

  // ---- request-link ----
  async function requestMagicLink(email) {
    if (!isValidEmail(email)) {
      return { ok: false, error: "invalid email address" };
    }
    const normalized = email.toLowerCase();
    const rawToken = await randomToken(MAGIC_TOKEN_LEN);
    const tokenHash = await sha256Hex(rawToken);

    await soma.invokePort("postgres", "execute", {
      sql:
        `INSERT INTO magic_tokens (token_hash, email, expires_at) ` +
        `VALUES ($1, $2, NOW() + ${MAGIC_INTERVAL})`,
      params: [tokenHash, normalized],
    });

    const link = `${PUBLIC_BASE_URL}/api/auth/verify?token=${rawToken}`;
    const body = [
      "SOMA TERMINAL v0.1",
      "RobCo Someco Unified Operating System",
      "",
      "An authorization link has been issued for this terminal.",
      "",
      `> ${link}`,
      "",
      `This link will expire in ${MAGIC_TTL_MIN} minutes.`,
      "If you did not request access, ignore this transmission.",
      "",
      "END OF LINE.",
    ].join("\n");

    try {
      await soma.invokePort("smtp", "send_plain", {
        to: email,
        subject: "SOMA TERMINAL — AUTHORIZATION LINK",
        body,
      });
    } catch (e) {
      console.error(
        `[auth] smtp.send_plain failed for ${email}:`,
        e.message,
      );
      return { ok: false, error: "failed to dispatch email" };
    }

    return { ok: true };
  }

  // ---- verify ----
  async function verifyMagicToken(rawToken, userAgent) {
    if (!rawToken || typeof rawToken !== "string") {
      return { ok: false, error: "missing token" };
    }
    const tokenHash = await sha256Hex(rawToken);

    // Fetch fresh, unused, unexpired token row in a single query.
    const tokenResult = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT email FROM magic_tokens ` +
        `WHERE token_hash = $1 AND used_at IS NULL AND expires_at > NOW()`,
      params: [tokenHash],
    });
    const tokenRow = tokenResult.rows?.[0];
    if (!tokenRow) {
      return { ok: false, error: "invalid or expired token" };
    }

    // Mark the magic token used before any other failure path can
    // leave it reusable.
    await soma.invokePort("postgres", "execute", {
      sql: "UPDATE magic_tokens SET used_at = NOW() WHERE token_hash = $1",
      params: [tokenHash],
    });

    // Find or create the user.
    const existing = await soma.invokePort("postgres", "query", {
      sql: "SELECT id, email FROM users WHERE email = $1",
      params: [tokenRow.email],
    });
    let user = existing.rows?.[0];
    if (!user) {
      // INSERT ... RETURNING via the query capability — the postgres
      // port's query capability handles any statement that returns
      // rows, including INSERT ... RETURNING.
      const ins = await soma.invokePort("postgres", "query", {
        sql:
          "INSERT INTO users (email) VALUES ($1) RETURNING id, email",
        params: [tokenRow.email],
      });
      user = ins.rows?.[0];
      if (!user) {
        return { ok: false, error: "failed to create user" };
      }
    } else {
      // The postgres port serializes every parameter as TEXT. Postgres'
      // parameter-type inference would see `$1::uuid` and infer UUID for
      // $1 directly (no cast needed), so the prepared statement would
      // tell tokio-postgres "param 0 is uuid" and our `&str` bind would
      // error. Forcing `$1::text::uuid` makes $1's inferred type TEXT —
      // the string is parsed as UUID server-side, which is what we want.
      await soma.invokePort("postgres", "execute", {
        sql: "UPDATE users SET last_login = NOW() WHERE id = $1::text::uuid",
        params: [user.id],
      });
    }

    // Issue a long-lived session token. Raw goes to the browser,
    // hash goes to the database.
    const rawSession = await randomToken(SESSION_TOKEN_LEN);
    const sessionHash = await sha256Hex(rawSession);

    // Insert the session row. We compute expires_at client-side too so
    // we don't depend on RETURNING for the timestamp (the postgres
    // port's row-to-json path currently collapses timestamptz columns
    // to null in some situations — harmless for our purposes since
    // `currentUser` filters expiry in SQL via `> NOW()`).
    // $2 is the UUID; force TEXT inference via `$2::text::uuid` so
    // tokio-postgres doesn't try to validate the string against the
    // inferred UUID type (it would otherwise reject the bind).
    await soma.invokePort("postgres", "execute", {
      sql:
        `INSERT INTO sessions (token_hash, user_id, expires_at, user_agent) ` +
        `VALUES ($1, $2::text::uuid, NOW() + ${SESSION_INTERVAL}, $3)`,
      params: [sessionHash, user.id, userAgent ?? null],
    });

    const expiresAt = new Date(
      Date.now() + SESSION_TTL_DAYS * 24 * 60 * 60 * 1000,
    );

    return {
      ok: true,
      session_token: rawSession,
      user: { id: user.id, email: user.email },
      expires_at: expiresAt.toISOString(),
    };
  }

  // ---- me ----
  async function currentUser(rawSessionToken) {
    if (!rawSessionToken) return null;
    const sessionHash = await sha256Hex(rawSessionToken);

    const result = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT s.user_id, u.email ` +
        `FROM sessions s JOIN users u ON u.id = s.user_id ` +
        `WHERE s.token_hash = $1 ` +
        `  AND s.revoked_at IS NULL ` +
        `  AND s.expires_at > NOW()`,
      params: [sessionHash],
    });
    const row = result.rows?.[0];
    if (!row) return null;

    // Slide the active session forward.
    await soma.invokePort("postgres", "execute", {
      sql: "UPDATE sessions SET last_seen = NOW() WHERE token_hash = $1",
      params: [sessionHash],
    });

    return { id: row.user_id, email: row.email };
  }

  // ---- logout ----
  async function logout(rawSessionToken) {
    if (!rawSessionToken) return;
    const sessionHash = await sha256Hex(rawSessionToken);
    await soma.invokePort("postgres", "execute", {
      sql:
        `UPDATE sessions SET revoked_at = NOW() ` +
        `WHERE token_hash = $1 AND revoked_at IS NULL`,
      params: [sessionHash],
    });
  }

  return { requestMagicLink, verifyMagicToken, currentUser, logout };
}
