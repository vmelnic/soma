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
    //
    // The listing intentionally omits `pack_spec` — manifests can
    // be multi-kilobyte and the sidebar only needs name + metadata.
    // Callers fetch the pack via loadContext.
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
  //
  // Commit 4: also returns `pack_spec` (the JSON-encoded manifest
  // the browser boots into the wasm runtime). NULL until a pack
  // has been stored — callers should fall back to the shared hello
  // pack in that case.
  async function loadContext(userId, contextId) {
    if (!looksLikeUuid(userId) || !looksLikeUuid(contextId)) {
      return { ok: false, error: "not found" };
    }
    const result = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT id, name, description, kind, pack_spec, ` +
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

  // ---- set pack ----
  // Stores a JSON-encoded PackSpec on the context and flips `kind`
  // to 'active'. Commit 6 will call this from the LLM-generated
  // output; for now tests exercise it directly with a hand-crafted
  // manifest.
  //
  // Validation happens in two places:
  //   1. The caller passed a parsed object OR a JSON string; we
  //      canonicalise to a string before storing.
  //   2. JSON.parse(packJson) must succeed (so we don't store
  //      malformed text that the browser would trip over later).
  //
  // Ownership is enforced on the UPDATE itself — the WHERE clause
  // includes `user_id` so an id probe from another tenant is a
  // no-op and returns "not found", same shape as an unknown
  // context.
  async function setPackSpec(userId, contextId, packInput) {
    if (!looksLikeUuid(userId) || !looksLikeUuid(contextId)) {
      return { ok: false, error: "not found" };
    }

    // Accept either a string or an object — the API layer sends
    // JSON, and JS callers can hand us a parsed object directly.
    let packString;
    if (typeof packInput === "string") {
      packString = packInput;
    } else if (packInput && typeof packInput === "object") {
      packString = JSON.stringify(packInput);
    } else {
      return { ok: false, error: "pack must be an object or JSON string" };
    }

    let parsed;
    try {
      parsed = JSON.parse(packString);
    } catch (e) {
      return { ok: false, error: `invalid JSON: ${e.message}` };
    }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return { ok: false, error: "pack must be a JSON object" };
    }
    // Lightweight shape check — the wasm runtime does its own full
    // validation, but rejecting obvious typos here surfaces a 400
    // instead of a browser-side boot error later.
    if (typeof parsed.id !== "string" || parsed.id.trim() === "") {
      return { ok: false, error: "pack.id is required" };
    }
    if (!Array.isArray(parsed.skills)) {
      return { ok: false, error: "pack.skills must be an array" };
    }

    // Ownership check + update in one statement via RETURNING so we
    // can tell the "not found" path from a successful UPDATE without
    // trusting the postgres port's execute-result shape. `updated_at`
    // is bumped server-side so the sidebar floats this context to the
    // top of the list.
    const updated = await soma.invokePort("postgres", "query", {
      sql:
        `UPDATE contexts ` +
        `SET pack_spec = $1, kind = 'active', updated_at = NOW() ` +
        `WHERE id = $2::text::uuid AND user_id = $3::text::uuid ` +
        `RETURNING id`,
      params: [packString, contextId, userId],
    });
    if (!updated.rows?.[0]) {
      return { ok: false, error: "not found" };
    }
    return await loadContext(userId, contextId);
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

  return {
    createContext,
    listForUser,
    loadContext,
    deleteContext,
    setPackSpec,
  };
}
