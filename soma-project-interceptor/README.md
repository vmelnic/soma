# soma-project-interceptor

Autonomous drone-vs-drone defense system using SOMA runtime. 8/8 proof phases pass end-to-end.

## What it proves

Full autonomous learning loop on a single interceptor drone:
1. Detect hostile via multi-sensor fusion (RF, visual, IR, acoustic)
2. Classify via IFF — hardwired abort on friendly
3. Pursue with 3D proportional navigation, lead pursuit, or pure pursuit
4. Engage (arm + proximity detonate) within 5m kill radius
5. Accumulate episodes from every engagement
6. Mine common skill patterns via PrefixSpan
7. Compile learned routines through Bayesian Model Reduction gate
8. Fire learned routines reactively on next threat detection

## Architecture

```
Sensors --> SOMA Runtime (10ms loop) --> Navigation/Engagement
                 |
            Episodes (stored onboard)
                 | (post-flight upload)
            Ground Station
                 | (consolidation)
            Compiled Routines
                 | (validated push)
            Fleet Update
```

## Key Properties

- **Safety is hardwired, not learned.** IFF gate, geofence, abort — policy-enforced, cannot be overridden.
- **Tactics are learned.** Intercept geometries improve from episodes. Fleet gets smarter after each engagement.
- **No cloud in the loop.** All in-flight decisions are local. <10ms reactive loop.
- **Graceful degradation.** GPS denied, comms lost, sensors failed — abort and RTB.

## Running

```bash
cd soma-project-interceptor
cargo run --release
```

All 8 phases must pass (exit 0). No external dependencies, no hardware required.

## Proof phases

| Phase | What | Verified |
|---|---|---|
| 1 | Port invocation (reset sim, step, observe) | Port produces typed PortCallRecords |
| 2 | Sensor fusion (RF + visual + IR + acoustic) | Multi-sensor detection and target state |
| 3 | DNA reflex fires reactively | Reactive monitor triggers routine on world state change |
| 4 | Full intercept kill | PN closes to <5m, arm, detonate, kill confirmed |
| 5 | Safety guarantee | IFF=friendly triggers abort, never kills friendly |
| 6 | Episode accumulation | Multiple engagements stored for learning |
| 7 | PrefixSpan learning | Schema induction + routine compilation from episodes |
| 8 | Learned routine fires | Compiled routine executes reactively via plan-following |

## Packs

- `packs/interceptor/` — 27 capabilities (sensors, nav, engagement, comms)
- `packs/dna/` — innate reflexes: threat_detect, engage, abort, evade, target_lost

## DNA Reflexes

| Reflex | Priority | Behavior |
|---|---|---|
| `dna.abort` | 25 | Disarm + disengage + RTB (HIGHEST) |
| `dna.evade` | 22 | Defensive maneuver if self is targeted |
| `dna.threat_detect` | 18 | Fuse sensors + IFF + share with swarm |
| `dna.engage` | 16 | Select intercept strategy |
| `dna.target_lost` | 14 | Search pattern or abort |

## Learned pattern

PrefixSpan discovers the invariant 5-step skeleton across all successful engagements:
```
fuse_target_state -> iff_query -> compute_intercept_vector -> arm -> detonate_proximity
```
The pursuit method (PN/lead/pure) varies per engagement but the skeleton is stable — compiled into a routine at 0.80 confidence.

## Status

Proven in simulation. Ready for hardware integration (real sensor ports, actuator ports, flight controller interface).
