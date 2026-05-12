#!/usr/bin/env node
/**
 * consolidate.js — CLI entry point for the episode→schema→routine pipeline.
 *
 * Usage:
 *   node src/consolidate.js
 */

import { consolidate } from "./routines.js";

function main() {
  console.log("[consolidate] Starting episode consolidation...\n");
  const result = consolidate();
  console.log(`\n[consolidate] Done: ${result.episodes} episodes → ${result.schemas} schemas → ${result.routines} routines (${result.rejected || 0} rejected)`);
}

main();
