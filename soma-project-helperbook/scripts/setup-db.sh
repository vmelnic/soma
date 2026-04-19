#!/bin/bash
# Apply HelperBook schema to PostgreSQL
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "Ensuring PostgreSQL is running..."
cd "$PROJECT_DIR"
docker compose up -d --wait postgres

echo "Applying schema..."
cat "$PROJECT_DIR/schema.sql" | docker exec -i soma-project-helperbook-postgres-1 psql -U soma -d helperbook

echo "Schema applied successfully."
echo "Tables created:"
docker exec soma-project-helperbook-postgres-1 psql -U soma -d helperbook -c "\dt" 2>/dev/null | grep -c "public" || echo "(check manually)"
