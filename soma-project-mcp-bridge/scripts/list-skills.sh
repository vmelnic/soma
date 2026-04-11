#!/usr/bin/env bash
# Show the hello port and its discovered capabilities.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
node "$SCRIPT_DIR/../mcp-client.mjs" list_ports
