-- soma-project-terminal schema
--
-- Commit 1 tables: users, sessions, magic_tokens. Commit 2 adds contexts.
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
