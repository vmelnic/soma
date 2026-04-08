#!/bin/bash
# Clean all HelperBook database tables (truncate all data, keep schema)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"
docker compose up -d --wait postgres

echo "Truncating all tables..."
docker exec soma-helperbook-postgres-1 psql -U soma -d helperbook -c "
DO \$\$
DECLARE
    r RECORD;
BEGIN
    FOR r IN (SELECT tablename FROM pg_tables WHERE schemaname = 'public') LOOP
        EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' CASCADE';
    END LOOP;
END
\$\$;
"

echo "Done. All tables empty."
