// Per-context memory — SOMA-native, fully isolated by context.
//
// Three storage tiers mirrored from docs/architecture.md:
//   episodes → schemas (via PrefixSpan) → routines (compiled).
// Each tier has its own table with a `context_id` FK. Every read
// or write joins contexts on user_id so cross-tenant probes see
// "not found", same shape as genuinely unknown context ids — same
// pattern used by backend/messages.mjs.
//
// Memory flows FROM the runtime TO the store. Commit 5 exposes the
// storage layer; producing episodes from live runtime activity is
// a later concern (the soma-next native control loop handles it
// today, the wasm path will catch up when the "organic multi-step
// episode" selector lands — see docs/memory-fusion.md).
//
// For commit 5, callers (tests + any future wasm export bridge)
// push payloads as JSON strings or objects. Validation happens
// here: the payload MUST be parseable JSON. Any further shape
// check lives on the runtime side where the wasm owns the types.

const CATEGORIES = Object.freeze({
  episodes: {
    table: "episodes",
    requiresName: false,
  },
  schemas: {
    table: "schemas",
    requiresName: true,
  },
  routines: {
    table: "routines",
    requiresName: true,
  },
});

const PAYLOAD_MAX = 100_000; // 100 KB — generous, caps runaway tests
const NAME_MAX = 200;

function looksLikeUuid(s) {
  return (
    typeof s === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)
  );
}

function normalizePayload(input) {
  if (typeof input === "string") {
    const text = input;
    if (text.length > PAYLOAD_MAX) {
      return { ok: false, error: "payload too large" };
    }
    try {
      JSON.parse(text);
    } catch (e) {
      return { ok: false, error: `invalid JSON: ${e.message}` };
    }
    return { ok: true, text };
  }
  if (input && typeof input === "object") {
    const text = JSON.stringify(input);
    if (text.length > PAYLOAD_MAX) {
      return { ok: false, error: "payload too large" };
    }
    return { ok: true, text };
  }
  return { ok: false, error: "payload must be an object or JSON string" };
}

export function createMemory(soma) {
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

  // ---- list all three tiers at once ----
  // One round trip per table. Could be a UNION ALL but the shape
  // difference (schemas + routines have `name`) makes the merge
  // ugly. Three small SELECTs are fine for commit 5 — indices cover
  // every query on `(context_id, created_at)`.
  async function listMemory(userId, contextId) {
    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    const [episodes, schemas, routines] = await Promise.all([
      soma.invokePort("postgres", "query", {
        sql:
          `SELECT id, payload, ` +
          `       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at ` +
          `FROM episodes WHERE context_id = $1::text::uuid ` +
          `ORDER BY created_at ASC, id ASC`,
        params: [contextId],
      }),
      soma.invokePort("postgres", "query", {
        sql:
          `SELECT id, name, payload, ` +
          `       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at ` +
          `FROM schemas WHERE context_id = $1::text::uuid ` +
          `ORDER BY created_at ASC, id ASC`,
        params: [contextId],
      }),
      soma.invokePort("postgres", "query", {
        sql:
          `SELECT id, name, payload, ` +
          `       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at ` +
          `FROM routines WHERE context_id = $1::text::uuid ` +
          `ORDER BY created_at ASC, id ASC`,
        params: [contextId],
      }),
    ]);

    return {
      ok: true,
      memory: {
        episodes: episodes.rows ?? [],
        schemas: schemas.rows ?? [],
        routines: routines.rows ?? [],
      },
    };
  }

  // ---- generic append helper ----
  // `category` is one of the keys of CATEGORIES. `opts` may carry
  // a `name` (required for schemas/routines) and a `payload`.
  async function appendRow(userId, contextId, category, opts) {
    const cfg = CATEGORIES[category];
    if (!cfg) return { ok: false, error: "unknown memory category" };

    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    const payloadResult = normalizePayload(opts?.payload);
    if (!payloadResult.ok) return payloadResult;
    const payloadText = payloadResult.text;

    let name = null;
    if (cfg.requiresName) {
      name = typeof opts?.name === "string" ? opts.name.trim() : "";
      if (name === "") return { ok: false, error: "name is required" };
      if (name.length > NAME_MAX) {
        return { ok: false, error: "name too long" };
      }
    }

    // Two different INSERT shapes depending on whether the category
    // has a name column. Both return the row + formatted timestamp
    // so the caller can push it into the UI without a second fetch.
    let result;
    if (cfg.requiresName) {
      result = await soma.invokePort("postgres", "query", {
        sql:
          `INSERT INTO ${cfg.table} (context_id, name, payload) ` +
          `VALUES ($1::text::uuid, $2, $3) ` +
          `RETURNING id, name, payload, ` +
          `  to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at`,
        params: [contextId, name, payloadText],
      });
    } else {
      result = await soma.invokePort("postgres", "query", {
        sql:
          `INSERT INTO ${cfg.table} (context_id, payload) ` +
          `VALUES ($1::text::uuid, $2) ` +
          `RETURNING id, payload, ` +
          `  to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at`,
        params: [contextId, payloadText],
      });
    }
    const row = result.rows?.[0];
    if (!row) return { ok: false, error: "failed to append" };
    return { ok: true, row };
  }

  async function appendEpisode(userId, contextId, payload) {
    return appendRow(userId, contextId, "episodes", { payload });
  }

  async function appendSchema(userId, contextId, name, payload) {
    return appendRow(userId, contextId, "schemas", { name, payload });
  }

  async function appendRoutine(userId, contextId, name, payload) {
    return appendRow(userId, contextId, "routines", { name, payload });
  }

  // ---- clear all memory for a context ----
  // Three DELETEs inside a single ownership gate. No transaction
  // boundary because the postgres port runs each call on its own
  // connection and the ordering is not safety-critical for the
  // caller — a partial failure leaves a smaller memory, never a
  // cross-context mixture.
  async function clearMemory(userId, contextId) {
    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    for (const table of ["episodes", "schemas", "routines"]) {
      await soma.invokePort("postgres", "execute", {
        sql: `DELETE FROM ${table} WHERE context_id = $1::text::uuid`,
        params: [contextId],
      });
    }
    return { ok: true };
  }

  return {
    assertOwnership,
    listMemory,
    appendEpisode,
    appendSchema,
    appendRoutine,
    clearMemory,
  };
}
