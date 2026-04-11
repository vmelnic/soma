-- soma-project-terminal schema
--
-- Commit 1 tables: users, sessions, magic_tokens.
-- Commit 2 tables: contexts.
-- Commit 3 tables: messages.
-- Commit 4 columns: contexts.pack_spec (nullable TEXT, JSON-encoded).
-- Commit 5 tables: episodes, schemas, routines — per-context memory.
-- Commit 8 tables: context_kv — per-context key/value store exposed
--                  to the browser via the backend-port bridge.
--
-- Tokens are stored as sha256 hashes, never plaintext. The raw token
-- is what we email to the user and what the browser holds as a cookie;
-- the server looks up by hash so a database leak doesn't compromise
-- live sessions.

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Users. Magic-link auth means we don't store a password hash at all.
-- A user is created on first successful magic-link verification.
CREATE TABLE IF NOT EXISTS users (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email       TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login  TIMESTAMPTZ
);

-- One-time magic tokens. sha256(plaintext) stored. Each email→link
-- request creates a row with a 15-minute expiry. `used_at` is set
-- when the link is clicked; expired or used hashes are refused.
CREATE TABLE IF NOT EXISTS magic_tokens (
    token_hash  TEXT PRIMARY KEY,           -- sha256 hex of the raw token
    email       TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS magic_tokens_email_idx ON magic_tokens (email);
CREATE INDEX IF NOT EXISTS magic_tokens_expires_idx ON magic_tokens (expires_at);

-- Long-lived session tokens issued after magic-link verification.
-- Same hash-at-rest model as magic_tokens. `revoked_at` lets us
-- invalidate sessions without a DELETE (keeps an audit trail).
CREATE TABLE IF NOT EXISTS sessions (
    token_hash  TEXT PRIMARY KEY,           -- sha256 hex of the raw session token
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    revoked_at  TIMESTAMPTZ,
    user_agent  TEXT
);

CREATE INDEX IF NOT EXISTS sessions_user_id_idx ON sessions (user_id);
CREATE INDEX IF NOT EXISTS sessions_expires_idx ON sessions (expires_at);

-- Contexts — the user's "projects". In commit 2 a context is just a
-- named row owned by a user. In commit 6 a context grows a PackSpec
-- column compiled from the user's natural-language description, and
-- commit 4 loads that pack into a browser-side soma-next runtime.
--
-- `kind` is a cheap enum-ish discriminator: 'draft' is the initial
-- state before a pack has been generated, 'active' means the pack
-- compiled and the context is usable, 'archived' hides it from the
-- default listing. Using TEXT rather than a real enum type keeps
-- schema migrations trivial and matches how the postgres port
-- serializes all parameters as TEXT anyway.
CREATE TABLE IF NOT EXISTS contexts (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,
    kind        TEXT NOT NULL DEFAULT 'draft',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS contexts_user_id_idx ON contexts (user_id);
CREATE INDEX IF NOT EXISTS contexts_user_updated_idx
    ON contexts (user_id, updated_at DESC);

-- Commit 4: pack_spec holds the context's compiled PackSpec as a
-- JSON-encoded string. NULL means "no pack yet" — the browser boots
-- the shared hello pack as a fallback. Commit 6 will populate this
-- column from the reasoning-brain's output. Stored as TEXT (rather
-- than JSONB) because the wasm entry point wants the raw string
-- back out; re-parsing a JSONB round trip would lose nothing but
-- burn an extra JSON encode per boot.
ALTER TABLE contexts ADD COLUMN IF NOT EXISTS pack_spec TEXT;

-- Chat history per context. Each turn is one row — user prompts and
-- assistant replies are both stored here in insertion order. Commit 6
-- will add a second role `brain` for structured pack-generation
-- turns (same table, different role value) so all context-local
-- reasoning ends up on a single timeline.
--
-- `role` is TEXT rather than a CHECK constraint so adding new roles
-- (brain, tool, system-note) in later commits is a pure data change.
--
-- ON DELETE CASCADE: deleting a context wipes its transcript with it.
-- No orphan messages, no leak of deleted context content.
CREATE TABLE IF NOT EXISTS messages (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    context_id  UUID NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS messages_context_created_idx
    ON messages (context_id, created_at);

-- ------------------------------------------------------------------
-- Commit 5: per-context memory (episodes / schemas / routines).
--
-- These are the three consolidation tiers from docs/architecture.md:
--   episodes → schemas (via PrefixSpan) → routines (compiled).
-- Each tier has its own table, each row is scoped to one context,
-- and `payload` holds the raw JSON blob shaped by the runtime layer.
-- Storing as TEXT (not JSONB) keeps us schema-free so the runtime
-- can evolve its episode/schema/routine shapes without ALTERs.
--
-- Ownership travels exclusively via context_id → contexts.user_id.
-- Every SELECT joins contexts so another operator's probe resolves
-- to zero rows, same "not found" shape as an unknown context.
-- ON DELETE CASCADE drops the rows with their parent context.
-- ------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS episodes (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    context_id  UUID NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
    payload     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS episodes_context_created_idx
    ON episodes (context_id, created_at);

CREATE TABLE IF NOT EXISTS schemas (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    context_id  UUID NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    payload     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS schemas_context_created_idx
    ON schemas (context_id, created_at);

CREATE TABLE IF NOT EXISTS routines (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    context_id  UUID NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    payload     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS routines_context_created_idx
    ON routines (context_id, created_at);

-- ------------------------------------------------------------------
-- Commit 8: backend-port bridge — per-context key/value store.
--
-- The browser wasm runtime can't persist anything on its own (no
-- storage port yet). Instead of building a browser-native one, we
-- expose a tiny KV space backed by postgres and accessible via a
-- session-authed + context-scoped HTTP bridge. Generated packs
-- can declare `context_kv.{set,get,delete,list}` capabilities and
-- the JS-side executor routes those calls via fetch instead of
-- soma_invoke_port.
--
-- UNIQUE (context_id, key) so `set` is an upsert and `get` is a
-- single row lookup. `value` is plain TEXT — callers serialize
-- however they want (JSON, base64, free text).
-- ------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS context_kv (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    context_id  UUID NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (context_id, key)
);

CREATE INDEX IF NOT EXISTS context_kv_context_idx
    ON context_kv (context_id);
CREATE INDEX IF NOT EXISTS context_kv_context_key_idx
    ON context_kv (context_id, key);
