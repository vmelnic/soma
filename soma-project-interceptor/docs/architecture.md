# Interceptor Architecture

## Mission

Autonomous drone-vs-drone defense. Detect, classify, track, and neutralize hostile UAVs protecting Ukrainian airspace.

## Design Principles

1. **Safety is not learned, it's hardwired.** IFF gate, geofence, altitude floor, abort-on-failure — these are policies, not routines. They cannot be overridden by compilation.
2. **Engagement is learned.** Intercept geometries, pursuit curves, timing — these improve from episodes. The fleet gets smarter after each engagement.
3. **No brain in the loop during terminal phase.** All engagement decisions come from compiled routines or DNA reflexes. <10ms loop. No LLM, no cloud, no latency.
4. **Brain is for training, not flying.** Ground station runs consolidation: episodes → schemas → routines. New routines validated by humans before fleet push.
5. **Degrade gracefully.** GPS denied → dead reckoning. Comms lost → autonomous with timeout abort. Sensor failure → abort and RTB.

## Execution Phases

```
LOITER → DETECT → CLASSIFY → TRACK → INTERCEPT → TERMINAL → POST
  ↑                                                            |
  └────────────── abort/miss → loiter ─────────────────────────┘
```

### LOITER
- Orbit at patrol altitude
- Continuous sensor scan (RF + visual + acoustic)
- Beacon status to ground station every 5s
- Receive tasking updates from ground

### DETECT
- Sensor trigger: RF signature, visual blob, acoustic match
- DNA routine `dna.threat_detect` fires immediately
- Multi-sensor fusion for position estimate
- Share detection with swarm peers

### CLASSIFY
- IFF query (cooperative: transponder; non-cooperative: RF fingerprint + visual match)
- If friendly → discard, resume loiter
- If unknown → track, request ground classification
- If hostile → confirm, proceed to TRACK

### TRACK
- Continuous sensor fusion: predict target state (position, velocity, acceleration)
- Compute closure geometry
- Select intercept strategy:
  - Non-maneuvering target → proportional navigation
  - Evasive target → lead pursuit with prediction
  - High-speed target → pure pursuit with max throttle

### INTERCEPT
- Compiled routine executes approach
- Continuous target state update (10ms loop)
- Arm warhead at safe distance
- Adjust pursuit curve based on target maneuvers

### TERMINAL
- Final 50m: no more course corrections possible at speed
- Proximity detonation trigger
- If miss: log miss distance + target state at closest approach

### POST
- Upload episode to ground station (via mesh relay or post-recovery)
- Episode includes: full sensor log, decisions made, outcome, target behavior
- Ground station mines episodes for improved routines

## DNA Routines (Innate Reflexes)

| Routine | Trigger | Action | Priority |
|---|---|---|---|
| `dna.threat_detect` | Any sensor trigger | Fuse + classify + share | 18 |
| `dna.engage` | Confirmed hostile | Deliberation (brain selects intercept strategy) | 16 |
| `dna.abort` | Safety violation | Disarm + disengage + RTB | 25 (highest) |
| `dna.evade` | Inbound threat to self | Defensive maneuver | 22 |
| `dna.target_lost` | Tracking lost | Deliberation (search pattern or abort) | 14 |

Key design: `dna.abort` has highest priority and is EXCLUSIVE — it overrides any other routine immediately.

`dna.engage` and `dna.target_lost` have EMPTY steps (deliberation mode) — the brain/ground decides what strategy to use. After enough successful intercepts, compiled routines replace deliberation with fast-path execution.

## Safety Policies (Non-negotiable)

These are enforced by the policy engine at the port level — no routine can bypass them:

1. **IFF Gate:** `engagement.*` skills require `iff.hostile == true`. Hard block.
2. **Geofence:** `navigation.*` skills constrained to engagement zone polygon. Hard block.
3. **Altitude Floor:** Cannot descend below MIN_SAFE_ALT except during armed terminal phase with target lock.
4. **Abort on Failure:** Sensor loss, comms timeout, battery critical → immediate disarm + RTB.
5. **Friendly Fire Prevention:** If IFF changes to friendly/uncertain during engagement → immediate abort.

## Learning Pipeline

```
In-flight:                          Ground station:
  Sensor data ──┐                     Episodes ──→ PrefixSpan
  Decisions ────┼──→ Episode            ↓
  Outcome ──────┘    (stored local)   Schemas ──→ BMR gate
                                        ↓
                    Fleet ←────────── Compiled routines
                    (after human validation)
```

What gets learned:
- Optimal intercept angles for target types (fixed-wing vs multirotor vs glide)
- Effective pursuit curves at different closure rates
- When to switch from proportional nav to lead pursuit
- Target maneuver prediction from initial trajectory

What NEVER gets learned (hardwired):
- IFF classification rules
- Abort conditions
- Geofence boundaries
- Engagement authorization

## Hardware Target

### Flight Controller
- STM32H7 or ESP32-S3 (SOMA already proven on ESP32)
- 10ms control loop
- Local episode storage (flash, ~50 episodes)

### Sensors
- RF scanner (detect drone control signals: 2.4GHz, 5.8GHz, 900MHz)
- Visual camera (blob detection, target tracking)
- IR camera (thermal signature, works at night)
- Acoustic array (propeller signature detection)
- IMU + GPS (own state estimation)

### Comms
- Mesh radio (LoRa or custom) for swarm coordination
- Telemetry downlink to ground station
- Peer-to-peer target sharing

### Engagement
- Proximity fuze (radar or optical)
- Fragmentation warhead (small radius, optimized for drone targets)
- Safe/arm logic with hardware interlocks (not software-only)

## Swarm Coordination

Multiple interceptors share:
- Target detections (first detector shares with all)
- Target assignments (ground station deconflicts, or autonomous round-robin)
- Engagement results (miss reports help others adjust)

NOT shared in-flight:
- Routines (only pushed from ground after validation)
- Learning updates (only post-flight)

## Simulation Environment

For development and testing before hardware:
- `sensor_mode = "simulation"`: synthetic targets with configurable behavior
- `nav_mode = "simulation"`: 6DOF physics model
- `engagement_mode = "simulation"`: proximity detection with miss distance calculation
- Scenario files define target types, flight patterns, quantities
- Monte Carlo runs for routine validation before fleet push

## Metrics

| Metric | Target |
|---|---|
| Detection-to-engagement time | <5s |
| Intercept success rate | >70% (improving with learning) |
| False engagement rate | 0% (IFF gate) |
| Friendly fire incidents | 0 (policy-enforced) |
| Abort response time | <50ms |
| Fleet routine update cycle | <24h from engagement to fleet push |
