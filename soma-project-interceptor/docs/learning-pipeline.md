# Learning Pipeline

## What gets learned vs what stays hardwired

### LEARNED (improves from episodes)

- Optimal intercept angles per target type
- Pursuit curve selection (proportional nav vs lead pursuit vs pure pursuit)
- Terminal approach timing (when to arm, when to detonate)
- Target maneuver prediction (heading change patterns)
- Sensor fusion weights (which sensor to trust in which conditions)
- Speed management (when to sprint vs conserve energy)

### HARDWIRED (policy-enforced, never overridden)

- IFF classification rules
- Geofence boundaries
- Altitude floor
- Abort conditions (comms loss, sensor failure, battery critical)
- Friendly fire prevention
- Engagement authorization chain

## Episode Structure

Each engagement produces an episode:

```json
{
  "episode_id": "eng-2026-05-14-001",
  "interceptor_id": "defender_01",
  "outcome": "kill",
  "target_type": "fixed_wing",
  "target_behavior": "straight_line",
  "engagement_time_ms": 12400,
  "miss_distance_m": 0.0,
  "closure_rate_ms": 45,
  "approach_angle_deg": 15,
  "pursuit_mode": "proportional_navigation",
  "steps": [
    { "skill_id": "sensors.fuse_target_state", "duration_ms": 5, "result": "ok" },
    { "skill_id": "navigation.compute_intercept_vector", "duration_ms": 2, "result": "ok" },
    { "skill_id": "navigation.proportional_navigation", "duration_ms": 12000, "result": "ok" },
    { "skill_id": "engagement.arm", "duration_ms": 1, "result": "ok" },
    { "skill_id": "engagement.detonate_proximity", "duration_ms": 1, "result": "ok" }
  ],
  "sensor_log": "eng-2026-05-14-001.bin",
  "conditions": {
    "wind_ms": 5,
    "visibility_m": 3000,
    "gps_available": true,
    "altitude_m": 105
  }
}
```

## Consolidation (Ground Station)

Runs after episodes are uploaded:

1. **PrefixSpan** — find repeated skill sequences across successful intercepts
2. **Schema induction** — group by target type + behavior → identify optimal approach patterns
3. **BMR gate** — compile schemas into routines only if accuracy > complexity cost
4. **Human validation** — operator reviews compiled routines before fleet push
5. **Fleet update** — push validated routines to all interceptors via next ground contact

## What improves over time

### Week 1 (few engagements)
- All intercepts use deliberation (brain picks strategy)
- Miss rate: ~40% (no learned optimization)
- Engagement time: slow (deliberation adds latency)

### Week 4 (50+ engagements)
- Common target types have compiled routines
- Straight-line targets: compiled fast-path, <5s engagement
- Evasive targets: still use deliberation for maneuver prediction
- Miss rate: ~20%

### Month 3 (200+ engagements)
- All common scenarios have compiled routines
- Evasive patterns catalogued, predictions compiled
- Brain only needed for truly novel target behavior
- Miss rate: <10%
- Engagement time: minimal (pure routine execution)

## Transfer Between Target Types

When a new hostile type appears (never seen before):
1. First encounter: `dna.engage` fires with empty steps → deliberation
2. Brain selects closest-matching existing routine as starting point
3. Episode recorded with result
4. After 3-5 encounters: schema induced for new type
5. After validation: new routine compiled and pushed to fleet

The fleet adapts to new threats within days, not months.

## Swarm Learning

Each interceptor stores episodes locally. After engagement:
1. Upload episode to ground station (via mesh relay or post-recovery)
2. Ground station consolidates episodes from ALL interceptors
3. Best tactics from any single unit propagate to entire fleet
4. One unit's success = everyone's improvement

This means:
- Fleet of 100 interceptors learns 100x faster than a single unit
- Rare scenarios still accumulate enough episodes for compilation
- No single unit needs extensive combat experience
