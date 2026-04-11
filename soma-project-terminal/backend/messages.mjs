// Per-context chat messages — SOMA-native.
//
// Every read/write goes through the postgres port. Ownership is
// enforced by joining contexts: a user can only touch messages for
// contexts where `contexts.user_id = current_user_id`. Attempting to
// read another user's transcript returns "not found", same shape as
// a genuinely unknown context.
//
// Roles are free-form TEXT on the schema side; the module only
// accepts "user" and "assistant" for commit 3. Commit 6 will add
// "brain" for structured pack-generation turns.

const ROLES = new Set(["user", "assistant"]);
const CONTENT_MAX = 8000; // ~2000 tokens of plain text
const MAX_HISTORY = 200;

function looksLikeUuid(s) {
  return (
    typeof s === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)
  );
}

export function createMessages(soma) {
  // Confirm the user owns the context before any read/write. This
  // keeps the ownership check in a single place so every caller
  // inherits it automatically.
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

  // ---- list ----
  async function listForContext(userId, contextId) {
    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    const result = await soma.invokePort("postgres", "query", {
      sql:
        `SELECT id, role, content, ` +
        `       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at ` +
        `FROM messages ` +
        `WHERE context_id = $1::text::uuid ` +
        `ORDER BY created_at ASC, id ASC`,
      params: [contextId],
    });
    return { ok: true, messages: result.rows ?? [] };
  }

  // ---- append ----
  // Caller must have already called assertOwnership (or we do it
  // here as a safety net — the extra query is cheap relative to a
  // brain call).
  async function append(userId, contextId, role, content) {
    if (!ROLES.has(role)) {
      return { ok: false, error: "invalid role" };
    }
    const text = typeof content === "string" ? content.trim() : "";
    if (text === "") return { ok: false, error: "empty content" };
    if (text.length > CONTENT_MAX) {
      return { ok: false, error: "content too long" };
    }
    const owned = await assertOwnership(userId, contextId);
    if (!owned) return { ok: false, error: "not found" };

    // INSERT ... RETURNING gives us the row id and created_at so the
    // UI can place the new message in the transcript without a
    // second fetch. We format created_at inline via to_char for the
    // same reason the contexts module does: the postgres port's
    // row-to-json path collapses raw timestamptz columns to null.
    const result = await soma.invokePort("postgres", "query", {
      sql:
        `INSERT INTO messages (context_id, role, content) ` +
        `VALUES ($1::text::uuid, $2, $3) ` +
        `RETURNING id, role, content, ` +
        `  to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS created_at`,
      params: [contextId, role, text],
    });
    const row = result.rows?.[0];
    if (!row) return { ok: false, error: "failed to append" };

    // Bump the context's updated_at so the sidebar sorts recent-
    // chat contexts to the top in commit 4. Fire-and-forget via
    // execute.
    await soma.invokePort("postgres", "execute", {
      sql:
        `UPDATE contexts SET updated_at = NOW() ` +
        `WHERE id = $1::text::uuid AND user_id = $2::text::uuid`,
      params: [contextId, userId],
    });

    return { ok: true, message: row };
  }

  // ---- history fetch for brain ----
  // Returns the last `MAX_HISTORY` messages as `{role, content}`
  // pairs, oldest first — the exact shape brain.runChatTurn wants
  // as its `history` argument. No timestamps; the brain doesn't
  // care.
  async function historyFor(userId, contextId) {
    const result = await listForContext(userId, contextId);
    if (!result.ok) return result;
    const rows = result.messages.slice(-MAX_HISTORY);
    return {
      ok: true,
      history: rows.map((r) => ({ role: r.role, content: r.content })),
    };
  }

  return { assertOwnership, listForContext, append, historyFor };
}
