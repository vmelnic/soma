#!/bin/bash
# Synthesize a Mind with all plugin training data + HelperBook domain data
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SOMA_DIR="$(dirname "$PROJECT_DIR")"

# Check if soma-synthesizer is installed
if ! command -v soma-synthesize &> /dev/null; then
    echo "soma-synthesize not found. Installing..."
    cd "$SOMA_DIR/soma-synthesizer"
    pip install -e .
fi

echo "Validating training data..."
soma-synthesize validate --plugins "$SOMA_DIR/soma-plugins"

echo ""
echo "Training Mind with all plugins + domain data..."
soma-synthesize train \
    --plugins "$SOMA_DIR/soma-plugins" \
    --domain "$PROJECT_DIR/domain/helperbook-training.json" \
    --output "$SOMA_DIR/models" \
    "$@"

echo ""
echo "Synthesis complete. Models at: $SOMA_DIR/models/"
ls -lh "$SOMA_DIR/models/"
