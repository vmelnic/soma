#!/bin/bash
set -e
cd "$(dirname "$0")/.."
echo "Seeding data..."
docker exec -i soma-builder-helperbook-postgres-1 psql -U soma -d helperbook < seed.sql
echo "Done."
