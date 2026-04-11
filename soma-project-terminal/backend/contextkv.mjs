// Per-context key/value store — the first "bridge port" exposed to
// the browser wasm runtime.
//
// The browser body has no persistent storage port of its own
// (soma-project-web's wasm build ships only dom / audio / voice).
// Instead of shipping a new wasm port, commit 8 exposes a thin KV
// space backed by postgres and reachable from the browser via a
// session-authed + context-scoped HTTP route. Generated packs can
// legitimately reference `context_kv.{set,get,delete,list}` in
// their skills and the JS-side executor (commit 9) routes those
// calls via fetch to the bridge route in server.mjs.
//
// Every read and write joins contexts on user_id so a cross-tenant
// id probe sees "not found" — same pattern as messages.mjs and
// memory.mjs. `ON DELETE CASCADE` on the FK drops a context's KV
// rows along with its parent.
//
// Keys and values are plain TEXT. Callers serialize however they
// want — JSON objects, base64 blobs, free-form notes. The length
// caps below are generous enough for real use and cheap enough to
// stop a runaway test from filling the database.

const KEY_MAX = 400;
const VALUE_MAX = 200_000; // 200 KB per value — roomy for todos, notes, drafts

function looksLikeUuid(s) {
  return (
    typeof s === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)
  );
}

function validateKey(key) {
  if (typeof key !== "string" || key.length === 0) {
    return "key is required";
  }
  if (key.length > KEY_MAX) return "key too long";
  return null;
}

function validateValue(value) {
  if (typeof value !== "string") {
    return "value must be a string";
  }
  if (value.length > VALUE_MAX) return "value too long";
  return null;
}

export function createContextKv(soma) {
  async function assertOwnership(userId, contextId) {
    if (!looksLikeUuid(userId) || !looksLikeUuid(contextId)) {
      return false;
    }
    const result = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT 1 AS ok FROM contexts ` +
        `WHERE id = $1::text::uuid AND user_id = $2::text::uuid`,
      params: [contextId, userId],
    });
    return !!result.rows?.[0];
  }

  // ---- set ----
  // Upsert via ON CONFLICT so `set` is idempotent on key. The
  // UNIQUE (context_id, key) constraint is the conflict target.
  // `updated_at` always bumps on write so the browser can sort
  // by recency.
  async function set(userId, contextId, key, value) {
    const keyErr = validateKey(key);
    if (keyErr) return { ok: false, error: keyErr };
    const valErr = validateValue(value);
    if (valErr) return { ok: false, error: valErr };

    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    const result = await soma.invokePort("postgres", "query", {
      sql:
        `INSERT INTO context_kv (context_id, key, value) ` +
        `VALUES ($1::text::uuid, $2, $3) ` +
        `ON CONFLICT (context_id, key) DO UPDATE ` +
        `  SET value = EXCLUDED.value, updated_at = NOW() ` +
        `RETURNING id, key, value, ` +
        `  to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at, ` +
        `  to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at`,
      params: [contextId, key, value],
    });
    const row = result.rows?.[0];
    if (!row) return { ok: false, error: "failed to set" };
    return { ok: true, row };
  }

  // ---- get ----
  async function get(userId, contextId, key) {
    const keyErr = validateKey(key);
    if (keyErr) return { ok: false, error: keyErr };
    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    const result = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT id, key, value, ` +
        `  to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at, ` +
        `  to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at ` +
        `FROM context_kv ` +
        `WHERE context_id = $1::text::uuid AND key = $2`,
      params: [contextId, key],
    });
    const row = result.rows?.[0];
    if (!row) return { ok: true, row: null };
    return { ok: true, row };
  }

  // ---- delete ----
  // Returns { ok, deleted: boolean } so the caller can distinguish
  // "key wasn't there" from "context wasn't yours".
  async function del(userId, contextId, key) {
    const keyErr = validateKey(key);
    if (keyErr) return { ok: false, error: keyErr };
    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    const result = await soma.invokePort("postgres", "query", {
      sql:
        `DELETE FROM context_kv ` +
        `WHERE context_id = $1::text::uuid AND key = $2 ` +
        `RETURNING id`,
      params: [contextId, key],
    });
    return { ok: true, deleted: !!result.rows?.[0] };
  }

  // ---- list ----
  // Optional prefix filter. No LIMIT for now — if a context grows
  // thousands of keys a future version can paginate. The browser
  // UI shows counts first anyway.
  async function list(userId, contextId, prefix) {
    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    let result;
    if (typeof prefix === "string" && prefix !== "") {
      result = await soma.invokePort("postgres", "query", {
        sql:
          `SELECT id, key, value, ` +
          `  to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at, ` +
          `  to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at ` +
          `FROM context_kv ` +
          `WHERE context_id = $1::text::uuid AND key LIKE $2 ` +
          `ORDER BY updated_at DESC, id DESC`,
        params: [contextId, `${prefix}%`],
      });
    } else {
      result = await soma.invokePort("postgres", "query", {
        sql:
          `SELECT id, key, value, ` +
          `  to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at, ` +
          `  to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at ` +
          `FROM context_kv ` +
          `WHERE context_id = $1::text::uuid ` +
          `ORDER BY updated_at DESC, id DESC`,
        params: [contextId],
      });
    }
    return { ok: true, rows: result.rows ?? [] };
  }

  return { set, get, del, list, assertOwnership };
}
