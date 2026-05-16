# soma-project-interceptor

Autonomous drone-vs-drone defense system using SOMA runtime.

## Purpose

Protect Ukrainian airspace by intercepting hostile drones autonomously. Each engagement makes the fleet smarter — learned intercept tactics propagate to all units via ground station consolidation.

## Architecture

```
Sensors → SOMA Runtime (10ms loop) → Navigation/Engagement
              ↓
         Episodes (stored onboard)
              ↓ (post-flight upload)
         Ground Station
              ↓ (consolidation)
         Compiled Routines
              ↓ (validated push)
         Fleet Update
```

## Key Properties

- **Safety is hardwired, not learned.** IFF gate, geofence, abort — policy-enforced, cannot be overridden.
- **Tactics are learned.** Intercept geometries improve from episodes. Fleet gets smarter after each engagement.
- **No cloud in the loop.** All in-flight decisions are local. <10ms reactive loop.
- **Graceful degradation.** GPS denied, comms lost, sensors failed — abort and RTB.

## Packs

- `packs/interceptor/` — capabilities (sensors, nav, engagement, comms), schemas, authored routines
- `packs/dna/` — innate reflexes: threat_detect, engage, abort, evade, target_lost

## DNA Reflexes

| Reflex | Priority | Behavior |
|---|---|---|
| `dna.abort` | 25 | Disarm + disengage + RTB (HIGHEST — overrides everything) |
| `dna.evade` | 22 | Defensive maneuver if self is targeted |
| `dna.threat_detect` | 18 | Fuse sensors + IFF + share with swarm |
| `dna.engage` | 16 | Deliberation: select intercept strategy (compiled after learning) |
| `dna.target_lost` | 14 | Deliberation: search pattern or abort |

## Development

```bash
# Simulation mode (no hardware required)
cd soma-project-interceptor
# Copy soma-next binary
cp ../soma-next/target/release/soma bin/
# Run with simulation ports
bin/soma --config soma.toml
```

## Status

Phase: Architecture + manifest design. Next: simulation ports, scenario runner, initial learning validation.
