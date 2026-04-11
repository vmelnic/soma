#!/usr/bin/env bash
# Apply schema.sql to the running postgres container.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
docker exec -i soma-terminal-postgres psql -U soma -d soma_terminal < "$PROJECT_ROOT/schema.sql"
printf '[setup-db] schema applied\n'
