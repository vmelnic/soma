# soma-project-minigrid

Grid navigation benchmark — SOMA learns multi-step procedural patterns from execution experience and improves across runs via persisted memory.

## How it works

1. **Pack declares routine** — the manifest ships a doorkey routine (`scan → go_to_key → pickup → go_to_door → toggle → go_to_goal`). SOMA loads it at boot — no hardcoded seeds in code.
2. **Agent executes** — the runtime follows the routine via plan-following. Fog of war forces exploration when targets aren't visible.
3. **Agent remembers** — explored map is persisted to WorldState. Episodes are stored and fed back through the learning pipeline (PrefixSpan → schema induction → BMR routine compilation).
4. **Agent improves** — on repeat runs of the same world, the agent skips already-explored areas and navigates directly. Steps decrease across runs (e.g. 126 → 36 → 24 → 24 on a 25x25 grid).

## Web editor

```bash
cd editor && npm install && node server.js
# http://localhost:3333
```

Paint custom worlds, run SOMA, see animated GIF results. Controls:

- **Size** / **Fog** — grid dimensions and view radius (fog of war)
- **Save** / **Export** — persist to `worlds/` or download JSON
- **Play** — run SOMA on the current world
- **Flush Memory** — clear all learned state (`data/`)

## CLI

```bash
# Single world
cargo run --release --bin run-minigrid -- worlds/my_world.json

# All worlds in a directory
cargo run --release --bin run-minigrid -- worlds/ --output output/
```

Output per world: `<name>.gif` (animated), `<name>.trace.json` (step-by-step).

## Build

```bash
# Build the gridworld port
cd ../soma-ports && cargo build --release

# Copy to pack
cp ../soma-ports/target/release/libsoma_port_gridworld.dylib packs/minigrid/

# Build the project
cargo build --release
```

## Persistence

State lives in `data/` (gitignored):

| File | Contents |
|------|----------|
| `episodes.json` | Execution episodes (lightweight — no cell data) |
| `schemas.json` | Induced schemas from episode mining |
| `routines.json` | Compiled routines (BMR-gated) |
| `world_state.json` | Explored maps per world (spatial memory) |

Flush via the web editor button or `rm -rf data/`.

## Structure

```
worlds/*.json              ← world definitions (size, seed, or custom cells)
packs/minigrid/            ← pack manifest (schema + routine) + gridworld cdylib
src/lib.rs                 ← module declarations
src/world.rs               ← WorldSpec loader
src/session.rs             ← session runner + episode builder
src/vis.rs                 ← GIF renderer
src/run.rs                 ← run-minigrid binary
editor/                    ← web editor (Express + vanilla JS)
data/                      ← persisted SOMA state (gitignored)
output/                    ← generated GIFs and traces (gitignored)
```

## Gridworld port capabilities

| Capability | What it does |
|------------|-------------|
| `reset` | Create grid from env type, seed, or custom layout. Accepts `explored_mask` to restore prior knowledge. |
| `scan` | Return grid observation (auto-resets if needed) |
| `go_to_key` | BFS pathfind to key (no-op if no locked doors remain) |
| `pickup` | Pick up object at current position (no-op if already carrying) |
| `go_to_door` | BFS pathfind to door-adjacent cell (no-op if no locked doors) |
| `toggle` | Unlock door with key (no-op if no locked doors) |
| `go_to_goal` | BFS pathfind to goal |
| `go_to_ball` | BFS pathfind to nearest ball |
| `go_to_box` | BFS pathfind to nearest box |
| `go_to` | Navigate to named target or `[x,y]` coordinates |
| `drop` | Drop carried object |

Observations include: agent position, cell grid, carrying state, object positions, fog explored percentage, and `explored_mask` for persistence.
