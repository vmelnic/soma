#!/bin/bash
# Start HelperBook SOMA
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SOMA_DIR="$(dirname "$PROJECT_DIR")"

# Ensure services are running
echo "Starting services..."
cd "$PROJECT_DIR"
docker compose up -d --wait

# Set env vars
export SOMA_PG_PASSWORD=soma

# Run SOMA
echo "Starting SOMA HelperBook..."
cd "$SOMA_DIR/soma-core"
cargo run --release --bin soma -- \
    --config "$PROJECT_DIR/soma.toml" \
    --model "$SOMA_DIR/models" \
    "$@"
