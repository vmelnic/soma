-- soma-project-terminal schema
--
-- Tables:
--   users         — magic-link identity, one row per operator
--   magic_tokens  — short-lived auth tokens (sha256 at rest)
--   sessions      — long-lived session tokens (sha256 at rest)
--   contexts      — the operator's conversation scopes / "projects"
--   messages      — chat transcript per context (user + assistant)
--
-- All auth tokens are stored as sha256 hashes, never plaintext. The
-- raw token is what we email to the user and what the browser holds
-- as a cookie; the server looks up by hash so a database leak
-- doesn't compromise live sessions.
--
-- Anything related to pack generation, skills, memory (episodes /
-- schemas / routines), context_kv, or pack_spec has been removed.
-- The terminal's architecture is now "one master pack maintained as
-- code, contexts are conversation scopes against it, the LLM brain
-- uses tool calling against soma-next's MCP catalog, no per-context
-- LLM-generated artifacts". See docs/terminal-multi-tenancy.md for
-- the tradeoff analysis behind this direction.

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ------------------------------------------------------------------
-- users: magic-link identity, no password hash ever.
-- ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email       TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login  TIMESTAMPTZ
);

-- ------------------------------------------------------------------
-- magic_tokens: one-time login tokens, sha256(plaintext) stored.
-- 15-minute expiry by default. `used_at` marks consumed tokens so
-- a replay attack on a stolen hash is rejected.
-- ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS magic_tokens (
    token_hash  TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS magic_tokens_email_idx ON magic_tokens (email);
CREATE INDEX IF NOT EXISTS magic_tokens_expires_idx ON magic_tokens (expires_at);

-- ------------------------------------------------------------------
-- sessions: long-lived session tokens issued after magic-link verify.
-- Same hash-at-rest model. `revoked_at` lets us invalidate sessions
-- without a DELETE (keeps an audit trail).
-- ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS sessions (
    token_hash  TEXT PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    revoked_at  TIMESTAMPTZ,
    user_agent  TEXT
);

CREATE INDEX IF NOT EXISTS sessions_user_id_idx ON sessions (user_id);
CREATE INDEX IF NOT EXISTS sessions_expires_idx ON sessions (expires_at);

-- ------------------------------------------------------------------
-- contexts: the operator's "projects", each one a named conversation
-- scope. No pack_spec — the terminal runs one master pack in the
-- backend and every context shares it. A context is just an id + a
-- name + a description; data isolation happens at tool-invocation
-- time via the context_id.
-- ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS contexts (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,
    kind        TEXT NOT NULL DEFAULT 'active',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS contexts_user_id_idx ON contexts (user_id);
CREATE INDEX IF NOT EXISTS contexts_user_updated_idx
    ON contexts (user_id, updated_at DESC);

-- One-shot cleanup for DBs that still have the old pack_spec column.
-- Idempotent — no-op if already dropped.
ALTER TABLE contexts DROP COLUMN IF EXISTS pack_spec;

-- The conversation-first schema has no "draft" concept — every
-- context is immediately usable because chat is the interface and
-- the brain composes its own capabilities via tool calls. Force
-- the default and backfill any rows still carrying the old value.
ALTER TABLE contexts ALTER COLUMN kind SET DEFAULT 'active';
UPDATE contexts SET kind = 'active' WHERE kind = 'draft';

-- ------------------------------------------------------------------
-- messages: chat transcript per context. Each turn is one row —
-- user prompts and assistant replies land here in insertion order.
-- Tool calls and their results happen inside the chat-turn loop and
-- are NOT stored as messages; the transcript the operator sees is
-- pure conversation.
--
-- ON DELETE CASCADE: deleting a context wipes its transcript with
-- it. No orphan messages.
-- ------------------------------------------------------------------
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
-- One-shot cleanup for obsolete tables from earlier commits. Drops
-- the per-context memory tiers and the context_kv store that were
-- replaced by direct tool calling against soma-next's MCP catalog.
-- Idempotent.
-- ------------------------------------------------------------------
DROP TABLE IF EXISTS context_kv;
DROP TABLE IF EXISTS routines;
DROP TABLE IF EXISTS schemas;
DROP TABLE IF EXISTS episodes;
