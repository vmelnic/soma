#!/usr/bin/env bash
# Bring up Postgres + Redis + Mailcatcher.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"
docker compose up -d --wait
printf '[start] services up:\n'
printf '        postgres    localhost:5433\n'
printf '        redis       localhost:6380\n'
printf '        mailcatcher SMTP localhost:1025 / web http://localhost:1080\n'
