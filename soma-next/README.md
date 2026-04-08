# SOMA Next

## Purpose

`soma-next` is a new production runtime.

It is not a migration of the current SOMA runtime.
It does not preserve `Program`, `ProgramStep`, opcode catalogs, ONNX inference, LoRA adaptation, or convention-oriented execution as core architecture.

`soma-next` is a single binary runtime for:

- goal-driven control
- typed world modeling
- skill execution
- external integration through ports
- persistent memory
- distributed SOMA-to-SOMA coordination
- LLM/API/UI/device interfaces

The runtime must be domain-agnostic.
Domain behavior must be loaded through packs, not baked into the binary.

## Normative Specs

The following documents are the canonical normative specs for the most important runtime extension and distributed contracts:

- [`docs/skill-spec.md`](/Users/vm/Projects/personal/soma/soma-next/docs/skill-spec.md)
- [`docs/port-spec.md`](/Users/vm/Projects/personal/soma/soma-next/docs/port-spec.md)
- [`docs/pack-spec.md`](/Users/vm/Projects/personal/soma/soma-next/docs/pack-spec.md)
- [`docs/distributed.md`](/Users/vm/Projects/personal/soma/soma-next/docs/distributed.md)

This top-level file remains the system architecture and replacement-boundary document.

## Hard Design Decisions

- No `intent -> full program -> execute` core path.
- No mandatory ONNX, training pipeline, or internal neural compiler.
- No domain workflows in the core runtime.
- No plugin/convention-centric cognition model.
- No legacy compatibility inside `soma-next`.
- No MVP framing. This spec targets a production runtime.

## Core Runtime Model

The primary execution unit is a `ControlSession`.

The runtime loop is:

1. receive a goal
2. build or refresh belief state
3. retrieve prior episodes, schemas, and routines
4. enumerate valid candidate skills
5. score and predict candidates
6. execute one skill
7. observe result
8. update belief state
9. continue, revise, delegate, or stop
10. persist full trace and memory

The runtime is closed-loop.
It does not depend on precomputing a full sequence.

## Architecture

### Main Components

- `Goal Runtime`
- `Belief Runtime`
- `Resource Runtime`
- `Skill Runtime`
- `Port Runtime`
- `Session Runtime`
- `Selector`
- `Predictor`
- `Critic`
- `Memory Runtime`
- `Policy Runtime`
- `Trace Runtime`
- `Pack Runtime`
- `Interface Runtime`
- `Distributed Runtime`

### Goal Runtime

Responsible for:

- parsing structured or natural-language requests into `GoalSpec`
- normalizing constraints and budgets
- validating permissions and scope
- defining success conditions

The output is a typed goal, not a program.

### Belief Runtime

Responsible for:

- holding current world state as typed resources and facts
- tracking uncertainty and provenance
- merging new observations
- maintaining active session context

Belief state must be queryable, serializable, and checkpointable.

### Resource Runtime

Responsible for:

- typed resource definitions
- resource indexing and identity
- resource versioning
- resource relationships
- resource patches and diffs

Examples of resource categories:

- identity
- sessions
- messages
- files
- devices
- sensors
- actuators
- documents
- jobs
- views
- domain-specific entities provided by packs

### Skill Runtime

Responsible for:

- loading skills from packs
- validating skill definitions
- binding skill inputs against belief state
- selecting concrete execution strategies
- executing primitive and composite skills
- exposing observable results

The skill runtime is the primary action layer.

### Port Runtime

Responsible for:

- low-level interaction with external systems
- typed input/output contracts
- execution sandboxing and capability boundaries
- structured observations

Ports are not cognitive.
Ports are adapters.

Examples of ports:

- filesystem
- postgres
- redis
- http
- websocket
- mqtt
- bluetooth
- gpio
- mobile renderer
- browser renderer
- push notifications
- queues

### Session Runtime

Responsible for:

- creating, persisting, resuming, pausing, aborting, and finalizing control sessions
- managing budgets, deadlines, and retries
- owning session-local working memory

### Selector

Responsible for:

- candidate generation and ranking
- hierarchical choice over routines, schemas, composite skills, and primitive skills
- balancing progress, cost, latency, risk, and information gain

Selection must not be a flat global action classifier.

### Predictor

Responsible for:

- short-horizon consequence estimation
- next-belief prediction
- success probability estimation
- cost and latency estimation
- information gain estimation

The predictor is optional as a learned module, but the predictor interface is mandatory.

### Critic

Responsible for:

- progress evaluation
- loop detection
- dead-end detection
- contradiction detection
- budget overrun detection
- stop/revise/backtrack decisions

### Memory Runtime

Responsible for:

- working memory
- episode memory
- schema memory
- routine memory
- world summaries
- retrieval indexes

### Policy Runtime

Responsible for:

- permissions
- trust boundaries
- destructive action rules
- quotas
- risk handling
- auditability
- pack-level capability rules

### Trace Runtime

Responsible for:

- full per-session trace storage
- per-step candidate/execution records
- audit rendering
- replay support
- post-mortem analysis

### Pack Runtime

Responsible for:

- loading packs
- validating pack compatibility
- registering resources, skills, policies, schemas, and routines
- managing pack version boundaries

### Interface Runtime

Responsible for:

- MCP
- API
- WebSocket
- CLI
- scheduler/events
- UI/device ingress

### Distributed Runtime

Responsible for:

- SOMA-to-SOMA communication
- capability advertisement
- remote session execution
- delegation
- belief/resource synchronization
- distributed observation streams
- routine/schema transfer

## First-Class Runtime Types

### GoalSpec

Required fields:

- `goal_id`
- `source`
- `objective`
- `constraints`
- `success_conditions`
- `risk_budget`
- `latency_budget_ms`
- `resource_budget`
- `deadline`
- `permissions_scope`
- `priority`

### BeliefState

Required fields:

- `belief_id`
- `session_id`
- `resources`
- `facts`
- `uncertainties`
- `provenance`
- `active_bindings`
- `world_hash`
- `updated_at`

### ResourceRef

Required fields:

- `resource_type`
- `resource_id`
- `version`
- `origin`

### Observation

Required fields:

- `observation_id`
- `session_id`
- `skill_id`
- `port_calls`
- `raw_result`
- `structured_result`
- `effect_patch`
- `success`
- `failure_class`
- `latency_ms`
- `resource_cost`
- `confidence`
- `timestamp`

### SkillSpec

Required fields:

- `skill_id`
- `pack`
- `kind`
- `inputs`
- `outputs`
- `required_resources`
- `preconditions`
- `expected_effects`
- `observables`
- `termination_conditions`
- `rollback_or_compensation`
- `cost_prior`
- `risk_class`
- `determinism`

### PortSpec

Required fields:

- `port_id`
- `kind`
- `capabilities`
- `input_schema`
- `output_schema`
- `failure_modes`
- `side_effect_class`
- `latency_profile`
- `auth_requirements`
- `sandbox_requirements`

### Episode

Required fields:

- `episode_id`
- `goal_fingerprint`
- `initial_belief_summary`
- `steps`
- `observations`
- `outcome`
- `total_cost`
- `success`
- `tags`
- `embedding`
- `created_at`

### Schema

Required fields:

- `schema_id`
- `pack`
- `name`
- `trigger_conditions`
- `resource_requirements`
- `subgoal_structure`
- `candidate_skill_ordering`
- `stop_conditions`
- `rollback_bias`
- `confidence`

### Routine

Required fields:

- `routine_id`
- `origin`
- `match_conditions`
- `compiled_skill_path`
- `guard_conditions`
- `expected_cost`
- `expected_effect`
- `confidence`

### ControlSession

Required fields:

- `session_id`
- `goal`
- `belief`
- `working_memory`
- `status`
- `trace`
- `budget_remaining`
- `created_at`
- `updated_at`

## Skills, Ports, and Packs

### Ports

Ports are low-level integration points.
They do not encode business logic.

Port responsibilities:

- execute external operations
- validate typed inputs
- return typed outputs
- emit structured observations
- report precise failure class

Ports must not contain planning logic.

### Skills

Skills are executable behaviors over resources and ports.

Skill categories:

- `PrimitiveSkill`
  - one direct behavior over one or more ports
- `CompositeSkill`
  - orchestrates subskills under explicit semantics
- `RoutineSkill`
  - compiled habitual shortcut
- `DelegatedSkill`
  - remote execution via another SOMA

Skills are the action abstraction of the runtime.

### Packs

Packs are the domain-extension unit.

A pack may provide:

- resource definitions
- skill definitions
- policies
- schemas
- routines
- UI/render adapters
- tests and fixtures

The core runtime must not ship domain packs as architecture.
Reference packs may exist in the repo for testing and examples only.

## What Belongs In Core vs Packs

### Core Runtime

Must include:

- session engine
- belief engine
- resource system
- skill runtime
- port runtime
- memory system
- selector interface
- predictor interface
- critic
- policy engine
- trace engine
- pack loader
- distributed runtime
- MCP/API/protocol interfaces

### Packs

Must contain:

- domain resources
- domain skills
- domain policies
- domain schemas
- domain routines

Examples:

- auth pack
- messaging pack
- calendar pack
- commerce pack
- iot pack
- web-render pack
- mobile-render pack

The runtime stays general.
Functionality breadth comes from packs.

## Session Execution Model

### Canonical Loop

For each session:

1. validate goal and budget
2. build initial belief state
3. retrieve nearest episodes
4. retrieve matching schemas
5. retrieve matching routines
6. enumerate valid skill candidates
7. bind inputs from belief/resources
8. score candidates
9. predict top candidates
10. choose one candidate
11. execute skill
12. collect observation
13. update belief
14. run critic
15. continue, revise, backtrack, delegate, or stop
16. persist trace and memory

### Session Status

Required states:

- `created`
- `running`
- `paused`
- `waiting_for_input`
- `waiting_for_remote`
- `blocked_by_policy`
- `completed`
- `failed`
- `aborted`

### Working Memory

Session-local working memory must store:

- active bindings
- unresolved slots
- current subgoal
- recent observations
- candidate shortlist
- current branch state
- budget deltas

Working memory must be separate from long-term memory.

## Memory Architecture

### Working Memory

- per-session
- transient
- low latency
- checkpointable

### Episode Memory

- append-only exact traces
- retrieval-first behavior
- success and failure both stored
- similarity indexing required

### Schema Memory

- reusable abstract structures
- induced from repeated episodes or provided by packs
- not tied to one exact trace

### Routine Memory

- high-confidence compiled shortcuts
- guarded by explicit match conditions
- bypasses deeper deliberation when safe

### World Summaries

- compact summaries of durable state
- used for bootstrapping belief state quickly

## Learning and Optional Models

### Hard Rule

`soma-next` must not require ONNX, training, or a monolithic internal neural planner to function.

### Allowed Optional Learned Components

- retrieval embedding model
- candidate ranking model
- short-horizon predictor
- language-to-goal parser
- anomaly detector

### Runtime Contract for Learned Components

If a learned component is absent or disabled:

- correctness must remain
- observability must remain
- safety and policy must remain
- quality or speed may degrade

### Required Non-Learned Learning Paths

- episode storage after every session
- schema induction from repeated successful traces
- routine compilation from stable schemas
- predictor calibration from observation error

## Policy and Safety

The runtime must implement:

- typed permission scopes
- pack capability restrictions
- port capability restrictions
- destructive action confirmation rules
- budget enforcement
- bounded retries
- bounded loops
- compensation/rollback hooks
- trust levels for local and remote packs
- audit-grade trace retention

No action may bypass policy because a skill or model suggested it.

## Observability and Trace

### SessionTrace

Each step must record:

- belief summary before step
- retrieved episodes/schemas/routines
- candidate skills considered
- predicted scores and costs
- selected skill
- port calls made
- observation returned
- belief patch applied
- progress delta
- critic decision
- timestamp

### Required System Metrics

- active sessions
- session success/failure rates
- average steps per session
- skill selection distribution
- routine hit rate
- schema hit rate
- episode retrieval hit rate
- predictor error
- port latency/failure by port
- remote delegation rates
- policy blocks

## Interface Runtime

### MCP/API Surface

The runtime must expose:

- create goal
- inspect session
- inspect belief
- inspect resources
- inspect packs
- inspect skills
- inspect trace
- pause/resume/abort session
- query metrics
- query policy decisions

### External LLM Role

LLMs are external interface components.
They may:

- translate natural language into goals
- summarize traces
- generate operator guidance

They must not be required for core runtime correctness.

## Distributed Runtime: SOMA-to-SOMA

SOMA-to-SOMA is a first-class subsystem.
It is not a side protocol.

### Distributed Responsibilities

- peer identity
- peer trust
- capability advertisement
- remote pack and skill exposure
- remote resource publication
- remote goal submission
- remote session execution
- delegation and subcontracting
- observation streaming
- belief synchronization
- routine/schema transfer
- offline queueing and replay
- cost/latency-aware routing

### Peer Object Model

Each peer must advertise:

- peer id
- trust level
- reachable transports
- loaded packs
- exposed skills
- exposed resources
- policy limits
- latency class
- current availability

### Remote Execution Semantics

Remote requests must support:

- submit goal
- invoke skill
- stream observations
- inspect remote session
- transfer session ownership
- subscribe to resource changes
- request schema or routine transfer

### Session Delegation

A local session may:

- delegate one skill to a peer
- delegate one subgoal to a peer
- migrate the whole session to a peer
- mirror the session to a peer for redundancy

Delegation must preserve:

- trace continuity
- policy boundaries
- budget accounting
- attribution of actions and observations

### Belief and Resource Sync

Distributed runtime must support:

- point-in-time resource query
- resource change subscription
- observation stream replay
- conflict-aware merge policy
- stale-state detection

### Distributed Policy

Cross-SOMA actions require:

- peer trust classification
- pack exposure policy
- skill exposure policy
- resource exposure policy
- signed identity or equivalent strong verification
- replay protection
- audit logging

## Pack System

### PackSpec

A pack must define:

- identity
- version
- compatibility range
- resources
- skills
- policies
- schemas
- routines
- dependencies
- observability metadata

### Load Rules

The pack runtime must:

- validate schemas before load
- validate dependency graph
- reject incompatible versions
- reject missing required resources/ports
- reject unsafe policy violations

### Isolation

Packs must be isolated by:

- capability scope
- namespace
- resource access policy
- remote exposure policy

## Repository Structure

Recommended greenfield layout:

```text
soma-next/
  README.md
  docs/
    architecture.md
    distributed.md
    pack-spec.md
    runtime-loop.md
    trace.md
  runtime/
    goal/
    belief/
    resources/
    skills/
    ports/
    sessions/
    controller/
    selector/
    predictor/
    critic/
    memory/
      episodes/
      schemas/
      routines/
      world/
    policy/
    trace/
    packs/
    distributed/
  sdk/
    resource-spec/
    skill-spec/
    port-spec/
    pack-spec/
  interfaces/
    mcp/
    api/
    cli/
  protocols/
    soma-peer/
  packs/
    reference/
  tests/
    integration/
    distributed/
    simulation/
    conformance/
```

## Replacement Boundary

`soma-next` does not import legacy runtime semantics as core architecture.

Not part of `soma-next`:

- `Program`
- `ProgramStep`
- opcode catalogs
- convention id routing
- ONNX inference as mandatory path
- LoRA-centric adaptation
- `intent -> program` inference loop

The old runtime may remain in the repo as a separate system.
`soma-next` must not be shaped by preserving it.

## Production Requirements

The binary is only considered production-ready if it provides:

- deterministic port execution
- durable sessions
- resumable sessions
- queryable belief state
- typed skills and resources
- pack validation on load
- pack isolation
- full execution trace
- policy enforcement
- distributed delegation and sync
- bounded retries and loop control
- checkpoint and restore for sessions and memory

## Non-Goals

`soma-next` is not:

- a monolithic trained planner
- a static workflow engine
- an application-specific backend
- a domain-specific runtime baked into one product
- a compatibility shell around old SOMA

## Implementation Rule

Core runtime intelligence must be based on:

- typed semantics
- memory
- selection
- prediction
- control

not on:

- text dictionaries
- giant intent templates
- full-sequence compiled plans
- hidden legacy abstractions
