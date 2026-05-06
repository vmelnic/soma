#!/bin/bash
set -e
cd "$(dirname "$0")/.."

echo "Starting services..."
docker compose up -d --wait

echo "Applying schema..."
docker exec -i soma-builder-helperbook-postgres-1 psql -U soma -d helperbook < schema.sql

echo "Seeding data..."
docker exec -i soma-builder-helperbook-postgres-1 psql -U soma -d helperbook < seed.sql

echo "Done. Database ready."
