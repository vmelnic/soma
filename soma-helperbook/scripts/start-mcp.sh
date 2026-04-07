#!/bin/bash
# Start HelperBook SOMA in MCP mode (for LLM connection)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SOMA_DIR="$(dirname "$PROJECT_DIR")"

export SOMA_PG_PASSWORD=soma

cd "$SOMA_DIR/soma-core"
cargo run --release --bin soma -- \
    --config "$PROJECT_DIR/soma.toml" \
    --model "$SOMA_DIR/models" \
    --mcp \
    "$@"
