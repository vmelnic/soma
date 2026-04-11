#!/usr/bin/env bash
# Wipe all rows from users, sessions, magic_tokens. Schema stays.
set -euo pipefail
docker exec -i soma-terminal-postgres psql -U soma -d soma_terminal <<'SQL'
TRUNCATE TABLE sessions, magic_tokens, users RESTART IDENTITY CASCADE;
SQL
printf '[clean-db] tables truncated\n'
