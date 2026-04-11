// Context registry — SOMA-native.
//
// A "context" is the user's project: a named row owned by a user.
// Commit 2 exposes bare CRUD; commits 3+ grow the row into a proper
// pack manifest (name + description + compiled PackSpec JSON) that
// a browser-side soma-next runtime will load dynamically.
//
// Every side effect routes through `SomaMcpClient.invokePort`:
//   - `postgres.query`   reads
//   - `postgres.execute` inserts + updates + deletes
//
// Ownership is enforced in SQL: every lookup includes
// `WHERE user_id = $2::text::uuid` so one operator's query for
// another operator's context id returns zero rows, same shape as
// "not found". No leaking of existence across tenants.
//
// The `$N::text::uuid` double cast is mandatory for every UUID bind.
// The postgres port serializes all parameters as TEXT, and Postgres'
// parameter-type inference would otherwise see `$N::uuid` and infer
// $N as UUID — tokio-postgres then rejects the `&str` bind. Forcing
// TEXT and parsing server-side is the only path that works.

const NAME_MAX = 120;
const DESCRIPTION_MAX = 2000;

function isNonEmptyString(s, max) {
  return typeof s === "string" && s.trim().length > 0 && s.length <= max;
}

// Rough UUID shape check — the postgres port will reject malformed
// UUIDs on its own, but rejecting here avoids a round trip and a
// `ERROR: invalid input syntax for type uuid` surfacing as a 500.
function looksLikeUuid(s) {
  return (
    typeof s === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)
  );
}

export function createContexts(soma) {
  // ---- create ----
  async function createContext(userId, input) {
    if (!looksLikeUuid(userId)) {
      return { ok: false, error: "invalid user id" };
    }
    const name = typeof input?.name === "string" ? input.name.trim() : "";
    if (!isNonEmptyString(name, NAME_MAX)) {
      return { ok: false, error: "name is required" };
    }
    const description =
      typeof input?.description === "string"
        ? input.description.slice(0, DESCRIPTION_MAX)
        : null;

    // INSERT ... RETURNING is the single-statement way to get the
    // generated UUID + timestamps back without a second SELECT.
    // Timestamp columns collapse to null through the postgres port's
    // row-to-json path, so we re-fetch via loadContext below rather
    // than trust the RETURNING row for created_at/updated_at.
    const inserted = await soma.invokePort("postgres", "query", {
      sql:
        `INSERT INTO contexts (user_id, name, description) ` +
        `VALUES ($1::text::uuid, $2, $3) ` +
        `RETURNING id`,
      params: [userId, name, description],
    });
    const row = inserted.rows?.[0];
    if (!row) {
      return { ok: false, error: "failed to create context" };
    }
    const full = await loadContext(userId, row.id);
    return full; // { ok, context } | { ok: false, error }
  }

  // ---- list ----
  async function listForUser(userId) {
    if (!looksLikeUuid(userId)) {
      return { ok: false, error: "invalid user id" };
    }
    // ORDER BY updated_at DESC matches the covering index. Archived
    // rows are hidden from the default listing. `id DESC` is the
    // tiebreaker when two rows share the same updated_at — UUIDs
    // aren't time-monotonic but they ARE stable, which is all we
    // need to keep the listing deterministic.
    const result = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT id, name, description, kind, ` +
        `       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at, ` +
        `       to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at ` +
        `FROM contexts ` +
        `WHERE user_id = $1::text::uuid AND kind <> 'archived' ` +
        `ORDER BY updated_at DESC, id DESC`,
      params: [userId],
    });
    return { ok: true, contexts: result.rows ?? [] };
  }

  // ---- load one ----
  // Timestamps are formatted server-side via to_char() to work around
  // the postgres port's timestamptz-to-json gap (see commit 1 notes
  // in auth.mjs for the same workaround on session expires_at).
  async function loadContext(userId, contextId) {
    if (!looksLikeUuid(userId) || !looksLikeUuid(contextId)) {
      return { ok: false, error: "not found" };
    }
    const result = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT id, name, description, kind, ` +
        `       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS created_at, ` +
        `       to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS updated_at ` +
        `FROM contexts ` +
        `WHERE id = $1::text::uuid AND user_id = $2::text::uuid`,
      params: [contextId, userId],
    });
    const row = result.rows?.[0];
    if (!row) return { ok: false, error: "not found" };
    return { ok: true, context: row };
  }

  // ---- delete ----
  // Hard delete for now. Commit 5 adds per-context episode/schema/
  // routine stores, and cascading those is a real concern — but for
  // commit 2 a context is just one row and no child data exists.
  async function deleteContext(userId, contextId) {
    if (!looksLikeUuid(userId) || !looksLikeUuid(contextId)) {
      return { ok: false, error: "not found" };
    }
    // Scope the DELETE to the owner so a crafted id from another
    // tenant is a no-op, not a leak.
    const existing = await loadContext(userId, contextId);
    if (!existing.ok) return existing;
    await soma.invokePort("postgres", "execute", {
      sql:
        `DELETE FROM contexts ` +
        `WHERE id = $1::text::uuid AND user_id = $2::text::uuid`,
      params: [contextId, userId],
    });
    return { ok: true };
  }

  return { createContext, listForUser, loadContext, deleteContext };
}
