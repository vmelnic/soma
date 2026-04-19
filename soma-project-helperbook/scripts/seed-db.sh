#!/bin/bash
# Seed HelperBook database with test data
# Usage: ./scripts/seed.sh [--reset]
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

# Ensure services are running
docker compose up -d --wait postgres

if [ "$1" = "--reset" ]; then
    echo "Resetting database..."
    docker exec soma-project-helperbook-postgres-1 psql -U soma -d helperbook -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
    echo "Applying schema..."
    cat schema.sql | docker exec -i soma-project-helperbook-postgres-1 psql -U soma -d helperbook
fi

echo "Seeding data..."
cat seed.sql | docker exec -i soma-project-helperbook-postgres-1 psql -U soma -d helperbook

echo "Done. Verifying..."
docker exec soma-project-helperbook-postgres-1 psql -U soma -d helperbook -c "SELECT 'users: ' || COUNT(*) FROM users UNION ALL SELECT 'messages: ' || COUNT(*) FROM messages UNION ALL SELECT 'appointments: ' || COUNT(*) FROM appointments UNION ALL SELECT 'reviews: ' || COUNT(*) FROM reviews;"
