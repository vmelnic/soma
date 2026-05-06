#!/bin/bash
set -e
cd "$(dirname "$0")/.."
echo "Resetting database..."
docker exec -i soma-builder-helperbook-postgres-1 psql -U soma -d helperbook -c "
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
GRANT ALL ON SCHEMA public TO soma;
"
echo "Applying schema..."
docker exec -i soma-builder-helperbook-postgres-1 psql -U soma -d helperbook < schema.sql
echo "Seeding data..."
docker exec -i soma-builder-helperbook-postgres-1 psql -U soma -d helperbook < seed.sql
echo "Done. Database reset."
