#!/usr/bin/env bash
# End-to-end proof: soma-next loads the hello MCP port, discovers capabilities
# via tools/list, and invokes greet + reverse through the full Port pipeline.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
node "$SCRIPT_DIR/../mcp-client.mjs" smoke
