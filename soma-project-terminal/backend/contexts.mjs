// Context registry — SOMA-native.
//
// A context is the operator's "project": a named conversation scope
// holding its own chat transcript and (eventually) its own scoped
// data space in postgres. All contexts share the ONE master pack
// the backend has loaded in soma-next — there is no per-context
// pack_spec, no pack generation, no LLM-produced artifacts. When
// the operator talks in a context, the chat brain's tool calls go
// to the shared runtime with the context_id threaded through so
// data stays scoped.
//
// Every side effect routes through `SomaMcpClient.invokePort`:
//   - `postgres.query`   reads
//   - `postgres.execute` inserts + updates + deletes
//
// Ownership is enforced in SQL: every lookup includes
// `WHERE user_id = $N::text::uuid` so one operator's query for
// another operator's context id returns zero rows, same shape as
// "not found". No leaking of existence across tenants.
//
// The `$N::text::uuid` double cast is mandatory for every UUID
// bind. The postgres port serializes all parameters as TEXT, and
// Postgres' parameter-type inference would otherwise see `$N::uuid`
// and infer $N as UUID — tokio-postgres then rejects the `&str`
// bind. Forcing TEXT and parsing server-side is the only path that
// works.

const NAME_MAX = 120;
const DESCRIPTION_MAX = 2000;

function isNonEmptyString(s, max) {
  return typeof s === "string" && s.trim().length > 0 && s.length <= max;
}

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
    return loadContext(userId, row.id);
  }

  // ---- list ----
  async function listForUser(userId) {
    if (!looksLikeUuid(userId)) {
      return { ok: false, error: "invalid user id" };
    }
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
  // Hard delete. ON DELETE CASCADE on messages takes care of the
  // transcript.
  async function deleteContext(userId, contextId) {
    if (!looksLikeUuid(userId) || !looksLikeUuid(contextId)) {
      return { ok: false, error: "not found" };
    }
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

  // ---- bump updated_at ----
  // Called from the chat route on every message so the context
  // floats to the top of the sidebar. Ownership-scoped like every
  // other write — a crafted id from another tenant is a no-op.
  async function bumpUpdatedAt(userId, contextId) {
    if (!looksLikeUuid(userId) || !looksLikeUuid(contextId)) return;
    await soma.invokePort("postgres", "execute", {
      sql:
        `UPDATE contexts SET updated_at = NOW() ` +
        `WHERE id = $1::text::uuid AND user_id = $2::text::uuid`,
      params: [contextId, userId],
    });
  }

  return {
    createContext,
    listForUser,
    loadContext,
    deleteContext,
    bumpUpdatedAt,
  };
}
