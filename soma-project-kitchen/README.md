# soma-project-kitchen

Tiny Kitchen — SOMA learns to organize a kitchen countertop, demonstrating all 10 Meta-World ML10 manipulation tasks.

## What it proves

A single SOMA routine covers all 10 ML10 manipulation primitives in one coherent scenario:

| ML10 Task | Kitchen Action |
|---|---|
| reach | Move arm to target position |
| push | Push cutting board to counter center |
| pick-place | Pick spice jar, place on cabinet shelf |
| door-open | Open cabinet door |
| drawer-close | Close utensil drawer |
| drawer-open | Open utensil drawer |
| button-press | Press food processor button |
| peg-insert-side | Insert knife into block slot |
| window-open | Slide kitchen window open |
| window-close | Slide kitchen window closed |

## Web editor

```bash
cd editor && npm install && node server.js
# http://localhost:3334
```

Run SOMA on the kitchen scenario, watch animated step-by-step execution. Controls:

- **Seed** — vary item positions
- **Play** — run SOMA on the scenario
- **Speed** — animation speed
- **Flush Memory** — clear learned state (`data/`)

## CLI

```bash
cargo run --release -- scenarios/organize.json
```

## Build

```bash
cd ../soma-ports && cargo build --release -p soma-port-kitchen
cp ../soma-ports/target/release/libsoma_port_kitchen.dylib packs/kitchen/
cargo build --release
```

## Structure

```
scenarios/*.json           <- scenario definitions
packs/kitchen/             <- pack manifest + kitchen cdylib
src/lib.rs                 <- module declarations
src/world.rs               <- ScenarioSpec loader
src/session.rs             <- session runner + episode builder
src/run.rs                 <- run-kitchen binary
editor/                    <- web editor (Express + vanilla JS)
data/                      <- persisted SOMA state (gitignored)
output/                    <- generated traces (gitignored)
```

## Port capabilities

| Capability | What it does | ML10 task |
|---|---|---|
| `reset` | Create kitchen from scenario config | - |
| `scan` | Observe kitchen state | reach |
| `push_board` | Push cutting board to center | push |
| `pick_jar` | Pick up nearest spice jar | pick-place (part 1) |
| `pick_knife` | Pick up knife | - |
| `place_shelf` | Place item on cabinet shelf | pick-place (part 2) |
| `place_counter` | Place item on counter | pick-place (alt) |
| `door_open` | Open cabinet door | door-open |
| `door_close` | Close cabinet door | - |
| `drawer_open` | Open drawer | drawer-open |
| `drawer_close` | Close drawer | drawer-close |
| `button_press` | Press food processor | button-press |
| `peg_insert` | Insert knife into block | peg-insert-side |
| `window_open` | Slide window open | window-open |
| `window_close` | Slide window closed | window-close |
