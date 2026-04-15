# SOMA as an LLM-Native Behavioral Application Language/Runtime

## Abstract

SOMA is an **LLM-native behavioral application language/runtime**: a system in which the primary unit of application authorship is not imperative source code, but structured behavioral intent executed by a runtime.

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

The central claim: **the application is never written — it is observed, compiled from experience, and distributed across peers.** The LLM demonstrates behavior. The runtime watches, extracts patterns, and compiles them into deterministic execution paths. Over time, the LLM programs itself out of the critical path. The application is an emergent property of observation, not a designed artifact.



## 1. The Cost Inversion

Every existing LLM agent framework has a linear or growing token cost. Every action requires a model call. Scale the workload, scale the bill.

SOMA inverts this curve.

- **Day 1**: the LLM reasons through every action. Full token cost per step.
- **Week 2**: PrefixSpan extracts recurring skill sequences from episodes. Schemas form.
- **Week 3**: schemas compile into routines. Routines execute without the LLM.
- **Month 2**: 60-80% of actions are compiled routines. LLM cost has dropped to a fraction.
- **Steady state**: the LLM handles only novel situations. Everything else runs at zero marginal token cost.

This is not an optimization. It is a structural advantage. A system that gets cheaper as it learns is fundamentally different from one that stays expensive forever.

The cost inversion also changes the economics of application ownership. Traditional applications have ongoing maintenance cost (developer time). LLM agent frameworks have ongoing inference cost (token spend). SOMA has a learning cost that converges toward zero as routines compile. The application becomes self-sustaining.


## 2. Core Thesis

**SOMA is an LLM-native behavioral application language/runtime.**

This means:

> the primary way an application is authored, reviewed, evolved, and operated is through structured behavioral declarations that a runtime can execute, route, constrain, transfer, and verify.

The application is not a collection of imperative source files. It is a **behavioral system description** that includes:

- event triggers (world state match conditions, webhooks, scheduler)
- state conditions (belief state, world state facts)
- actions (skill invocations via ports)
- retries and fallbacks (branching via `on_failure` with `Goto` or `CallRoutine`)
- escalations (brain fallback when autonomous confidence is low)
- policy scopes (7 lifecycle hooks, per-routine policy scope override)
- routing rules (load-aware routine router with `LocalFirst`, `LeastLoaded`, `RoutineAffinity` strategies)
- placement rules (routine replication to peers)
- failover/transfer semantics (routine transfer between SOMA instances, declarative failover on peer death)
- priority and conflict resolution (`priority`, `exclusive` fields on routines)
- durability and observability requirements (episode recording, trace steps, world state snapshots)

The runtime turns those declarations into live application behavior.


## 3. The Application Without an Author

This is the most radical implication.

Traditional software: a team designs behavior, encodes it in source files, reviews, tests, deploys, maintains. The application exists because humans wrote it.

SOMA: an operator demonstrates behavior through conversation. "When a CRM webhook arrives, look up the client, create an invoice, send a confirmation email." They do it three times. The runtime observes, extracts the pattern, compiles a routine. The fourth time the webhook arrives, the routine fires autonomously.

Nobody designed this application. Nobody wrote a spec. Nobody reviewed a pull request. The application emerged from observed behavior, the way a reflex forms from repeated stimulus-response.

This is not "low-code" or "no-code" in the traditional sense. Those paradigms still have an author — someone dragging boxes, configuring flows, clicking "deploy." In SOMA, the author is the operator teaching through action, and the compiler is experience.

What does software look like when the author is the user and the compiler is repeated observation? That is the question SOMA answers.


## 4. Why Existing Language Categories Are Not Enough

Calling SOMA a "programming language" in the traditional sense is only partially correct.

Traditional languages describe *how computation is implemented*: functions, methods, loops, classes, modules, services.

But many real applications are dominated not by pure computation, but by behavior under operational conditions:

- if an event occurs, react
- if an action fails, retry or branch to a fallback
- if retry fails, escalate
- if a node is unavailable, transfer responsibility
- if a request matches a capability boundary, route accordingly
- if policy denies execution, halt or require approval

These concerns are usually encoded in fragmented ways across frameworks, queues, services, and infrastructure.

SOMA pulls these concerns upward into first-class authoring elements.

The more precise framing is not "programming language," but:

**behavioral application language/runtime**


## 5. Behavioral Authoring as the Primary Surface

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

Instead of expressing behavior indirectly through low-level implementation, the author expresses the behavior itself.

For example:

- **if payment fails → retry with backup method → if still fails → notify operator**
  - Implemented: `CompiledStep::Skill { on_failure: NextStep::Goto { step_index: N } }` where step N is the backup path, with a final `on_failure: NextStep::CallRoutine { routine_id: "notify_operator" }`
- **CRM webhook → route to instance with CRM routines**
  - Implemented: webhook listener patches world state → reactive monitor matches routine → `RoutineRouter` with `RoutineAffinity` strategy routes to the peer with CRM skills
- **invoice request → route to invoicing instance**
  - Implemented: `RoutineRouter::route()` with `LeastLoaded` strategy finds the peer with invoicing port capabilities
- **instance failure → transfer routines to surviving instances**
  - Implemented: `HeartbeatManager` detects peer offline → emits world state fact → failover routine matches → `replicate_routine` transfers compiled routines to surviving peers

These are **behavioral rules**, **runtime scenarios**, or **live operational contracts** — not functions.


## 6. The Distributed Learning Network

One SOMA instance learns a behavior. `transfer_routine` sends it to a peer. `replicate_routine` fans it out to N peers. Every instance in the fleet now has the compiled routine.

This scales learning, not just execution.

A traditional application scales by deploying the same code to more servers. The code doesn't improve — it runs. A SOMA fleet scales by distributing learned behaviors. When instance A learns how to onboard CRM leads from 3 demonstrated episodes, instances B through N inherit that compiled routine without re-learning.

The implications:
- **Knowledge propagation**: one operator teaches one instance, every instance benefits
- **Specialization**: instances develop different routine libraries based on their workload, then share specializations
- **Resilience**: when a peer dies, its routines survive in the peers it replicated to. Declarative failover routines (matching `peer.status == "offline"`) re-distribute automatically
- **Collective intelligence**: the fleet's behavioral repertoire grows as a union of all instances' learning, not as a single codebase deployed everywhere

This is closer to how organizations work than how software works. Knowledge transfers between team members. Expertise compounds. New hires inherit institutional knowledge. SOMA's routine transfer mechanism is the technical substrate for the same pattern.


## 7. Why SOMA Is LLM-Native

A system is not LLM-native merely because it uses LLMs.

A system becomes LLM-native when its primary authoring surface is well matched to how LLMs reason effectively, AND when the system structurally adapts to the LLM's strengths and weaknesses.

**Structural LLM-nativeness:**

- The LLM **programs itself out of the critical path**. This is the deepest form of LLM-awareness: the system knows that LLM reasoning is expensive and unreliable at scale, so it compiles LLM-derived behavior into deterministic routines that bypass the LLM entirely.
- The runtime is **fully self-describing**. `list_ports` reveals capabilities, `dump_state` reveals context, `dump_world_state` reveals current facts. The LLM builds its mental model from the runtime itself, not from external documentation that may be stale.
- **29 MCP tools** form the instruction set the LLM executes against. The LLM doesn't write code — it calls tools. The tools are typed, self-documenting, and grounded in real execution results.
- **Observation-grounded execution** prevents hallucination at the action layer. Every port call produces a typed `PortCallRecord`. The LLM can still hallucinate when reasoning, but the body's actions are always real.
- **`author_routine`** lets the LLM directly translate intent into compiled behavior — the LLM is not just using the system, it is programming it.

**Surface-level LLM-friendliness:**

- Routines are bounded, meaningful, and scenario-oriented — matching how LLMs reason about behavior
- Behavioral diffs (changes to `compiled_steps` JSON) are easier to review than code diffs across 20 files
- The entire application state is accessible via a single `dump_state` call — no searching through files


## 8. The Central Role of the Runtime

The runtime is not incidental. It is fundamental.

In a conventional language ecosystem: code is primary, runtime mostly executes code.

In SOMA: behavioral declarations are primary, runtime interprets, routes, constrains, transfers, and coordinates them.

Without the runtime, routines are only descriptions. With the runtime, they become executable operational semantics.

That is why SOMA is described as a **language/runtime**, not just a language.

The runtime is where:

- events are received (webhook listener, scheduler, world state patches)
- state is updated (belief state, world state facts, episode store)
- skills are invoked (port dispatch via 22 dynamically loaded adapters, 140+ capabilities)
- policies are enforced (7 lifecycle hooks, per-routine policy scope)
- routines are placed (load-aware `RoutineRouter` with 3 strategies)
- priorities are resolved (`find_matching` sorts by priority DESC, `exclusive` blocks lower-priority matches)
- responsibilities are transferred (`transfer_routine`, `replicate_routine` across peers)
- continuity is preserved (episode ring buffer, schema induction, routine compilation)
- observability and audit are produced (session traces, port call records, metrics)

The runtime is part of the meaning of the application.


## 9. Three-Layer Model

### 9.1 Implementation Layer

The substrate implemented in Rust. Written once, doesn't change per application.

- runtime internals (16-step control loop, session controller)
- transport protocols (TCP/TLS, WebSocket, Unix socket, mDNS discovery)
- storage engines (episode ring buffer, schema store, routine store, world state)
- scheduling (background scheduler with one-shot and recurring tasks)
- adapters (port SDK, dynamic `.dylib` loading, MCP-client port backend)
- execution engine (skill lifecycle: bind → preconditions → authorize → execute → observe → patch)
- policy enforcement hooks (7 lifecycle points)
- observability infrastructure (traces, metrics, proprioception)

### 9.2 Behavioral Application Layer

The true application authoring layer.

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
- policies (per-routine `policy_scope` overrides namespace, destructive ops require confirmation)
- routing logic (`RoutineRouter` with `LocalFirst`, `LeastLoaded`, `RoutineAffinity`)
- priority and conflict resolution (`priority: u32`, `exclusive: bool`, sorted dispatch)
- failover logic (peer offline → world state fact → declarative failover routine fires)
- escalation logic (brain fallback when selector confidence < 0.3)
- continuity rules (episode → schema → routine compilation pipeline)

This is the layer that deserves to be called the **application language**.

### 9.3 Runtime Coordination Layer

Enacts the behavioral model in live operation.

- event ingestion (webhook HTTP listener, world state patches, scheduler fires)
- state transitions (belief updates from observations, world state fact accumulation)
- skill invocation (port dispatch with typed PortCallRecord results)
- routine dispatch (reactive monitor evaluates autonomous routines against world state, priority-sorted, exclusive-aware)
- placement decisions (`RoutineRouter` consulted before execution, delegates to remote peer when load exceeds threshold)
- transfer/failover (`transfer_routine` and `replicate_routine` via wire protocol, `on_peer_offline` callback)
- policy enforcement (per-skill, per-session, per-step, per-routine-scope)
- logging and auditing (session traces with step-by-step port calls, observations, critic decisions)
- recovery (failure classification → retry/switch/backtrack/delegate/stop)

The application is authored mainly in Layer 2 and operationalized by Layer 3.


## 10. Formal Definition of a Routine

A SOMA routine is:

> a structured behavioral unit that specifies how the runtime should react to a class of events or conditions, under given policies and state assumptions, using available skills and ports, with defined recovery, escalation, routing, and transfer semantics.

This definition distinguishes a routine from:

- a function (routines branch, compose, and fire reactively)
- a workflow step (routines are complete behavioral units with their own match conditions and priority)
- a test scenario (routines execute against real systems)
- a queue consumer (routines are event-driven but also goal-driven and schedule-driven)
- an infrastructure rule (routines operate at the application level, not the infrastructure level)

A routine is a **live runtime behavior contract**.

### 10.1 Routine Structure (as implemented)

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
    pub priority: u32,                // higher fires first
    pub exclusive: bool,              // blocks lower-priority matches
    pub policy_scope: Option<String>, // overrides policy namespace during execution
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
- **Priority** — higher-priority routines fire first; `exclusive` prevents lower-priority matches
- **Policy isolation** — `policy_scope` constrains what the routine can do during execution

### 10.2 Routine Lifecycle

1. **Observed**: LLM-assisted execution produces episodes with skill sequences and world state context
2. **Induced**: PrefixSpan extracts recurring patterns → schemas form with trigger conditions
3. **Compiled**: high-confidence schemas compile into routines with `compiled_steps`
4. **Authored**: alternatively, the LLM creates routines directly via `author_routine` MCP tool
5. **Transferred**: received from a remote peer via wire protocol, stored with `origin: PeerTransferred`
6. **Matched**: reactive monitor evaluates `match_conditions` against world state snapshot (priority-sorted, exclusive-aware)
7. **Executed**: plan-following mode walks each step, applies `on_success`/`on_failure` branching, resolves sub-routines via call stack
8. **Invalidated**: on pack version break, resource schema change, confidence drop, or policy change


## 11. Illustrative Routine Shapes

### 11.1 As Implemented (JSON via `author_routine` MCP tool)

```json
{
  "routine_id": "payment_recovery",
  "match_conditions": [
    { "condition_type": "world_state", "expression": { "payment.status": "failed" }, "description": "payment failed" }
  ],
  "steps": [
    { "type": "skill", "skill_id": "payment.retry_backup", "on_success": { "action": "continue" }, "on_failure": { "action": "goto", "step_index": 2 } },
    { "type": "skill", "skill_id": "payment.confirm", "on_success": { "action": "complete" }, "on_failure": { "action": "abandon" } },
    { "type": "sub_routine", "routine_id": "notify_operator", "on_success": { "action": "complete" }, "on_failure": { "action": "abandon" } }
  ],
  "priority": 10,
  "exclusive": true,
  "policy_scope": "finance_ops",
  "autonomous": true
}
```

This routine: tries backup payment → if that succeeds, confirms → if backup fails, jumps to step 2 (notify operator sub-routine) → if notification completes, done. Runs under `finance_ops` policy scope. Priority 10 with exclusive — blocks any lower-priority payment routines from firing on the same event.

### 11.2 As Conceptual Surface (future DSL, not yet implemented)

```text
routine PaymentRecovery
  on PaymentFailed
  when invoice.status == "pending"
  do retry payment using backup_method
  else notify operator
  policy finance_ops
  priority 10 exclusive
  observe payment_recovery_attempt
```

The gap between 11.1 and 11.2 is an authoring surface, not a runtime limitation. The runtime already supports everything the DSL would express. The `author_routine` MCP tool bridges this gap for LLM-mediated authoring.


## 12. What Counts as "The Program"

In SOMA, the program is not imperative source code.

The effective application program is the combination of:

- routines (with composition, branching, priority, policy scope)
- goals
- belief/state model (world state facts, session beliefs)
- ports (22 dynamically loaded adapters, 140+ capabilities)
- schemas (PrefixSpan-induced patterns)
- skills (declared in pack manifests)
- policies (lifecycle hooks, per-routine scope)
- routing definitions (`RoutineRouter` strategy)
- placement metadata (peer registry, load thresholds)
- failover/transfer behavior (`replicate_routine`, `transfer_routine`, declarative failover)
- observability requirements (episode recording, trace steps)

The program becomes a **behavioral and operational knowledge structure**, not a collection of source files.

The real competitor is not another framework — it is human developers. 5-10 engineers spending 6-12 months building what SOMA learns in weeks from operator demonstrations. That is the economic framing that matters.


## 13. The Real Competition

SOMA's competition is not Temporal, Airflow, or LangChain. These serve different needs:

- **Temporal/Airflow**: execute workflows that humans design. The workflow doesn't improve. SOMA's routines compile from observation and improve over time.
- **LangChain/CrewAI**: orchestrate LLM calls. Remove the LLM and you have nothing. SOMA runs without an LLM once routines compile.
- **Traditional codebases**: designed, reviewed, deployed, maintained by teams. SOMA eliminates the design/review/deploy cycle for behavioral logic.

The real competition is the **cost of human software engineering** for CRUD, orchestration, and event-driven applications. If SOMA can replace 6 months of team effort with 3 weeks of operator demonstration, the value proposition is clear regardless of architectural elegance.

Where SOMA does NOT compete:
- Performance-critical inner loops (game physics, signal processing, HFT)
- Rich visual interfaces (the product IS its UI)
- Systems requiring formal verification (avionics, medical devices)
- Tiny scripts (3-line Python script is simpler than bootstrapping a runtime)


## 14. What Still Needs Formalization

### 14.1 Routine Conflict Resolution

**Status: implemented.** Routines carry `priority: u32` and `exclusive: bool`. `find_matching()` sorts by priority DESC then confidence DESC. `exclusive: true` blocks lower-priority matches. Remaining gap: named mutex groups across unrelated routines (minor extension).

### 14.2 Scheduling and Dispatch Ordering

**Status: mostly implemented.** Priority-based dispatch ordering and exclusive flag provide deterministic dispatch. Remaining gap: idempotency guarantees (a routine may fire twice if world state oscillates between ticks).

### 14.3 Transfer and Failover Semantics

**Status: partially implemented.** Routine transfer works. `HeartbeatManager` emits world state facts on peer death, enabling declarative failover routines. Remaining gaps: in-flight session handoff, at-least-once execution guarantees, state snapshot transfer alongside routine transfer.

### 14.4 Policy Scope Per Routine

**Status: implemented.** `policy_scope: Option<String>` on Routine overrides namespace in `PolicyContext` during plan-following execution.

### 14.5 Verification Surface

**Status: gap.** This is the enterprise blocker. Routines are inspectable data structures, but there is no formal tool to verify "this routine always completes within budget Y" or "this routine's branching covers all failure modes." Static analysis over the `CompiledStep` graph or bounded model checking would address this.

### 14.6 Operational Maturity

**Status: gap.** Production systems need:

- **Routine versioning**: roll back to a previous version when a new compilation is wrong. Currently, invalidation is destructive (delete). No "undo."
- **Staging gate**: review compiled routine steps before marking `autonomous: true`. Currently manual — no automated approval workflow.
- **Adversarial resilience**: what if 3 episodes of bad practice compile into a harmful routine? No automated detection of routines that violate safety invariants.
- **Canary deployment**: run a new routine against 10% of matching events before full rollout. Not implemented.

### 14.7 Human-Friendly Authoring Surface

**Status: partially implemented.** `author_routine` MCP tool provides LLM-mediated authoring. A textual DSL (§11.2) does not exist but the LLM authoring path covers the same use case via tool calls.


## 15. Why This Is a Next-Era Direction

The next era will not be defined by slightly better syntax or larger context windows.

It will be defined by a change in the **unit of software authorship**.

SOMA demonstrates such a change:

- from code to behavior (routines replace functions)
- from implementation to intent (goals and match conditions replace control flow)
- from services to capabilities (ports replace service integrations)
- from hidden resilience logic to first-class recovery semantics (branching `on_failure` replaces scattered retry middleware)
- from deployment topology to dynamic placement and transfer (routine router replaces static service mesh)
- from scattered app logic to executable routines (compiled steps with composition replace distributed business logic)
- from constant LLM cost to converging-to-zero cost (compiled routines bypass the LLM)
- from designed applications to emergent applications (observation replaces specification)

That is a deeper shift than a new mainstream language syntax.


## 16. Practical Criterion for Success

The claim succeeds if complex applications can be:

- authored primarily through routines and related declarations
- evolved by reviewing behavioral diffs (routine `compiled_steps` are diffable JSON)
- understood without reconstructing hidden control flow from many files
- operated through runtime-native policy, routing, and recovery semantics
- adapted without collapsing back into large handwritten imperative codebases
- learned from operator demonstration, not designed from specifications

**Current evidence**:

- soma-helperbook: service marketplace (3 ports, 32 capabilities) operates entirely through `invoke_port` calls
- soma-project-terminal: multi-user platform where operators teach the system through conversation, routines compile from episodes, reactive monitor fires them autonomously on webhooks
- soma-project-esp32: embedded leaf firmware driven entirely by brain-side routine composition
- End-to-end proven: sub-routine composition (parent calls sub-routine, both execute, stack pops correctly), goto branching (step skipped via `on_success: Goto`), combined composition + branching (sub-routine with internal branching inside a parent routine)
- 1261 tests verify the runtime substrate across all layers


## 17. Final Position

SOMA is: **an LLM-native behavioral application language/runtime.**

Its significance lies in three structural properties no other system combines:

1. **The cost inversion** — the system gets cheaper as it learns, converging toward zero marginal LLM cost
2. **The emergent application** — behavior is observed and compiled, not designed and coded
3. **The distributed learning network** — one instance learns, every peer inherits

In SOMA, the application is not a set of instructions telling the machine how to compute. It is a structured declaration of how the system should behave under conditions, constraints, events, and failure — with composition, branching, routing, priority, policy isolation, and transfer as first-class primitives.

The open questions are not architectural — they are operational: verification, versioning, adversarial resilience, canary deployment. These are the gaps between "architecturally sound" and "enterprise-ready." They are solvable. The foundation supports them.
