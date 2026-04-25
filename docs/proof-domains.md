# Proof Domains and Benchmarks

Where SOMA's learning pipeline can be tested against established baselines, with published leaderboards and reproducible comparison.

## What makes a strong proof domain for SOMA

SOMA learns procedural patterns: repeatable multi-step skill sequences where the same structure works across instances. A strong proof domain exercises:

1. **Multi-step sequences** — goals require 3+ skills in order, not single-shot.
2. **Repeatable patterns** — the same sequence works across task instances with different parameters.
3. **Transfer** — routine compiled from instance A fires on novel instance B without retraining.
4. **Hierarchical composition** — sub-tasks compose into larger tasks (sub-routines).
5. **Clear success/failure** — each step and each goal has an observable outcome.

SOMA is not built for search problems (Rubik's cube), continuous-control torque optimization, or pixel-based perception. It operates at the skill/port abstraction level: discrete actions with typed observations.

## Proof domains

### Grid navigation (MiniGrid)

Discrete grid world. Agent navigates rooms, picks up keys, opens doors, avoids obstacles, reaches goal. Step-based: action in, observation out.

**Why it fits:** Discrete steps map 1:1 to SOMA's control loop. Multi-room tasks require sequences (pick key -> open door -> navigate -> pick key -> open door). Procedurally generated layouts test transfer. DoorKey, MultiRoom, and ObstacleCourse variants test increasing complexity.

**Ports needed:** `grid.observe` (get visible cells), `grid.move_forward`, `grid.turn_left`, `grid.turn_right`, `grid.pickup`, `grid.toggle` (open door). Six ports wrapping the Gymnasium MiniGrid API.

**What SOMA should learn:** "For DoorKey tasks: observe -> navigate_to_key -> pickup -> navigate_to_door -> toggle -> navigate_to_goal." This sequence transfers across all DoorKey layouts.

**Benchmark:** MiniGrid on PapersWithCode. Standard baselines: PPO, DQN, RIDE, BeBold, NovelD. Published sample efficiency numbers (frames to solve). SOMA's claim: learns from ~10 episodes where deep RL needs 100,000+ frames.

### Robotic manipulation (Meta-World)

50 distinct simulated robot arm tasks: push, pick-place, open drawer, close window, press button. Each task has parametric variations (different object positions).

**Why it fits:** Multi-step manipulation sequences. 50 tasks test multi-task transfer. Train on 45 tasks, test on 5 held-out. Hierarchical: "assemble widget" = sub-routines for each part. The standard benchmark for measuring whether learning transfers across task variations.

**Ports needed:** `arm.get_position`, `arm.get_gripper`, `arm.move_to`, `arm.grip`, `arm.release`, `arm.get_object_positions`. Wrapping Meta-World's step API via discretized actions.

**What SOMA should learn:** Common sub-routines like "approach -> grip -> lift -> move -> release" that transfer across pick-place variants with different object positions.

**Benchmark:** Meta-World ML10 and ML45 on PapersWithCode. Standard baselines: PEARL, MAML, multi-task SAC. Published success rates and sample efficiency per task. SOMA's claim: routine transfer across task variants without retraining.

### Autonomous driving (CARLA)

High-fidelity urban driving simulator. Navigate intersections, avoid pedestrians, follow traffic rules, reach destination.

**Why it fits:** Multi-step route execution. Hierarchical: "drive route" = sub-routines for "navigate intersection", "lane change", "avoid pedestrian". Transfer: same driving patterns apply across different routes. High-profile leaderboard with industry attention.

**Ports needed:** `car.get_position`, `car.get_speed`, `car.get_obstacles`, `car.set_throttle`, `car.set_steering`, `car.set_brake`, `car.get_traffic_light`. Wrapping CARLA's Python API. Requires discretizing continuous control into skill-level actions (e.g., `car.lane_change_left` as a macro-skill).

**What SOMA should learn:** "For intersection_crossing: check_light -> decelerate -> scan_pedestrians -> proceed_or_wait -> accelerate." Transfers across different intersections.

**Benchmark:** CARLA Leaderboard (leaderboard.carla.org). Metrics: route completion, infraction score, driving score. Standard baselines: TransFuser, InterFuser, TCP, ThinkTwice. High visibility.

**Honest limitation:** CARLA's continuous control is a poor fit for SOMA's discrete skill selection. Would need macro-skills (lane_change, intersection_crossing) rather than raw throttle/steering per frame. This changes what's being measured — strategic driving decisions, not low-level control.

### Drone navigation

Simulated drone navigates 2D or 3D environment to reach targets while avoiding obstacles.

**Why it fits:** Multi-step sequences (scan -> navigate -> avoid -> approach -> confirm). Transfer: same avoidance pattern works for different obstacle layouts. Hierarchical: "survey area" = sub-routines for "navigate_to_waypoint" + "scan_area". Can start simple (2D grid) and scale to 3D physics sim.

**Ports needed:** `drone.get_position`, `drone.get_obstacles`, `drone.set_throttle`, `drone.set_yaw`, `drone.set_pitch`, `drone.get_target_bearing`. For 2D grid version: `drone.observe`, `drone.move`, `drone.turn`.

**What SOMA should learn:** "For reach_target: scan -> move_forward -> scan -> adjust_heading -> move_forward -> check_target." Routine fires on novel target positions.

**Benchmark (2D):** Custom, no established leaderboard. Compare against A* (optimal), PPO, DQN on same grid.

**Benchmark (3D):** AirSim environments. No formal leaderboard but widely used in RL papers. Compare sample efficiency.

### IT incident response

Detect anomaly, diagnose root cause, mitigate, verify recovery, notify. Uses real infrastructure ports.

**Why it fits:** Procedural multi-step sequences that repeat across incidents. Transfer: "database connection spike" has the same mitigation pattern regardless of which service triggered it. Hierarchical: "full incident response" = sub-routines for "diagnose", "mitigate", "verify". Closest to what SOMA can run today — HelperBook already has postgres, smtp, http ports.

**Ports needed:** Already exist: `soma.ports.postgres.query`, `soma.ports.postgres.execute`, `soma.smtp.send_plain`, `soma.ports.http.request`. Add: `monitoring.check_health`, `monitoring.get_metrics`.

**What SOMA should learn:** "For db_connection_spike: check_health -> query_connections -> kill_idle -> verify_health -> notify_oncall." Transfers across databases, services.

**Benchmark:** No standard leaderboard. Compare against runbook automation (PagerDuty, Opsgenie) on resolution time and success rate.

### Order fulfillment / business process

Receive order, validate inventory, pick items, pack, ship, confirm delivery, handle exceptions.

**Why it fits:** Classic procedural workflow. Every order follows the same pattern with different parameters. Transfer: routine works for any product/customer. Hierarchical: "fulfill order" = sub-routines for "validate", "pick", "pack", "ship".

**Ports needed:** `inventory.check`, `inventory.reserve`, `warehouse.assign_pick`, `shipping.create_label`, `shipping.dispatch`, `notification.send`, `payment.capture`.

**Benchmark:** No standard leaderboard. Compare against hardcoded workflow engines (Temporal, Airflow) on adaptability when process changes.

## Visual proof concepts (ML10-complete)

Three web UI concepts that cover all 10 Meta-World ML10 manipulation tasks (reach, push, pick-place, door-open, drawer-close, drawer-open, button-press, peg-insert-side, window-open, window-close) in a single coherent 2D browser simulation.

### Tiny Kitchen (top-down countertop)

Robot arm in center of a kitchen counter. Objects: spice jars, cutting board, cabinet with hinged door, utensil drawer, sliding window above sink, food processor with button, knife block with peg slots.

| ML10 Task | Kitchen Action |
|---|---|
| reach | Move arm to ingredient on counter |
| push | Slide cutting board to target position |
| pick-place | Pick spice jar, place on shelf |
| door-open | Open cabinet door (pull handle) |
| drawer-close | Close utensil drawer |
| drawer-open | Open utensil drawer |
| button-press | Press food processor button |
| peg-insert-side | Insert knife into block slot |
| window-open | Slide kitchen window open |
| window-close | Slide kitchen window closed |

**Operator interaction:** Drag objects to arbitrary positions, toggle active tasks, adjust difficulty (randomize positions, add obstacles). ~1000 LOC. Requires 2-link arm inverse kinematics. Universal metaphor — everyone understands a kitchen.

### Clockmaker's Workbench (side-view)

Steampunk brass mechanical hand on a watchmaker's desk. Objects: gears, springs, cuckoo clock housing with door, parts cabinet with drawers, stamp press, pin holes, glass display case with sliding panels.

| ML10 Task | Clockmaker Action |
|---|---|
| reach | Move hand to component |
| push | Slide gear along track to meshing position |
| pick-place | Pick spring, place into clock housing |
| door-open | Open cuckoo clock door |
| drawer-close | Close parts drawer |
| drawer-open | Open parts drawer (reveals components) |
| button-press | Press stamp/punch tool down onto piece |
| peg-insert-side | Insert pin into clock mechanism hole |
| window-open | Slide glass case panel open |
| window-close | Slide glass case panel closed |

**Operator interaction:** Place components, open parts catalog sidebar, toggle X-ray mode to see inside housing/drawers. Ghost trails show arm paths from prior episodes — directly visualizes learning. ~1300 LOC. Side-view 2-link arm IK. Most visually striking aesthetic but highest implementation cost.

### Post Office Sorting Room (isometric)

Gantry robot on overhead rail above a conveyor belt. Objects: mail slots, package bins, service window, stamp machine, cabinet with door, category drawers, pegboard for sorting tags.

| ML10 Task | Post Office Action |
|---|---|
| reach | Move claw to mail piece on conveyor |
| push | Push package along conveyor to sorting zone |
| pick-place | Pick letter, drop into correct mail slot |
| door-open | Open mailroom cabinet |
| drawer-close | Close category drawer |
| drawer-open | Open category drawer |
| button-press | Stamp a letter (press down on stamp pad) |
| peg-insert-side | Hang sorting tag on pegboard hook |
| window-open | Open service window |
| window-close | Close service window |

**Operator interaction:** Spawn mail items onto conveyor, label drawers with categories, rearrange room layout, set conveyor speed, view scoreboard with improvement graph. ~850 LOC. No IK needed — gantry arm is X-rail + Y-drop. Conveyor creates natural motion. Scoreboard directly visualizes SOMA learning throughput.

## Established benchmarks with leaderboards

| Benchmark | Domain | Metrics | Where | Best fit for SOMA |
|---|---|---|---|---|
| MiniGrid | Grid navigation | Sample efficiency, success rate | PapersWithCode | Discrete steps, transfer, compositional tasks |
| Meta-World ML10/ML45 | Robotic manipulation | Success rate, sample efficiency | PapersWithCode | Multi-task transfer, sub-routine learning |
| CARLA Leaderboard | Autonomous driving | Route completion, driving score | leaderboard.carla.org | Strategic decisions (macro-skills) |
| BabyAI | Grid + language | Sample efficiency, success rate | PapersWithCode | Instruction-following, compositional goals |
| NetHack (NLE) | Roguelike game | Score, dungeon depth | NeurIPS challenge, PapersWithCode | Long-horizon planning, procedural environments |
| ProcGen | 16 procedural games | Generalization across levels | PapersWithCode | Transfer to unseen layouts |
| Crafter | Open-world survival | Achievement score | PapersWithCode | Hierarchical sub-goals, exploration |

## What SOMA measures that others don't

Standard RL benchmarks measure cumulative reward and sample efficiency. These matter, but SOMA's claims require additional metrics:

1. **Episodes to first routine** — how many executions before a reusable routine compiles. SOMA's claim: 5-10 episodes. Deep RL baseline: thousands of frames for comparable policy.
2. **Transfer rate** — fraction of novel instances where a compiled routine fires and succeeds without retraining.
3. **Compositional generalization** — can two learned sub-routines compose to solve a task neither was trained on?
4. **Routine compression ratio** — BMR model evidence. Lower = more compressed = better generalization.
5. **Deliberation bypass rate** — fraction of steps executed via plan-following vs. deliberative EFE scoring. Higher = more learned, faster execution.

## Recommended starting point

**MiniGrid-DoorKey-8x8.** One pack, six ports, a proof harness (`soma-project-minigrid`). Discrete, step-based, directly comparable to published PPO/DQN baselines. If SOMA solves DoorKey in 10 episodes where PPO needs 100,000 frames, that's the headline. Then scale to MultiRoom and KeyCorridor for hierarchical composition proof.

Second: **Meta-World ML10** for multi-task transfer. Third: **CARLA** for visibility, if macro-skill abstraction works.
