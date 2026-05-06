#!/usr/bin/env bash
# Snapshot SOMA content into ingest/ for kNN-LM falsification.
# Copies files; does not symlink. Run from repo root or anywhere — paths are absolute.

set -euo pipefail
cd "$(dirname "$0")/.."

REPO_ROOT="$(cd ../.. && pwd)"
DEST_CODE="ingest/soma-codebase"
DEST_DOCS="ingest/soma-design-docs"
DEST_HOLDOUT="ingest/holdout"

mkdir -p "$DEST_CODE" "$DEST_DOCS" "$DEST_HOLDOUT"

# Indexed code
for f in \
    soma-next/src/runtime/goal_executor.rs \
    soma-next/src/runtime/session.rs \
    soma-next/src/interfaces/mcp.rs \
    soma-next/src/memory/working.rs \
    soma-next/src/memory/routines.rs \
    soma-next/src/types/routine.rs \
    soma-next/src/types/session.rs
do
    flat="$(echo "$f" | sed 's|/|__|g')"
    cp "$REPO_ROOT/$f" "$DEST_CODE/$flat"
done

# Indexed docs
for f in \
    docs/architecture.md \
    docs/vision.md \
    docs/ports.md \
    docs/compiled-routines-as-products.md \
    docs/native-brain.md \
    docs/ltc-sdm.md \
    docs/qwen-sdm.md \
    docs/qwen-knn.md \
    docs/neuroscience-architecture.md \
    CLAUDE.md
do
    [ -e "$REPO_ROOT/$f" ] || { echo "skip missing: $f" >&2; continue; }
    cp "$REPO_ROOT/$f" "$DEST_DOCS/$(basename "$f")"
done

# Held-out files (NOT indexed; only used by bench.py)
for f in \
    soma-next/src/runtime/goal_registry.rs \
    soma-next/src/bootstrap.rs \
    docs/embodied-program-synthesis.md \
    docs/context-os-proof.md
do
    [ -e "$REPO_ROOT/$f" ] || { echo "skip missing: $f" >&2; continue; }
    cp "$REPO_ROOT/$f" "$DEST_HOLDOUT/$(basename "$f")"
done

echo "snapshot complete:"
echo "  code:    $(ls "$DEST_CODE" | wc -l | tr -d ' ') files"
echo "  docs:    $(ls "$DEST_DOCS" | wc -l | tr -d ' ') files"
echo "  holdout: $(ls "$DEST_HOLDOUT" | wc -l | tr -d ' ') files"
