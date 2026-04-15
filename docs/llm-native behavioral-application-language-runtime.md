# SOMA as an LLM-Native Behavioral Application Language/Runtime

## Abstract

SOMA is not a programming language in the classical sense.
It is an **LLM-native behavioral application language/runtime**: a system in which the primary unit of application authorship is not imperative source code, but structured behavioral intent executed by a runtime.

In this model, an application is defined through:

- routines (with composition, branching, and sub-routine calls)
- goals
- beliefs/state
- ports
- schemas
- skills
- policies
- routing/placement rules
- transfer/failover behavior

The runtime interprets and enacts these structures, making them operational.
This is a meaningful shift away from the traditional programming-language paradigm, where application behavior is encoded indirectly through functions, classes, services, and scattered control flow.

SOMA therefore points toward a next-era software model in which the application is not primarily "written" as code, but **declared as executable behavior**.



## 1. Introduction

Most existing programming languages were designed for human authors writing imperative or declarative source files that are then compiled or interpreted.
Even modern high-level stacks still assume the same basic model:

- the application is a codebase
- behavior emerges from implementation
- control flow is distributed across files and services
- resilience, routing, retry logic, and failover are typically encoded indirectly

This model is increasingly misaligned with the needs of large-scale machine-assisted software development.

For LLMs, the problem is not merely syntax generation.
The deeper problem is that most real systems are too implicit:

- business logic is fragmented
- routing rules are hidden
- retry semantics are scattered
- operational responsibilities are non-local
- behavior is difficult to inspect as a whole

A next-era application model must make behavior first-class.

SOMA moves in that direction by shifting application authorship upward from implementation details to runtime-native behavior.


## 2. Core Thesis

**SOMA is an LLM-native behavioral application language/runtime.**

This means:

> the primary way an application is authored, reviewed, evolved, and operated is through structured behavioral declarations that a runtime can execute, route, constrain, transfer, and verify.

Under this thesis, the application is not mainly a collection of imperative source files.
It is mainly a **behavioral system description**.

That description includes:

- event triggers (world state match conditions, webhooks, scheduler)
- state conditions (belief state, world state facts)
- actions (skill invocations via ports)
- retries and fallbacks (branching via `on_failure` with `Goto` or `CallRoutine`)
- escalations (brain fallback when autonomous confidence is low)
- policy scopes (7 lifecycle hooks, per-skill policy checks)
- routing rules (load-aware routine router with `LocalFirst`, `LeastLoaded`, `RoutineAffinity` strategies)
- placement rules (routine replication to peers)
- failover/transfer semantics (routine transfer between SOMA instances)
- durability and observability requirements (episode recording, trace steps, world state snapshots)

The runtime turns those declarations into live application behavior.


## 3. Why Existing Language Categories Are Not Enough

Calling SOMA a "programming language" in the traditional sense is only partially correct.

Traditional languages are centered around constructs such as:

- functions
- methods
- loops
- classes
- modules
- services
- procedural side effects

These constructs describe *how computation is implemented*.

But many real applications are dominated not by pure computation, but by behavior under operational conditions:

- if an event occurs, react
- if an action fails, retry or branch to a fallback
- if retry fails, escalate
- if a node is unavailable, transfer responsibility
- if a request matches a capability boundary, route accordingly
- if policy denies execution, halt or require approval

These concerns are usually encoded in fragmented ways across frameworks, queues, services, and infrastructure.

SOMA pulls these concerns upward into first-class authoring elements.

That is why the more precise framing is not simply "programming language," but:

**behavioral application language/runtime**


## 4. Behavioral Authoring as the Primary Surface

A SOMA application is authored primarily through structures such as:

- routines (with `CompiledStep` sequences supporting branching and sub-routine calls)
- goals
- beliefs
- schemas
- policies
- ports
- skills
- routing definitions
- transfer/failover rules

This is a different unit of programming.

Instead of expressing behavior indirectly through low-level implementation, the author expresses the behavior itself.

For example:

- **if payment fails -> retry with backup method -> if still fails -> notify operator**
  - Implemented: `CompiledStep::Skill { on_failure: NextStep::Goto { step_index: N } }` where step N is the backup path, with a final `on_failure: NextStep::CallRoutine { routine_id: "notify_operator" }`
- **CRM webhook -> route to instance with CRM routines**
  - Implemented: webhook listener patches world state → reactive monitor matches routine → `RoutineRouter` with `RoutineAffinity` strategy routes to the peer with CRM skills
- **invoice request -> route to invoicing instance**
  - Implemented: `RoutineRouter::route()` with `LeastLoaded` strategy finds the peer with invoicing port capabilities
- **instance failure -> transfer routines to surviving instances**
  - Implemented: `replicate_routine` MCP tool transfers compiled routines (including `compiled_steps`) to peers via `transfer_routine` wire protocol

These are not naturally described as ordinary functions.
They are **behavioral rules**, **runtime scenarios**, or **live operational contracts**.

This shift matters because it makes the application surface closer to business intent and operational reality.


## 5. Relationship to Gherkin

The comparison to Gherkin is useful.

Gherkin has important properties:

- it is human-readable
- it is structured
- it is scenario-based
- it is close to business meaning
- it is easier to review than raw code for many behaviors

However, Gherkin usually serves as a specification or acceptance layer.
It describes expected behavior, but it does not normally *become* the live behavior of the application.

SOMA goes further.

In SOMA, the behavioral expression is not merely documentation or test specification.
It is part of the application's executable operational model. A routine compiled from observed episodes IS the running application — it fires autonomously when world state conditions match, branches on failure, calls sub-routines, and transfers across instances.

So the right comparison is not:

> SOMA is like Gherkin.

It is closer to:

> SOMA turns scenario-shaped behavioral intent into runtime-native executable behavior.

That makes SOMA stronger than a BDD-style notation.
It is not just behavior description.
It is behavior execution.


## 6. Why SOMA Is LLM-Native

A system is not LLM-native merely because it uses LLMs.

A system becomes LLM-native when its primary authoring surface is well matched to how LLMs reason effectively.

LLMs tend to work better with structures that are:

- semantically compact
- explicit
- scenario-oriented
- close to intent
- easy to diff
- easy to revise locally
- mechanically constrained

They tend to struggle with systems whose behavior is spread across many files and hidden behind conventions.

A routine such as:

> if payment fails -> retry with backup method -> if still fails -> notify operator

is highly LLM-friendly because it is:

- bounded
- meaningful
- operationally clear
- directly editable
- easy to review

By contrast, the same behavior in a conventional codebase might be distributed across:

- retry middleware
- payment adapters
- queue consumers
- notification services
- feature flags
- ops scripts
- tests
- configuration files

That fragmentation makes repo-scale reasoning difficult.

SOMA's routine-oriented surface is much closer to an **LLM-native application authoring model** than conventional source code.

Additionally, SOMA is LLM-native in a deeper structural sense:

- The LLM **programs itself out of the critical path**. Day 1, it reasons through every action. After enough episodes, routines compile and execute without LLM involvement. The LLM becomes the exception handler, not the main loop.
- The runtime is **fully self-describing**. `list_ports` reveals capabilities, `dump_state` reveals context, `dump_world_state` reveals current facts. The LLM builds its mental model from the runtime itself, not from external documentation.
- **29 MCP tools** form the instruction set the LLM executes against. The LLM doesn't write code — it calls tools. The tools are typed, self-documenting, and grounded in real execution results.


## 7. The Central Role of the Runtime

The runtime is not incidental.
It is fundamental.

In a conventional language ecosystem:

- code is primary
- runtime mostly executes code

In SOMA:

- behavioral declarations are primary
- runtime interprets, routes, constrains, transfers, and coordinates them

This distinction is crucial.

Without the runtime, routines are only descriptions.
With the runtime, they become executable operational semantics.

That is why SOMA is described as a **language/runtime**, not just a language.

The runtime is where:

- events are received (webhook listener, scheduler, world state patches)
- state is updated (belief state, world state facts, episode store)
- skills are invoked (port dispatch via 11 dynamically loaded adapters, 88 capabilities)
- policies are enforced (7 lifecycle hooks, destructive operation guards)
- routines are placed (load-aware `RoutineRouter` with 3 strategies)
- responsibilities are transferred (`transfer_routine`, `replicate_routine` across peers)
- continuity is preserved (episode ring buffer, schema induction, routine compilation)
- observability and audit are produced (session traces, port call records, metrics)

The runtime is therefore part of the meaning of the application.


## 8. Three-Layer Model

A useful formal framing for SOMA is a three-layer model.

### 8.1 Implementation Layer

This is the substrate implemented in Rust.

It includes:

- runtime internals (16-step control loop, session controller)
- transport protocols (TCP/TLS, WebSocket, Unix socket, mDNS discovery)
- storage engines (episode ring buffer, schema store, routine store, world state)
- scheduling (background scheduler with one-shot and recurring tasks)
- adapters (port SDK, dynamic `.dylib` loading, MCP-client port backend)
- execution engine (skill lifecycle: bind → preconditions → authorize → execute → observe → patch)
- policy enforcement hooks (7 lifecycle points)
- observability infrastructure (traces, metrics, proprioception)

This layer exists to build the engine. It is written once and does not change per application.

### 8.2 Behavioral Application Layer

This is the true application authoring layer.

It defines:

- routines — with `CompiledStep` sequences supporting:
  - sequential skill execution (`NextStep::Continue`)
  - branching on success/failure (`NextStep::Goto { step_index }`)
  - sub-routine composition (`NextStep::CallRoutine { routine_id }`, `CompiledStep::SubRoutine`)
  - early completion (`NextStep::Complete`)
  - deliberation fallback (`NextStep::Abandon`)
  - call stack nesting up to 16 levels deep (`plan_stack`, `PlanFrame`)
- goals (natural language or structured objective)
- schemas (PrefixSpan-induced patterns from episode clusters)
- ports (external system adapters: postgres, smtp, s3, crypto, redis, etc.)
- skills (declared in pack manifests, map to port capabilities)
- policies (read-only skip, destructive require confirmation, irreversible require host override)
- routing logic (`RoutineRouter` with `LocalFirst`, `LeastLoaded`, `RoutineAffinity`)
- failover logic (routine replication to N peers, transfer on demand)
- escalation logic (brain fallback when selector confidence < 0.3)
- continuity rules (episode → schema → routine compilation pipeline)

This is the layer that deserves to be called the **application language**.

### 8.3 Runtime Coordination Layer

This layer enacts the behavioral model in live operation.

It handles:

- event ingestion (webhook HTTP listener, world state patches, scheduler fires)
- state transitions (belief updates from observations, world state fact accumulation)
- skill invocation (port dispatch with typed PortCallRecord results)
- routine dispatch (reactive monitor evaluates autonomous routines against world state)
- placement decisions (`RoutineRouter` consulted before execution, delegates to remote peer when load exceeds threshold)
- transfer/failover (`transfer_routine` and `replicate_routine` via wire protocol)
- policy enforcement (per-skill, per-session, per-step)
- logging and auditing (session traces with step-by-step port calls, observations, critic decisions)
- recovery (failure classification → retry/switch/backtrack/delegate/stop)

The application is authored mainly in Layer 2 and operationalized by Layer 3.


## 9. Formal Definition of a Routine

A SOMA routine is:

> a structured behavioral unit that specifies how the runtime should react to a class of events or conditions, under given policies and state assumptions, using available skills and ports, with defined recovery, escalation, routing, and transfer semantics.

This definition distinguishes a routine from:

- a function (routines branch, compose, and fire reactively)
- a workflow step (routines are complete behavioral units with their own match conditions)
- a test scenario (routines execute against real systems)
- a queue consumer (routines are event-driven but also goal-driven and schedule-driven)
- an infrastructure rule (routines operate at the application level, not the infrastructure level)

A routine is a **live runtime behavior contract**.

### 9.1 Routine Structure (as implemented)

```rust
pub struct Routine {
    pub routine_id: String,
    pub namespace: String,
    pub origin: RoutineOrigin,        // PackAuthored | EpisodeInduced | SchemaCompiled | PeerTransferred
    pub match_conditions: Vec<Precondition>,   // trigger: goal_fingerprint OR world_state
    pub compiled_steps: Vec<CompiledStep>,     // action sequence with branching
    pub guard_conditions: Vec<Precondition>,   // must ALL pass regardless of trigger
    pub expected_cost: f64,
    pub expected_effect: Vec<EffectDescriptor>,
    pub confidence: f64,
    pub autonomous: bool,             // fires without operator approval
}
```

Each step in `compiled_steps`:

```rust
pub enum CompiledStep {
    Skill {
        skill_id: String,
        on_success: NextStep,   // Continue | Goto | CallRoutine | Complete | Abandon
        on_failure: NextStep,
    },
    SubRoutine {
        routine_id: String,
        on_success: NextStep,
        on_failure: NextStep,
    },
}
```

This gives routines:
- **Sequential execution** — steps run in order via `NextStep::Continue`
- **Branching** — `on_failure: Goto { step_index: 3 }` jumps to a recovery step
- **Composition** — `CompiledStep::SubRoutine` calls another routine, pushes a stack frame, returns on completion
- **Early exit** — `NextStep::Complete` finishes the routine (pops to parent if nested)
- **Escalation** — `NextStep::Abandon` drops to full deliberation (LLM takes over)

### 9.2 Routine Lifecycle

1. **Learned**: episodes accumulate → PrefixSpan extracts skill sequence → schema induced → routine compiled with `compiled_steps`
2. **Authored**: declared in pack manifest JSON with explicit `compiled_skill_path` or `compiled_steps`
3. **Transferred**: received from a remote peer via wire protocol, stored with `origin: PeerTransferred`
4. **Matched**: reactive monitor evaluates `match_conditions` against world state snapshot; OR session controller matches against goal fingerprint
5. **Executed**: plan-following mode loads `effective_steps()`, walks each step, applies `on_success`/`on_failure` branching
6. **Invalidated**: on pack version break, resource schema change, confidence drop, or policy change


## 10. Illustrative Routine Shapes

### 10.1 As Implemented (JSON in pack manifest)

```json
{
  "routine_id": "payment_recovery",
  "match_conditions": [
    { "condition_type": "world_state", "expression": { "payment.status": "failed" }, "description": "payment failed" }
  ],
  "compiled_steps": [
    { "type": "skill", "skill_id": "payment.retry_backup", "on_success": { "action": "continue" }, "on_failure": { "action": "goto", "step_index": 2 } },
    { "type": "skill", "skill_id": "payment.confirm", "on_success": { "action": "complete" }, "on_failure": { "action": "abandon" } },
    { "type": "sub_routine", "routine_id": "notify_operator", "on_success": { "action": "complete" }, "on_failure": { "action": "abandon" } }
  ],
  "guard_conditions": [],
  "autonomous": true
}
```

This routine: tries backup payment → if that succeeds, confirms → if backup fails, jumps to step 2 (notify operator sub-routine) → if notification completes, done.

### 10.2 As Conceptual Surface (future DSL, not yet implemented)

```text
routine PaymentRecovery
  on PaymentFailed
  when invoice.status == "pending"
  do retry payment using backup_method
  else notify operator
  policy finance_ops
  durable true
  observe payment_recovery_attempt
```

Distributed example:

```text
routine CRMWebhookHandling
  on CRMWebhookReceived
  route to instances with capability.crm
  transfer on instance_failure
  policy crm_processing
  durable true
  observe crm_webhook_flow
```

These shapes are important because they are:

- human-readable
- structurally explicit
- operationally meaningful
- easy to diff
- easier for LLMs to edit safely than fragmented imperative code

The gap between 10.1 (implemented) and 10.2 (conceptual) is an authoring surface, not a runtime limitation. The runtime already supports everything the DSL would express.


## 11. What Counts as "The Program"

In SOMA, the program is not imperative source code.

The effective application program is the combination of:

- routines (with composition and branching)
- goals
- belief/state model (world state facts, session beliefs)
- ports (11 dynamically loaded adapters, 88 capabilities)
- schemas (PrefixSpan-induced patterns)
- skills (declared in pack manifests)
- policies (lifecycle hooks)
- routing definitions (`RoutineRouter` strategy)
- placement metadata (peer registry, load thresholds)
- failover/transfer behavior (`replicate_routine`, `transfer_routine`)
- observability requirements (episode recording, trace steps)

This is a major conceptual shift.

The program becomes a **behavioral and operational knowledge structure**, not merely a collection of source files.

That aligns well with long-horizon LLM development, where explicit artifacts and structured constraints matter more than raw code volume.


## 12. What Still Needs Formalization

For SOMA to fully earn the label of a next-era language/runtime, several things need a crisp formal model. The following items were originally identified as gaps. Several have since been implemented (priority/conflict resolution, policy scope, LLM authoring surface). The remaining items are noted with their current status.

### 12.1 Routine Conflict Resolution

**Status: implemented.**

Routines carry `priority: u32` (higher fires first) and `exclusive: bool`. `find_matching()` sorts by priority DESC then confidence DESC. When `exclusive: true`, the reactive monitor fires only the first match, blocking lower-priority routines from the same trigger. This provides priority ordering, mutual exclusion, and deterministic dispatch order. Mutex groups (explicit named exclusion groups across unrelated routines) are not yet implemented but are a minor extension.

### 12.2 Scheduling and Dispatch Ordering

**Status: mostly implemented.**

The scheduler subsystem handles timed tasks (one-shot, recurring). The reactive monitor handles world-state-triggered dispatch. Priority-based dispatch ordering is now implemented: `find_matching()` sorts by priority DESC then confidence DESC, and `exclusive: true` provides "consume" semantics (first match blocks the rest). Remaining gaps:

- no idempotency guarantee (a routine may fire twice if world state oscillates)
- no idempotency tokens

**What exists**: scheduler with `delay_ms`, `interval_ms`, `max_fires`. Reactive monitor with snapshot hash change detection, `fired_set`, priority-sorted dispatch, and exclusive flag.

### 12.3 Transfer and Failover Semantics

**Status: partially implemented.**

Routine transfer between peers works (`transfer_routine`, `replicate_routine`). But:

- no in-flight responsibility handoff (if a routine is mid-execution and the instance dies, the session is lost)
- no at-least-once or exactly-once execution guarantee
- no automatic failover (manual `replicate_routine` replicates, but no automatic re-routing on peer failure)
- no state transfer with routine transfer (world state and episode context stay local)

**What exists**: `RoutineRouter` with load-aware routing, `replicate_routine` for proactive replication, heartbeat-based peer liveness detection, chunked transfer with SHA-256 integrity. `HeartbeatManager` now has an `on_peer_offline` callback that emits world state facts when peers go offline, enabling declarative failover routines (e.g., an autonomous routine matching `peer.status == "offline"` can trigger `replicate_routine` to surviving peers).

**What's needed**: session migration (partially spec'd in `DelegationManager::migrate_session`), state snapshot transfer alongside routine transfer. Automatic re-routing on peer failure is now possible declaratively via failover routines triggered by `on_peer_offline` world state facts.

### 12.4 Policy Scope Per Routine

**Status: implemented.**

Routines now carry `policy_scope: Option<String>`. When set, this overrides the `namespace` in `PolicyContext` via `build_context` in adapters.rs during plan-following execution. The `PaymentRecovery` example with `policy finance_ops` is now directly expressible. The `author_routine` MCP tool accepts `policy_scope` as an optional field, so the LLM can assign policy scopes when authoring routines.

### 12.5 Verification Surface

**Status: gap.**

Routines are inspectable data structures (not opaque weights). `compiled_steps` can be read, diffed, and analyzed. But there is no formal verification tool.

**What exists**: routine inspection via `dump_state`, `execute_routine` with full trace, episode replay.

**What's needed**: a verification tool that can answer questions like "does this routine always complete within budget Y?", "can this routine reach state X?", "does this routine's branching cover all failure modes?". This could be static analysis over the `CompiledStep` graph or bounded model checking.

### 12.6 Human-Friendly Authoring Surface

**Status: partially implemented.**

Routines can be authored as JSON in pack manifests, learned from episodes, or created via the `author_routine` MCP tool (#29). The LLM-mediated authoring path is now live: the brain translates natural language behavioral intent into a structured routine definition (steps, branching, priority, policy scope) and submits it via `author_routine`. The routine is validated and registered in the routine store.

**What exists**: JSON pack manifests, automatic learning from episodes via PrefixSpan → schema → routine, and `author_routine` MCP tool for LLM-mediated authoring.

**What still doesn't exist**: a textual DSL that compiles to `Routine` structs with `CompiledStep` sequences (the §10.2 conceptual surface). The LLM authoring path covers the same use case via tool calls rather than a standalone language.


## 13. Why This Is a Next-Era Direction

The next era will not be defined by slightly better syntax or larger context windows.

It will be defined by a change in the **unit of software authorship**.

SOMA demonstrates such a change:

- from code to behavior (routines replace functions)
- from implementation to intent (goals and match conditions replace control flow)
- from services to capabilities (ports replace service integrations)
- from hidden resilience logic to first-class recovery semantics (branching `on_failure` replaces scattered retry middleware)
- from deployment topology to dynamic placement and transfer (routine router replaces static service mesh)
- from scattered app logic to executable routines (compiled steps with composition replace distributed business logic)

That is a deeper shift than a new mainstream language syntax.

It suggests that the next era of software may be authored less as instruction sequences and more as **executable behavioral systems**.


## 14. Practical Criterion for Success

The claim that SOMA is an LLM-native behavioral application language/runtime succeeds if complex applications can be:

- authored primarily through routines and related declarations
- evolved by reviewing behavioral diffs (routine `compiled_steps` are diffable JSON)
- understood without reconstructing hidden control flow from many files
- operated through runtime-native policy, routing, and recovery semantics
- adapted without collapsing back into large handwritten imperative codebases

**Current evidence**:

- soma-helperbook: service marketplace (3 ports, 32 capabilities) operates entirely through `invoke_port` calls — no application source code
- soma-project-terminal: multi-user platform where operators teach the system through conversation, routines compile from episodes, and the reactive monitor fires them autonomously on webhooks
- soma-project-esp32: embedded leaf firmware driven entirely by brain-side routine composition — the leaf has no concept of "read sensor, show on screen," only primitive port calls composed by the brain
- 1261 tests verify the runtime substrate across all layers

If SOMA continues on this trajectory, it is not merely "inspired by" a language.

It is a new kind of application language/runtime.


## 15. Final Position

SOMA is: **an LLM-native behavioral application language/runtime.**

This framing is stronger and more accurate than treating it as a conventional programming language.

Its significance lies not in copying the old language model, but in replacing the traditional role of application code with a higher-level behavioral authoring surface.

In SOMA, the application is not primarily a set of instructions telling the machine how to compute.
It is a structured declaration of how the system should behave under conditions, constraints, events, and failure — with composition, branching, routing, and transfer as first-class primitives.

That is a credible next-era direction for software.
