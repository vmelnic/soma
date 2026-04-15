# SOMA: A Goal-Driven Runtime Architecture for Direct Intent-to-Execution Computing

**Version 1.0 — April 2026**

---

## Abstract

SOMA (from Greek σῶμα, "body") is a computational architecture in which the runtime itself constitutes the program. No application source code is written, generated, compiled, or interpreted. A single Rust binary receives typed goals, maintains a structured belief state, selects skills from a ranked hierarchy, invokes external systems through dynamically loaded port adapters, observes results, patches beliefs, and iterates under policy constraints until the goal is satisfied or a budget is exhausted. The architecture implements a 16-step control loop with multi-objective skill selection, observation-grounded belief patching, rule-based critic evaluation, and an episodic learning pipeline that promotes observed execution patterns into reusable schemas and compiled routines. External intelligence — typically a large language model — drives the runtime through a 29-tool MCP interface; the runtime drives external systems through typed port libraries. The implementation comprises a ~10MB binary with 1261 tests and zero warnings, a workspace of 11 dynamically loaded port adapters covering databases, cryptography, email, object storage, authentication, geolocation, image processing, push notifications, and timers, and multiple deployed projects demonstrating end-to-end operation — including an embedded `no_std` leaf on ESP32 microcontrollers with 12 hardware ports, runtime-configurable pin assignments, and a driver-agnostic display port that renders live sensor data on an SSD1306 OLED under brain-side MCP control. We present the architecture, its formal properties, the episodic learning pipeline, the distributed execution model, and the embedded-leaf deployment path.

---

## 1. Introduction

### 1.1 The Problem

Every application ever built follows the same pattern: a human understands what needs to happen, then encodes that understanding into a formal language the machine can execute. The encoding is lossy — the developer's mental model of behavior is compressed into syntax, type systems, and control flow. The resulting artifact grows in complexity, accumulates technical debt, and eventually resists modification. The gap between intent and execution is bridged by writing and maintaining source code.

Recent advances in large language models have accelerated code generation but not eliminated the intermediate artifact. AI-assisted development still produces source code — it merely produces it faster. The artifact remains: a codebase that must be reviewed, tested, deployed, versioned, and maintained. The LLM understands intent; the generated code does not. The entire middle layer — source files, build tools, dependency managers, deployment pipelines — persists.

### 1.2 The SOMA Paradigm

SOMA eliminates the intermediate artifact. Instead of generating code that encodes behavior, the runtime *is* the behavior. A SOMA instance boots from declarative manifests that describe available capabilities (ports), executable operations (skills), abstract control structures (schemas), habitual shortcuts (routines), and safety constraints (policies). An external caller — an LLM, another SOMA, or any MCP client — submits a typed goal. The runtime selects skills, invokes ports, observes outcomes, updates its beliefs about the world, and iterates until the goal is satisfied.

The key structural claim: the control loop, its skill hierarchy, its belief state, and its learned routines collectively *are* the program. There is no compilation step, no code generation step, no intermediate representation between the caller's intent and the runtime's execution. New capabilities arrive as loaded ports and pack manifests, not as application rewrites.

The interaction model separates concerns cleanly:

- The **LLM** provides natural language understanding, planning, and conversational intelligence. It is temporary and replaceable.
- The **runtime** provides deterministic execution, permanent state, episodic memory, and safety enforcement. It persists across LLM sessions.
- **Ports** provide typed interfaces to external systems — databases, filesystems, email servers, object stores, sensors, actuators.

Any LLM can drive any SOMA instance. Switching LLMs loses zero context: the runtime's belief state, episode history, and loaded capabilities are queryable in a single call.

### 1.3 Contributions

This paper makes the following contributions:

1. A formal architecture for intent-to-execution computing where the runtime replaces application source code. The architecture is domain-agnostic; domains are expressed as pack manifests.
2. A 16-step control loop with multi-objective skill selection, observation-grounded belief patching, budget-constrained execution, and rule-based critic evaluation.
3. A three-tier episodic learning pipeline (episodes → schema induction → routine compilation) that extracts reusable control structures from observed execution traces.
4. A skill hierarchy (routines → schemas → composites → primitives) with tier-weighted multi-objective scoring and policy-gated selection.
5. A policy system with seven lifecycle hooks providing fine-grained safety enforcement over a deliberative control loop.
6. A working implementation: 1261 tests, 11 dynamically loaded server-side port adapters, multiple deployed projects, a distributed execution layer supporting TCP/TLS, WebSocket, and Unix socket transport with verified cross-instance skill delegation and routine transfer, and a `no_std` embedded deployment path proven on two distinct ESP32 chips with hardware-level I²C bus sharing between sensor and display ports.

---

## 2. Related Work

### 2.1 Cognitive Architectures

SOAR (Laird, Newell & Rosenbloom, 1987) implements a propose-decide-apply cycle with chunking — the automatic compilation of problem-solving traces into production rules. ACT-R (Anderson et al., 2004) uses a modular architecture with declarative and procedural memory, where frequently accessed declarative chunks gain activation strength. Both systems maintain an explicit world model, select operators through conflict resolution, and learn from execution traces.

SOMA shares the deliberative cycle structure and trace-based learning but differs in three respects: (a) SOMA's skill selection uses multi-objective scoring over five weighted dimensions rather than utility-theoretic conflict resolution, (b) SOMA's learning pipeline produces typed schemas and routines with explicit confidence thresholds rather than opaque production rules or activation levels, and (c) SOMA's port abstraction provides a typed boundary to external systems that cognitive architectures typically lack.

### 2.2 BDI Agents

The Belief-Desire-Intention model (Rao & Georgeff, 1995) grounds agent behavior in beliefs about the world, desires (goals), and intentions (committed plans). SOMA's belief state corresponds to BDI beliefs; goals correspond to desires; the selected skill execution path corresponds to intentions. SOMA extends the BDI model with: structured observation records (PortCallRecord), budget-constrained execution across three dimensions (risk, latency, resource), and a critic that detects loops, dead ends, and progress stalls with explicit heuristics rather than relying on plan failure alone.

### 2.3 Neural Program Synthesis

DeepCoder (Balog et al., 2017) uses neural networks to guide search over a domain-specific language. AlphaCode (Li et al., 2022) generates competition-level source code from natural language. Toolformer (Schick et al., 2023) teaches LLMs to insert tool calls into generated text. ToolLLM (Qin et al., 2023) extends this to 16,000+ APIs.

These systems generate source code or tool-call sequences as text — human-readable artifacts that must be parsed and dispatched. SOMA does not generate text. The control loop selects and executes skills directly through typed interfaces. The distinction is structural: there is no serialization-deserialization boundary between decision and execution.

### 2.4 Positioning

|  | Cognitive Architectures | Neural Program Synthesis | Tool-Augmented LLMs | SOMA |
|---|---|---|---|---|
| **Decision mechanism** | Production rules | Neural inference | LLM generation | Multi-objective scoring |
| **Output** | Operator applications | Source code (text) | API calls (text) | Direct port invocations |
| **Learning** | Chunking / activation | Offline training | In-context examples | Episodes → schemas → routines |
| **Safety** | Domain axioms | None | Prompt guardrails | 7-hook policy engine |
| **External systems** | Ad hoc | None | Tool dispatchers | Typed port contracts |
| **State persistence** | Working memory | None | Context window | Belief state + episodes |

---

## 3. Foundational Principles

### 3.1 The Runtime Is the Program

SOMA does not generate, compile, or interpret application source code at any layer. The runtime's control loop, skill registry, belief state, and accumulated routines collectively constitute the executable behavior. New behavior is introduced by loading pack manifests that declare skills, ports, schemas, routines, and policies — not by writing application code.

Pack manifests contain domain-specific strings as data values — SQL queries in skill input schemas, email templates in port configurations, file paths in capability declarations. These are declarative parameters within the manifest, not source code. The manifest describes what the runtime should be able to do; the runtime decides how and when to do it.

### 3.2 Everything Is a Port

The runtime contains exactly six layers: runtime logic, adapter layer, memory stores, interfaces, distributed transport, and built-in ports. Every external capability — databases, filesystems, email, object storage, authentication, cryptography, image processing, geolocation, push notifications, timers, sensors, actuators — is a dynamically loaded port. A server SOMA loads PostgreSQL and S3 ports. An embedded SOMA loads GPIO and I2C ports. Same runtime, different body.

### 3.3 Observation-Grounded Execution

Every port invocation produces a typed `PortCallRecord` — regardless of success or failure. The record contains: invocation identifier, outcome classification, latency measurement, resource cost, confidence estimate, input hash, session provenance, and auth/policy/sandbox results. The control loop makes all decisions based on these observations, not on assumptions about what ports will do. Belief patches are derived from observations. Critic evaluations are derived from observation sequences. Episodes record the complete observation history.

### 3.4 Budget-Constrained Deliberation

Every session operates under three budgets: risk (cumulative exposure to side effects), latency (wall-clock time), and resource (abstract cost units). The control loop checks budgets before each iteration. The policy engine enforces budget thresholds. The critic triggers termination when any budget dimension falls below 10% of its initial allocation. This makes execution bounded by construction — the runtime cannot enter unbounded loops or accumulate unbounded costs.

### 3.5 Separation of Intelligence and Execution

The runtime does not converse, parse natural language beyond goal extraction, or generate human-readable text. Conversational intelligence belongs to the external LLM. The runtime provides: deterministic skill execution, permanent belief state, episodic memory, safety enforcement, and self-description. The LLM provides: natural language understanding, intent decomposition, result explanation, and user interaction.

This separation is load-bearing. The runtime's state is permanent and queryable — when a new LLM session begins, `dump_state` returns complete context (loaded ports, registered skills, active sessions, recent episodes, current metrics, belief state) in a single call. No context is lost across LLM sessions, model switches, or provider changes.

---

## 4. Architecture

### 4.1 System Overview

```
┌─────────────────────────────────────────────────────┐
│  Caller (LLM, MCP client, peer SOMA)                │
└──────────────────────┬──────────────────────────────┘
                       │  MCP (JSON-RPC 2.0) / Transport
                       ▼
┌─────────────────────────────────────────────────────┐
│  SOMA Runtime (single Rust binary, ~10MB)           │
│                                                     │
│  ┌───────────────────────────────────────────────┐  │
│  │  Runtime Logic                                │  │
│  │  session · belief · skill · selector ·        │  │
│  │  predictor · critic · policy · goal ·         │  │
│  │  port · pack · resource · trace · metrics     │  │
│  └───────────────────────┬───────────────────────┘  │
│  ┌───────────────────────┴───────────────────────┐  │
│  │  Adapter Layer (trait wiring)                  │  │
│  └───────────────────────┬───────────────────────┘  │
│  ┌─────────────┐ ┌──────┴──────┐ ┌──────────────┐  │
│  │   Memory    │ │ Interfaces  │ │ Distributed  │  │
│  │  episodes   │ │  CLI (11)   │ │  TCP / TLS   │  │
│  │  schemas    │ │  MCP (19)   │ │  WebSocket   │  │
│  │  routines   │ │             │ │  Unix socket  │  │
│  └─────────────┘ └─────────────┘ └──────────────┘  │
│  ┌───────────────────────────────────────────────┐  │
│  │  Ports: filesystem · http (built-in)          │  │
│  │         + dynamic .dylib/.so (loaded at boot) │  │
│  └───────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘
         │                              │
    ┌────┴─────┐                  ┌─────┴─────┐
    │ External │                  │   Peer    │
    │ Systems  │                  │   SOMAs   │
    └──────────┘                  └───────────┘
```

The runtime is organized into six layers:

1. **Runtime Logic**: The core deliberation engine. All components are trait-based and injected at bootstrap. The `SessionController` orchestrates the 16-step control loop. The `PolicyEngine` gates actions at seven lifecycle hooks. The `PortRuntime` manages port lifecycle and dispatch. The `SkillRuntime` registers and resolves skills.

2. **Adapter Layer**: Wires trait interfaces to concrete implementations. The bootstrap process assembles the full runtime by constructing adapters from configuration and pack manifests.

3. **Memory Stores**: Bounded, append-oriented stores for episodes (execution traces), schemas (abstract control structures), and routines (compiled shortcuts). In-memory by default; disk-backed when a data directory is configured.

4. **Interfaces**: CLI with 11 commands for direct operation. MCP server with 29 JSON-RPC tools for external orchestration (16 core + 3 scheduler + 4 distributed + 3 world state + 3 execution).

5. **Distributed Transport**: TCP with optional TLS, WebSocket, and Unix Domain Socket transport. Delegation model with five units: skill invocation, subgoal submission, session migration, resource operation, and schema/routine transfer.

6. **Ports**: Two built-in ports (filesystem, HTTP). All other capabilities load dynamically from shared libraries (`.dylib` on macOS, `.so` on Linux) discovered from configured search paths.

### 4.2 Type System

The architecture uses a rich classification system that makes safety properties explicit and machine-evaluable:

- **SideEffectClass**: None, ReadOnly, LocalStateMutation, ExternalStateMutation, Destructive, Irreversible
- **RiskClass**: Negligible, Low, Medium, High, Critical
- **DeterminismClass**: Deterministic, PartiallyDeterministic, Stochastic, DelegatedVariant
- **TrustLevel**: Untrusted, Restricted, Verified, Trusted, BuiltIn
- **CapabilityScope**: Local → Session → Tenant → Device → Peer → Public
- **IdempotenceClass**: Idempotent, NonIdempotent, ConditionallyIdempotent
- **FactProvenance**: Asserted, Observed, Inferred, Stale, Remote

These classifications are not advisory metadata. The policy engine evaluates them at runtime: a `Destructive` side effect class triggers the destructive-operation policy gate; a `TrustLevel` below `Verified` restricts capability scope; a non-idempotent operation is not retried automatically.

### 4.3 Bootstrap

The runtime assembles from configuration (`soma.toml`) and pack manifests (`manifest.json`):

1. Initialize port runtime with host capabilities and sandbox profile.
2. For each pack manifest: parse, validate, register declared ports (create adapters by kind), register declared skills.
3. Resolve data directory; create memory stores (in-memory if no data directory; disk-backed otherwise).
4. Construct all adapters: belief source, episode store, schema store, routine store, skill registry, skill executor, predictor, critic.
5. Build policy engine; register host-level default safety policies; register pack-declared policies (rejected on conflict with host policies).
6. Assemble `SessionController` from all components.
7. Return `Runtime` with session controller, goal runtime, skill runtime, port runtime, stores, pack specs, metrics.

Configuration precedence: compiled defaults < `soma.toml` < `SOMA_*` environment variables < CLI flags.

---

## 5. The Control Loop

### 5.1 Session Lifecycle

A session begins when a caller submits a goal and ends when the critic decides to stop, a budget is exhausted, or the caller aborts. Each iteration of the control loop executes the following steps:

1. **Budget validation**: Check risk, latency, resource, and step-count budgets. Terminate if any is exhausted.
2. **Belief capture**: Snapshot current belief state (facts, resources, bindings, world hash).
3. **Episode retrieval**: Query episode store for traces of similar goals (longest-common-prefix matching on goal fingerprint).
4. **Schema retrieval**: Query schema store for schemas whose trigger conditions match current belief.
5. **Routine retrieval**: Query routine store for routines whose match and guard conditions are satisfied.
6. **Candidate enumeration**: Collect all eligible skills from the skill registry.
7. **Working memory cleanup**: Clear unresolved slots from prior iterations.
8. **Candidate scoring**: Score each candidate on five weighted dimensions (predicted success, cost, latency, risk, information gain) with tier bonuses.
9. **Candidate ranking**: Select top-*k* candidates (default *k* = 3).
10. **Skill selection**: Choose highest-scored candidate. Apply policy gate (`BeforeCandidateSelection`).
11. **Skill execution**: Run the 8-step skill lifecycle (scope enforcement → input binding → precondition validation → authorization → execution → observation collection → belief patching → termination evaluation).
12. **Budget deduction**: Deduct observed costs from remaining budgets.
13. **Observable evaluation**: Compare observation against declared observable fields.
14. **Critic evaluation**: Evaluate the observation sequence and decide loop control: `Continue`, `Revise`, `Backtrack`, `Delegate`, or `Stop`.
15. **Failure recovery**: If the critic decides to revise or backtrack, adjust state accordingly.
16. **Trace recording**: Append step record (belief summary, candidates, scores, selected skill, observation, belief patch, critic decision) to the session trace.

### 5.2 Goal Specification

A `GoalSpec` is a typed structure with:

- **Objective**: Natural language description of the desired outcome.
- **Success conditions**: JSON predicates evaluated against belief state.
- **Budgets**: Risk (0.0–1.0), latency (milliseconds), resource (abstract units).
- **Deadline**: Optional wall-clock bound.
- **Permissions scope**: Capability scope constraints.

Natural language input is wrapped as-is into the objective field with default budgets (risk=0.5, latency=30s, resource=100.0). Structured JSON input is deserialized directly. The goal runtime validates that the objective is non-empty, budgets are positive, any deadline is in the future, and success conditions are non-empty. Extreme values are capped (max latency=10 minutes, max resource=10,000).

### 5.3 Belief State

The belief state is the runtime's model of what is true about the world at a given moment:

- **Resources**: Typed entries with identity, version, and mutability. Version conflict detection on updates.
- **Facts**: Subject-predicate-value triples with confidence (0.0–1.0) and provenance (`Asserted`, `Observed`, `Inferred`, `Stale`, `Remote`).
- **Uncertainties**: Explicit declarations of what is unknown.
- **Active bindings**: Current variable bindings for skill input resolution.
- **World hash**: SHA-256 digest of the entire belief state, recomputed after every mutation. Used by the critic to detect belief-state loops.

Belief patches are applied after every observation. A patch may add, update, or remove resources and facts. The world hash provides a deterministic fingerprint: if two consecutive iterations produce the same world hash, the critic detects a loop.

### 5.4 Critic Evaluation

The critic is a rule-based pattern detector that evaluates the observation history and decides the control loop's next action:

- **Loop detection**: Belief hash repetition (three or more identical world hashes) or skill selection repetition (same skill selected three or more times in a five-step window).
- **Dead-end detection**: Three or more consecutive execution failures.
- **Budget proximity**: Any budget dimension below 10% of its initial allocation.
- **Progress stall**: Moving average of progress deltas below 0.01 over three steps.

The critic outputs a `CriticDecision` enum with five values:

| Decision | Meaning |
|---|---|
| `Continue` | Goal not yet satisfied; progress is being made. |
| `Revise` | Current approach is stalling; try a different skill or binding. |
| `Backtrack` | Current path has failed; undo last belief patch and retry. |
| `Delegate` | Goal exceeds local capabilities; submit to a peer. |
| `Stop` | Goal is satisfied, or no further progress is possible. |

Each decision carries a confidence value and a human-readable reason string for auditability.

---

## 6. Skills

### 6.1 Taxonomy

Skills are the executable units of behavior. Four kinds form a hierarchy:

**Primitive**: A single port invocation. The skill specifies a port, a capability, an input schema, and an output schema. Execution dispatches to the port's `invoke` method and returns the `PortCallRecord`.

**Composite**: An ordered subskill graph with conditional branching. Each subskill may itself be primitive, composite, or delegated. The composite executor iterates subskills, evaluates branch conditions against belief state, and aggregates observations (minimum confidence, summed latency, concatenated port calls).

**Routine**: A compiled habitual shortcut. Routines have match conditions (belief predicates that must hold for activation), guard conditions (additional safety predicates), a compiled skill path (ordered list of skills to execute), expected cost, expected effect, and a confidence threshold. Routines execute without the full deliberation overhead of the control loop.

**Delegated**: Execution dispatched to a remote peer. The delegation context carries session identity, remaining budget, required trust level, policy context, execution trace cursor, and attribution.

### 6.2 Multi-Objective Selection

The selector scores each candidate skill on five weighted dimensions:

| Dimension | Weight | Source |
|---|---|---|
| Predicted success | 0.40 | Predictor (exponential moving average from past observations) |
| Cost | 0.20 | Skill's declared cost prior + predictor estimate |
| Latency | 0.15 | Skill's declared latency profile + predictor estimate |
| Risk | 0.15 | Skill's declared risk class + current budget remaining |
| Information gain | 0.10 | Predictor estimate of novel information yield |

Tier bonuses shift scores toward more abstract, proven skills:

| Tier | Bonus |
|---|---|
| Routine | +0.30 |
| Schema-derived | +0.20 |
| Composite | +0.10 |
| Primitive | +0.00 |

Tiebreaker: reversible (rollback-capable) skills are preferred. Delegated candidates are filtered if delegation policy disallows remote execution.

### 6.3 Execution Lifecycle

Each skill execution passes through eight inner steps:

1. **Capability scope enforcement**: Verify the skill's required scope does not exceed the session's granted scope.
2. **Input binding**: Extract values from belief state, goal fields, and working memory. Bind with provenance tracking.
3. **Precondition validation**: Evaluate belief-contains conditions declared by the skill.
4. **Authorization**: Apply policy hooks (`BeforeBindingFinalInputs`, `BeforeExecutionBegins`, `BeforeSideEffectingStep`).
5. **Execution**: Dispatch by skill kind — port invocation for primitives, subskill iteration for composites, skill path execution for routines, transport dispatch for delegated.
6. **Observation collection**: Receive `PortCallRecord` (primitives) or aggregated observation (composites/routines).
7. **Belief patching**: Apply observation-derived patches to belief state.
8. **Termination evaluation**: Check if the skill's declared termination conditions are met. Execute rollback if needed.

---

## 7. Ports — The Body

### 7.1 The Port Contract

A port is a typed interface to an external system. Every port implements a trait with the following obligations:

- **`spec()`**: Return a `PortSpec` declaring the port's identity, capabilities, side-effect class, latency profile, cost profile, authentication requirements, and sandbox requirements.
- **`invoke(capability_id, input)`**: Execute a capability and return a `PortCallRecord`. The record must be returned even on failure — it is the observation that drives the control loop.
- **`validate_input(capability_id, input)`**: Validate input against the capability's declared schema before dispatch.

The `PortCallRecord` is the fundamental observation unit:

| Field | Purpose |
|---|---|
| `invocation_id` | Unique identifier for tracing |
| `success` | Boolean outcome |
| `failure_class` | Typed failure classification (if failed) |
| `output` | Structured result data |
| `latency_ms` | Measured wall-clock time |
| `resource_cost` | Measured resource consumption |
| `input_hash` | SHA-256 of input (for deduplication and replay) |
| `retry_safe` | Whether the same invocation can be retried |
| `session_id`, `goal_id` | Provenance chain |
| `auth_outcome`, `policy_outcome` | Gate results |

### 7.2 Dynamic Loading

External ports are compiled as shared libraries (`cdylib` crates) exporting a single C-ABI symbol:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn soma_port_sdk::Port {
    Box::into_raw(Box::new(MyPort::new()))
}
```

At bootstrap, the runtime discovers libraries from configured search paths using the naming convention `libsoma_port_{port_id}.{dylib|so|dll}`. The `SdkPortAdapter` bridges the SDK's `Port` trait to the runtime's internal `Port` trait via JSON serialization, avoiding ABI mismatch between independently compiled binaries.

Optional Ed25519 signature verification: when `require_signatures` is enabled, the runtime checks `.sig` and `.pub` sidecar files before loading any port library.

### 7.3 Port Catalog

The current implementation provides 11 external ports plus an SDK:

| Port | Capabilities | External System |
|---|---|---|
| auth | 10 | OTP, sessions, TOTP, bearer tokens |
| crypto | 13 | SHA, HMAC, bcrypt, AES-GCM, RSA, JWT, randomness |
| geo | 5 | Haversine distance, radius filter, bounds check |
| image | 5 | Thumbnail, resize, crop, format conversion |
| postgres | 15 | Raw SQL, CRUD, DDL, transactions |
| push | 4 | FCM, WebPush, device registry |
| redis | 13 | Strings, hashes, lists, pub/sub, key listing |
| s3 | 5 | Put, get, delete, presign, list |
| smtp | 3 | Plain text, HTML, attachment email |
| timer | 4 | Timeout, interval, cancellation, listing |
| filesystem | 7 | readdir, readfile, writefile, stat, mkdir, rmdir, rm (built-in) |
| http | 4 | GET, POST, PUT, DELETE (built-in) |

---

## 8. Memory and Adaptation

### 8.1 Episode Store

An episode is a complete trace of a finished session:

- **Goal fingerprint**: Normalized representation of the goal for similarity matching.
- **Initial belief summary**: Snapshot of belief state at session start.
- **Steps**: Ordered sequence of `EpisodeStep` records, each containing: belief summary, candidate list, scores, selected skill, observation, belief patch, progress delta, and critic decision.
- **Outcome**: Success, Failure, PartialSuccess, Aborted, Timeout, or BudgetExhausted.
- **Cost summary**: Total resource consumption across all dimensions.

The store is a bounded ring buffer (default capacity 1024). Retrieval uses longest-common-prefix matching on the goal fingerprint, with optional embedding-based cosine similarity and tag-based filtering.

### 8.2 Schema Induction

When the episode store reaches 75% capacity, the schema induction process activates. A schema is an abstract control structure extracted from multiple successful episodes:

- **Trigger conditions**: Belief predicates that must hold for the schema to be relevant.
- **Subgoal structure**: A graph of subgoal nodes with descriptions, candidate skill lists, and dependency edges.
- **Candidate skill ordering**: Linear ordering of skills for execution.
- **Stop conditions**: Predicates for termination.
- **Rollback bias**: Eager, Cautious, Minimal, or None.
- **Confidence**: Proportion of successful episodes that followed this pattern.

Induction requires three or more successful episodes with the same goal fingerprint. The common skill sequence is extracted and generalized into the schema's subgoal structure. Induction is conservative: only patterns that recur and succeed are promoted.

### 8.3 Routine Compilation

A schema with confidence exceeding 0.7 is eligible for compilation into a routine. Routines are the fastest execution path — they bypass the full deliberation loop and execute a fixed skill sequence:

- **Match conditions**: Belief predicates for activation (all must hold).
- **Guard conditions**: Additional safety predicates (all must hold).
- **Compiled skill path**: Ordered list of skills to execute.
- **Expected cost and effect**: Predicted resource consumption and belief changes.
- **Origin**: PackAuthored (declared in manifest), EpisodeInduced, SchemaCompiled, or PeerTransferred.

Routines are invalidated when: a resource schema changes, a precondition fails at runtime, a policy changes, the pack version changes, or confidence drops below threshold. Invalidated routines fall back to schema-level or primitive-level execution.

### 8.4 The Learning Pipeline

The three stores form an ascending pipeline:

```
Execution traces → Episodes (raw observation history)
                         ↓  (3+ successful episodes, same fingerprint)
                   Schemas (abstract control structures)
                         ↓  (confidence > 0.7)
                   Routines (compiled shortcuts)
```

This pipeline implements a form of experience-based learning: the runtime observes its own execution, detects recurring patterns, abstracts them into reusable structures, and compiles them into fast paths. The process is conservative (high thresholds), transparent (every routine traces back to specific episodes), and reversible (routines can be invalidated and recomputed).

The pipeline is related to chunking in SOAR (Laird, Newell & Rosenbloom, 1987) and compilation in ACT-R (Anderson, 1982), but operates on typed skill sequences with explicit confidence tracking rather than on production rules or declarative chunks.

---

## 9. Policy and Safety

### 9.1 Lifecycle Hooks

The policy engine evaluates safety constraints at seven points in the execution lifecycle:

| Hook | When |
|---|---|
| `BeforeCandidateSelection` | Before the selector enumerates skill candidates |
| `BeforeBindingFinalInputs` | After candidate selection, before input binding |
| `BeforeExecutionBegins` | After binding, before dispatch |
| `BeforeSideEffectingStep` | Before any step with non-None side-effect class |
| `BeforeDelegation` | Before dispatching to a remote peer |
| `BeforeRollback` | Before executing a rollback action |
| `BeforeRemoteExposure` | Before exposing internal state to a peer |

### 9.2 Policy Rules

A policy rule consists of:

- **Conditions**: Predicates over the action context (resource exists, trust level ≥ threshold, budget ≥ threshold, side-effect class matches).
- **Effect**: `Allow`, `Deny`, or `RequireApproval`.
- **Priority**: Host policies override pack policies. Among same-priority rules, the most restrictive effect wins (default-deny on deadlock).

Default host safety policies enforce:

- **Destructive operation gate**: `RequireApproval` for any action with `Destructive` or `Irreversible` side-effect class when trust level is `Verified` or below.
- **Read-only allowance**: `Allow` unconditionally for `ReadOnly` side-effect class.
- **Budget enforcement**: `Deny` when step count exceeds configured maximum.
- **Bounded loops**: `Deny` when the critic detects a loop condition.

### 9.3 Rate Limiting

Per-rule rate limiting uses a sliding-window algorithm (ring buffer of timestamps). Configurable burst limit and sustained rate. Exceeding the limit triggers the rule's overflow effect (typically `Deny`).

---

## 10. Distributed Execution

### 10.1 Transport

The distributed layer supports three transport protocols:

- **TCP with optional TLS**: Length-framed JSON-RPC messages (4-byte big-endian length prefix). TLS via configured certificate and key files.
- **WebSocket**: Standard WebSocket framing over HTTP upgrade.
- **Unix Domain Socket**: For same-host inter-instance communication.

Typed message envelope:

| Message | Purpose |
|---|---|
| `InvokeSkill` | Execute a skill on a peer |
| `QueryResource` | Query a resource on a peer |
| `SubmitGoal` | Submit a goal for remote execution |
| `TransferSchema` | Transfer a learned schema to a peer |
| `TransferRoutine` | Transfer a compiled routine to a peer |
| `ChunkedTransfer*` | Resumable large transfers with hash verification |
| `Ping` / `Pong` | Heartbeat with nonce and load reporting |

### 10.2 Delegation Model

Five delegation units:

1. **Skill invocation**: Execute a single skill on a remote peer. The result is an observation in the local session.
2. **Subgoal submission**: Submit a subgoal to a peer for full control-loop execution. The result is a completed episode.
3. **Session migration**: Transfer an entire in-progress session to a peer, including belief state, trace, budget, policy context, and resource bindings.
4. **Resource operation**: Query or mutate a resource on a peer.
5. **Schema/routine transfer**: Share learned control structures with peers.

Delegation context carries: session identity, remaining budget, required trust level, policy context, trace cursor, and attribution. Acceptance requires: peer trust ≥ required trust, verified peer capability, and sufficient peer budget.

Session mirroring provides redundancy without transfer of authority — the same session runs on multiple peers simultaneously.

### 10.3 Peer Rate Limiting

Per-peer, per-action rate limiting with configurable windows. Graduated response: throttle → reduce window → disconnect and blacklist. Blacklist threshold is configurable. This prevents a compromised or misbehaving peer from consuming unbounded resources.

---

## 11. Interfaces

### 11.1 MCP Server

The MCP server implements JSON-RPC 2.0 over stdin/stdout with 24 tools:

| Tool | Purpose |
|---|---|
| `create_goal` | Submit a goal, create a session, run to completion or wait state |
| `inspect_session` | Query session state (goal, belief, budget, status, trace) |
| `inspect_belief` | Query belief state and facts |
| `inspect_resources` | Enumerate resources in belief |
| `inspect_packs` | List loaded packs and their skills |
| `inspect_skills` | Query skill metadata |
| `inspect_trace` | Fetch session trace |
| `pause_session` | Pause an active session |
| `resume_session` | Resume a paused session |
| `abort_session` | Terminate a session |
| `list_sessions` | Enumerate all sessions |
| `query_metrics` | Runtime metrics (sessions, steps, port calls, policy denials) |
| `query_policy` | Evaluate a hypothetical policy decision |
| `dump_state` | Full runtime state snapshot |
| `invoke_port` | Direct port capability invocation |
| `list_ports` | Enumerate registered ports and capabilities |
| `schedule` | Create a scheduled port invocation (one-shot or recurring) |
| `list_schedules` | Enumerate active schedules |
| `cancel_schedule` | Cancel a scheduled invocation |
| `list_peers` | Enumerate connected remote SOMA peers |
| `invoke_remote_skill` | Invoke a skill on a remote SOMA peer |
| `transfer_routine` | Push a compiled routine to a remote peer |
| `execute_routine` | Run a compiled routine by ID with pre-loaded plan |
| `trigger_consolidation` | Manually trigger the episode → schema → routine pipeline |

The `invoke_port` and `list_ports` tools provide direct port access, bypassing the goal/session/skill machinery. The three scheduler tools — `schedule`, `list_schedules`, and `cancel_schedule` — enable timed port invocations with one-shot delays, recurring intervals, fire count limits, and optional brain routing for result interpretation. The three distributed tools — `list_peers`, `invoke_remote_skill`, and `transfer_routine` — enable LLM-orchestrated multi-instance coordination. The `execute_routine` tool bridges the LLM-driven and autonomous paths by letting the brain trigger compiled routines directly. MCP mode supports `--listen`, `--peer`, `--unix-listen`, `--unix-peer`, and `--discover-lan` flags for distributed operation.

### 11.2 The LLM Interaction Model

A typical LLM-to-SOMA interaction:

1. The LLM calls `dump_state` to obtain full runtime context.
2. The LLM calls `list_ports` to discover available capabilities.
3. Based on the human's request, the LLM either:
   - Calls `create_goal` for complex, multi-step objectives (the runtime handles skill selection, sequencing, and error recovery), or
   - Calls `invoke_port` directly for simple, single-capability operations.
4. The LLM calls `inspect_session` or `inspect_trace` to monitor progress.
5. The LLM explains results to the human.

The runtime holds all state. The LLM is stateless with respect to the SOMA instance — it can be replaced at any point without loss of execution context, belief state, or episode history.

---

## 12. Packs and Projects

### 12.1 Pack Manifests

A pack is the deployment unit. A pack manifest (`manifest.json`) declares:

- **Ports**: External system adapters to register.
- **Skills**: Executable operations (primitive, composite, routine, delegated) with full contracts — inputs, outputs, observables, termination conditions, rollback actions, cost priors, risk classes, determinism, remote exposure.
- **Schemas**: Abstract control structures.
- **Routines**: Pre-compiled execution paths.
- **Policies**: Safety constraints.
- **Exposure metadata**: What the pack exposes to peers.
- **Dependency metadata**: What the pack requires from the runtime.

The manifest is the application. Different manifests produce different application behaviors from the same runtime binary.

### 12.2 The Project Pattern

A `soma-project-*` is a self-contained, deployable unit:

```
soma-project-<name>/
  bin/soma                                 # Pre-compiled runtime binary
  packs/<port>/manifest.json               # Pack manifest
  packs/<port>/libsoma_port_<name>.dylib   # Port shared library
  .env                                     # Configuration (credentials, etc.)
  mcp-client.mjs                           # MCP test client
  scripts/run-mcp.sh                       # Launch MCP server
  scripts/test-all.sh                      # Smoke tests
  samples/                                 # Test payloads
```

The project pattern makes deployment a directory copy. No build step. No dependency resolution. No container image construction. The binary, the port libraries, and the manifest together constitute the entire application.

### 12.3 What Developers Write

To build a new application on SOMA, a developer writes:

1. A pack manifest declaring the application's skills, port requirements, schemas, and policies.
2. Port adapters, if the required external system interface does not already exist.
3. Nothing else.

No controllers, no routes, no ORM mappings, no serialization layers, no build configuration for the application itself. The runtime exists. The ports exist. The application is the manifest plus the running instance.

---

## 13. Implementation

### 13.1 soma-next

Single Rust binary. ~56,000 lines across: runtime logic (session controller, belief, skill registry, skill executor, selector, predictor, critic, policy engine, goal runtime, port runtime, pack runtime, resource runtime, metrics, proprioception, dynamic port loading, port signature verification), adapter layer, memory stores (episodes, schemas, routines, checkpoints, persistence, working memory, world state), interfaces (CLI, MCP, internal API types), distributed transport (TCP/TLS, WebSocket, Unix socket, delegation, routing, streaming, sync, rate limiting, heartbeat, authentication, peer management, message queue, chunked transfer, distributed trace), type definitions (goals, sessions, skills, ports, packs, policies, beliefs, observations, resources, episodes, routines, schemas, peers), and built-in ports (filesystem, http).

1261 tests. Zero compiler warnings. Zero clippy warnings. ~11MB release binary. 29 MCP tools (16 core + 3 scheduler + 4 distributed + 3 execution + 3 world state).

### 13.2 soma-ports

Workspace of 11 port crates plus an SDK. Each port compiles to a `cdylib` shared library. Total: 88 capabilities across 11 ports. The SDK defines the `Port` trait, `PortSpec`, `PortCapabilitySpec`, `PortCallRecord`, and all classification enums.

Ports with external system dependencies: postgres (`SOMA_POSTGRES_URL`), redis (`SOMA_REDIS_URL`), s3 (AWS SDK with `SOMA_S3_*` configuration), smtp (`SOMA_SMTP_*` configuration). Ports with no external dependency: auth, crypto, geo, image, push, timer (pure local logic or in-memory state).

### 13.3 Deployed Projects

Deployed projects demonstrate end-to-end operation across two execution paths (LLM-driven and autonomous) and two deployment targets (server and embedded leaf):

- **soma-project-smtp**: Email delivery via SOMA MCP. The LLM discovers SMTP capabilities via `list_ports`, invokes `send_plain`, `send_html`, or `send_attachment` via `invoke_port`.
- **soma-project-s3**: Object storage via SOMA MCP. Upload, download, delete, presign, and list operations.
- **soma-project-postgres**: Database access via SOMA MCP. SQL queries, CRUD, DDL, and transaction operations.
- **soma-project-llm**: Ollama integration. A local LLM (gemma4:e2b) generates SQL from natural language questions, SOMA executes via the postgres port, LLM interprets results. Demonstrates the brain/body split with a non-cloud LLM.
- **soma-project-mcp**: Claude Code integration. SOMA runs as an MCP server registered in `.mcp.json`, enabling Claude to use all SOMA tools directly. Demonstrates the LLM-driven path with a production LLM.
- **soma-project-s2s**: SOMA-to-SOMA communication. Two SOMA instances cooperate over TCP: transport layer verification (ping/pong, skill invocation, goal submission), cross-instance delegation via MCP (`invoke_remote_skill`), and schema/routine transfer between peers. 42 tests across three levels.
- **soma-project-multistep**: End-to-end proof of multi-step autonomous routine learning. Episodes → schema → routine → real `SessionController` plan-following walks 3 skills against `/tmp` and reaches `Completed`. Five phases, all passing.
- **soma-project-esp32**: Embedded `no_std` leaf firmware. Dual-chip proven on real hardware (ESP32-S3 Sunton 1732S019 and ESP32 LX6 WROOM-32D, both with and without wifi). 12 hardware ports (gpio, delay, uart, i2c, spi, adc, pwm, wifi, storage, thermistor, board, display), runtime-configurable pin assignments persisted to flash, mDNS auto-discovery, and an SSD1306 OLED display port sharing I²C0 with the i2c port via `embedded-hal-bus::RefCellDevice`. The server-side soma-next reaches the leaf through `invoke_remote_skill` over TCP on port 9100 after discovering it via `_soma._tcp.local.` mDNS browsing. A brain-side loop reading the thermistor every 5 seconds and drawing the temperature on the OLED was verified on the physical WROOM-32D panel. Section 13.4 details the embedded architecture.
- **soma-project-terminal**: Multi-user SOMA-native web platform demonstrating the LLM-driven path as an end-user conversational product. A Fallout-inspired terminal UI in the browser talks to a Node HTTP gateway that spawns one `soma-next --mcp` child process loaded with a master pack of three ports (crypto, postgres, smtp). Operators authenticate via magic link, create named contexts (conversation scopes), and chat with a tool-calling LLM brain — `gpt-4o-mini` or `gpt-5-mini` depending on operator preference, with the wrapper auto-detecting the model family and branching parameters accordingly (`temperature` + `max_tokens` for chat models, `reasoning_effort` for reasoning models). The chat brain has exactly one tool, `invoke_port`, and receives the live port catalog embedded in its system prompt via a one-shot `list_ports` snapshot cached at backend startup; it discovers capabilities through the prompt text rather than through introspection tool calls. Every operator context is a scoped conversation against the single runtime — there is no per-context pack generation, no view DSL, no template interpretation, no client-side framework. Per-context data isolation is achieved by the chat brain's system prompt teaching it to prefix every stored artifact (postgres tables, key-value keys) with the context's namespace string. Voice input routes through OpenAI Whisper via a dedicated `/api/transcribe` endpoint. The browser is a thin Markdown chat client with zero wasm; the entire interaction surface is one transcript `<div>`, one input field, and one microphone button. 34 Playwright tests verify the full flow (auth, context CRUD, chat tool-calling, voice, isolation) in ~9 seconds headless. This project is the cleanest user-facing demonstration of the LLM-driven path in the repository: every operator request becomes a tool call on real SOMA ports, no code is generated or maintained per application, and the chat brain composes CRUD operations, email delivery, and cryptographic primitives entirely through `invoke_port` calls discovered from its system prompt catalog. The project documents the multi-tenancy tradeoffs of sharing one SOMA runtime across operators (`docs/terminal-multi-tenancy.md`) and currently operates in single-operator mode by design, with a per-operator spawned-subprocess pool as the documented future direction.

Each project follows the standard project pattern: runtime binary (for server projects) or firmware binary (for the embedded leaf), port library or port crate workspace, pack manifest or cargo feature set, test scripts, launch scripts.

### 13.4 Embedded Leaf Architecture

The ESP32 deployment target represents the "body without a brain" endpoint of the architecture. The leaf runs a `no_std` firmware under 1 MB that exposes the same wire protocol as the server SOMA (`InvokeSkill`, `ListCapabilities`, `TransferRoutine`, `RemoveRoutine`, `Ping`) but hosts no control loop, no episodic memory, no skill selection, and no goal runtime. It is a pure dispatcher: incoming `InvokeSkill` messages are routed to registered port crates; `TransferRoutine` stores a linear sequence of primitive invocations that the leaf walks on subsequent invocations. All deliberation lives in the server SOMA that drives the leaf over TCP.

This split exploits the same brain/body separation the server architecture relies on. A leaf can be deployed to wildly constrained hardware (no filesystem, no dynamic linking, no heap growth, no standard library) because its role is not to decide — only to execute, sense, and adapt the hardware to commands from a capable peer. The leaf's observed state (pin assignments, I²C bus topology, stored routines) is exposed through the wire protocol so the brain has full proprioception over the body it drives.

**Port composition via cargo features.** Every hardware capability is a separate chip-agnostic crate in the workspace (`ports/gpio`, `ports/i2c`, `ports/display`, etc.). The firmware binary depends on each port as an `optional = true` dependency and exposes matching cargo features that the build system toggles. A minimal firmware (`gpio` + `delay` only) fits in ~150 KB; a full-featured build with wifi, storage, display, and runtime pin configuration lands around 750 KB. Port crates never reference a specific chip — the chip is selected exclusively through a top-level `chip-esp32`/`chip-esp32s3` feature that cascades to the appropriate `esp-hal` chip feature.

**Chip abstraction via uniform module interface.** Two files — `firmware/src/chip/esp32.rs` and `firmware/src/chip/esp32s3.rs` — implement a shared interface (`NAME`, `NAME_LOWER`, `TEST_LED_PIN`, `PinConfig`, `init_peripherals`, `register_all_ports`). `main.rs` resolves the active chip module via a `cfg`-gated alias (`chip::active`) and never references a specific chip. Adding a new chip (ESP32-S2, C3, C6, H2) is a four-step recipe: copy a module, register it in `mod.rs`, add a cargo feature, add a `chips/<chip>.toml` cargo config overlay. main.rs and the port crates remain untouched.

**Runtime-configurable pin assignments.** Pin numbers are not compile-time constants. At boot, each chip module calls `PinConfig::load(&FlashKvStore::new())`, which reads `pins.*` keys from a dedicated 4 KB flash sector and falls back to per-chip `DEFAULT_*` constants for missing keys. Pin dispatch is through `esp_hal::gpio::AnyPin::steal(n)`, which wraps a runtime `u8` into a typed pin handle. ADC is the one exception: `esp-hal`'s `AdcChannel` trait is only implemented for statically-known `GpioPin<N>`, so the chip module enumerates every ADC1-capable pin in a `match` and constructs a typed `Adc` instance in each arm. At runtime exactly one arm runs, so `peripherals.ADC1` is moved exactly once. Reconfiguring a pin — for example, moving I²C0 from GPIO 5/4 to GPIO 21/22 on a new board with a differently-wired OLED — is an MCP call to `board.configure_pin` followed by `board.reboot`. No reflash is required.

**Type-erased hardware injection.** Port crates that need hardware access (adc, pwm, board, display) take their hardware state as `Box<dyn FnMut(...)>` closures injected by the firmware at construction. The port crate stays free of `esp-hal` and any driver crates; the firmware owns the concrete driver and closes over it in the injected closures. This keeps the port crate chip-agnostic and, more importantly, keeps the workspace build matrix small — adding a new chip never touches port crates. The `display` port demonstrates the pattern at scale: its crate has zero `esp-hal` / `ssd1306` / `embedded-graphics` dependencies, yet the firmware constructs a fully working `Ssd1306` driver with `BufferedGraphicsMode` text rendering behind seven injected closures.

**Shared I²C bus via embedded-hal-bus.** The `i2c` port and the `display` port both consume I²C0 on the same physical pins. The firmware wraps the `esp-hal` I²C instance in a `RefCell` leaked to `'static` lifetime via `Box::leak` and hands each consumer its own `embedded_hal_bus::i2c::RefCellDevice`. The `I2cPort` crate is generic over any `embedded_hal::i2c::I2c` implementor, which makes the swap between "raw bus" and "shared bus" transparent at the port level. The leaf's dispatch loop is single-threaded, so `RefCell` is safe (no `critical_section::Mutex` needed).

**mDNS auto-discovery.** The server SOMA's `--discover-lan` flag spawns an mdns-sd browser for `_soma._tcp.local.` and registers discovered peers in the shared peer ID map. On the leaf side, an `edge-mdns` responder bound to a `smoltcp` UDP socket on 224.0.0.251:5353 announces the service record `soma-<chip>-<mac>._soma._tcp.local.` after DHCP assigns an address. The server's `invoke_remote_skill` MCP tool then reaches the leaf by derived peer ID without any static configuration. The full discovery→invoke cycle was verified against both chips on a real LAN.

**End-to-end proof of the cognitive body/dumb body split.** A brain-side Python loop running `scripts/thermistor-to-display.py` against the WROOM-32D reads the thermistor every 5 seconds via `invoke_skill thermistor.read_temp`, formats the temperature as a string, and writes it to the OLED via `invoke_skill display.draw_text`. The physical OLED displays the updating reading ("Temperature: 22.00 C" + ancillary lines). The leaf has no concept of "every 5 seconds" — it is the brain's cadence. The leaf has no concept of "read sensor, show on screen" — it is the brain's composition of two primitive invocations. The same loop runs through the server SOMA's MCP interface: an LLM calling `invoke_remote_skill` twice per tick produces identical behavior with zero firmware changes. This is the cleanest demonstration in the codebase of the architecture's central thesis: intelligence is in the brain, execution is in the body, and new "applications" (the read-sensor-show-temperature behavior) appear without writing any code on the body at all.

---

## 14. Historical Validation

The SOMA paradigm was originally validated through three proofs of work using an earlier prototype (`soma-core`) that employed neural inference — a BiLSTM encoder with a GRU autoregressive decoder mapping tokenized intents to sequences of function calls. The architecture has since evolved from neural program generation to deliberative goal-driven control, but the proofs validated the foundational principle that intent can map to execution without an intermediate code artifact.

### 14.1 POW 1 — Intent to Execution Without Code

A seq2seq neural network (~800K parameters) mapped natural language to sequences of libc calling conventions (open, read, write, opendir, readdir, stat, getcwd, uname, gettimeofday, and others) on macOS ARM64. A generic execution bridge dispatched calls through seven calling patterns (direct, buffered_read, write_bytes, struct_query, iterate, buffered_str, synapse_send) — analogous to CPU addressing modes. No function name appeared in the execution path. Adding a new function required only a catalog entry declaring its pattern.

The model generated multi-step programs with data dependencies (output of step *i* as input to step *j*) and correctly sequenced libc calls for filesystem operations, system queries, and cross-instance communication.

### 14.2 POW 2 — Experiential Learning

LoRA adapters (rank 8, alpha 2.0, 44,480 trainable parameters on 1,071,864 frozen base parameters) were applied to the decoder. The base model was deliberately trained on only 50% of intent templates, leaving 50% as novel phrasings. After 40 adaptation cycles on 72 experience samples, average confidence on 12 novel phrasings increased from 84.36% to 89.98% (+5.62%). The largest improvements occurred on intents with lowest baseline confidence (up to +17.1%). Resetting LoRA to zero returned all confidences to exact baseline values, confirming the improvement resided solely in the adapter weights.

### 14.3 POW 3 — Inter-Instance Communication

Two SOMA instances on the same host, each with its own model, discovered each other via presence broadcasting and exchanged data via TCP. The model treated `send_signal` as a body capability alongside libc functions. The routing decision (local EMIT vs. network SEND) was neural — the model learned during synthesis that intents containing "send to soma-b" produce programs ending with `send_signal`. All five protocol tests passed.

### 14.4 Architectural Evolution

The transition from neural inference (soma-core) to deliberative control (soma-next) preserved the core thesis — the runtime is the program — while changing the mechanism. In the neural prototype, a trained model *generated* execution sequences. In the current architecture, a control loop *selects* from declared skills. Both eliminate the intermediate code artifact. The deliberative architecture provides: explicit auditability (every decision is traceable through the session trace), safety enforcement (the policy engine gates every action), and transparent learning (episodes, schemas, and routines are inspectable data structures rather than opaque weight matrices).

---

## 15. Discussion

### 15.1 The Bootstrap Problem

SOMA eliminates application source code, but the runtime itself is written in Rust. This is the standard bootstrapping constraint: the first compiler must be written in another language. The SOMA runtime and its port adapters are the last programs that need to be hand-written for a given domain. Once operational, the runtime handles all domain behavior through pack manifests and loaded ports.

### 15.2 Cognitive Architecture, Not Neural Inference

The current SOMA architecture is a cognitive architecture — a structured system that perceives (observations), believes (belief state), decides (skill selection), acts (port invocation), and learns (episodic pipeline). It does not perform neural inference. The phrase "neural architecture" in SOMA's tagline refers to the cognitive architecture pattern: the runtime's structure mirrors a nervous system's function (sense → model → decide → act → adapt), not its implementation (neural networks).

This is a deliberate design choice. Neural inference provides generalization but sacrifices auditability, safety enforcement, and deterministic replay. Deliberative control provides full transparency — every decision is traceable, every policy gate is auditable, every learned routine can be inspected and invalidated. For a system that executes operations with real-world side effects (database writes, email sends, file deletions), auditability is a hard requirement.

### 15.3 Comparison with Existing Paradigms

| Aspect | Traditional Software | AI Code Generation | SOMA |
|---|---|---|---|
| **Artifact** | Source code | Source code (AI-generated) | Pack manifests (no code) |
| **Maintenance** | Manual edit-test-deploy | Regenerate (may break) | Update manifest or load port |
| **Context** | Developer's knowledge | LLM context window (lost) | Runtime state (permanent) |
| **Safety** | Unit tests, code review | Prompt guardrails | Policy engine, 7 lifecycle hooks |
| **Learning** | Manual refactoring | Re-prompting | Episodes → schemas → routines |
| **External systems** | Libraries, SDKs | Generated integration code | Typed port contracts |
| **Deployment** | Build pipeline | Build pipeline | Directory copy |

### 15.4 Coexistence

SOMA does not require replacing existing systems. Three coexistence models: (A) SOMA behind a traditional API via the HTTP port — transparent to existing clients, (B) SOMA orchestrating legacy services via port adapters, (C) gradual migration — one capability at a time becomes a SOMA port and skill.

SOMA is well-suited to: data-driven applications, CRUD operations, API orchestration, IoT automation, multi-service coordination. Traditional code remains preferable for: performance-critical inner loops, UI framework implementation, and systems requiring formal verification beyond what the policy engine provides.

### 15.5 What Counts as Code

The claim "no application source code" invites a fair objection: aren't pack manifests, SQL strings in skill schemas, and port configuration files just code in a different notation?

The SOMA runtime and port adapters are hand-written Rust (section 15.1). That is real code. Pack manifests are declarative configuration — they describe what the runtime should be able to do, not how. The how is the control loop, skill selection, belief updates, and policy enforcement. A Kubernetes manifest is not "application code" even though it describes behavior. A database schema is not "application code" even though it shapes what the application stores. Pack manifests are in this category: they parameterize a general-purpose engine, they don't contain execution logic.

The honest edge case is SQL strings embedded in skill declarations. These are domain-specific logic, and the boundary between "configuration parameter" and "code" is genuinely fuzzy here. The key distinction: they don't execute alone. A bare SQL string is inert data. It becomes operative only when the runtime's control loop selects a skill, binds parameters from belief state, dispatches through a port adapter, and the policy engine permits execution. The SQL is an input to a capability, not a standalone program.

The autonomous path eliminates even this residual ambiguity. When routines compile from episodes via PrefixSpan sequence mining, the "code" — the skill sequences that constitute the compiled procedure — was never written by anyone. It emerged from observed execution traces, clustered by embedding similarity, and generalized into reusable patterns. The developer authored nothing; the runtime learned the procedure from its own experience.

---

## 16. Limitations and Open Questions

### 16.1 Skill Authoring

Skills are currently declared in pack manifests as JSON. Complex composite skills with conditional branching require careful manual specification. Tooling for skill authoring, validation, and testing would lower the barrier to creating new applications.

### 16.2 Schema Induction Quality

Schema induction requires three or more successful episodes with the same goal fingerprint. Goals with high variance in phrasing may not cluster. The current fingerprinting uses longest-common-prefix matching, which is sensitive to word order. Semantic similarity matching would improve induction coverage but introduces embedding model dependencies.

### 16.3 Predictor Accuracy

The predictor uses exponential moving average calibration (alpha=0.1) from past observations. This is effective for stationary environments but may lag in environments where port behavior changes (e.g., database latency spikes). More sophisticated predictive models — Bayesian updating, contextual bandits — could improve selection quality at the cost of complexity.

### 16.4 Formal Verification

The deterministic control loop, finite skill hierarchy, typed observations, and explicit policy gates make SOMA more amenable to formal verification than LLM-based systems. However, the tools do not yet exist. The bounded execution model (budget constraints, step limits) provides termination guarantees, but correctness guarantees over arbitrary skill compositions remain an open problem.

### 16.5 Adversarial Inputs

An adversary who controls the MCP client can submit goals designed to exhaust budgets, trigger expensive port invocations, or exploit policy gaps. Defense layers include: the LLM's own refusal mechanisms, the policy engine's budget enforcement, per-action rate limiting, and the destructive-operation confirmation gate. A formal analysis of the attack surface — particularly the interaction between policy rules and composite skill execution — is needed.

### 16.6 Routine Staleness

Routines compiled from historical episodes may become stale as external system behavior changes. The current invalidation triggers (resource schema change, precondition failure, policy change, confidence drop) are reactive. Proactive staleness detection — periodic re-evaluation of routine confidence against recent episodes — would improve reliability.

---

## 17. Conclusion

SOMA is a computational architecture where the runtime is the program. A goal-driven control loop selects skills, invokes ports, observes outcomes, and adapts through an episodic learning pipeline — without generating, compiling, or interpreting application source code at any layer. The policy engine enforces safety at seven lifecycle hooks. The type system makes side effects, risk classes, trust levels, and determinism properties explicit and machine-evaluable. The distributed layer enables multi-instance delegation and knowledge transfer.

The implementation — 1261 tests, 11 port adapters, 88 capabilities, 29 MCP tools, and eleven deployed projects spanning LLM-driven server operation (smtp, s3, postgres, mcp, mcp-bridge), the autonomous learning path (multistep), cross-instance communication (s2s), in-browser runtime (web), end-user conversational product (terminal), and embedded leaf firmware on real ESP32 hardware (esp32) — demonstrates that the architecture is operational, not theoretical. The episodic learning pipeline (episodes → schemas → routines) provides a concrete mechanism for experience-based adaptation. The pack manifest pattern provides a concrete answer to "what do developers write instead of code."

SOMA does not generate code. It eliminates the need for it.

---

## References

Anderson, J.R. (1982). "Acquisition of Cognitive Skill." *Psychological Review*, 89(4), 369–406.

Anderson, J.R. et al. (2004). "An Integrated Theory of the Mind." *Psychological Review*, 111(4), 1036–1060.

Balog, M. et al. (2017). "DeepCoder: Learning to Write Programs." *ICLR 2017*.

Buehler, E.L. & Buehler, M.J. (2024). "X-LoRA: Mixture of Low-Rank Adapter Experts." *APL Machine Learning*, 2(2), 026119.

Devlin, J. et al. (2017). "RobustFill: Neural Program Learning under Noisy I/O." *ICML 2017*.

Hu, E.J. et al. (2021). "LoRA: Low-Rank Adaptation of Large Language Models." *ICLR 2022*.

Laird, J.E., Newell, A. & Rosenbloom, P.S. (1987). "SOAR: An Architecture for General Intelligence." *Artificial Intelligence*, 33(1), 1–64.

Li, Y. et al. (2022). "Competition-Level Code Generation with AlphaCode." *Science*, 378(6624).

McClelland, J.L., McNaughton, B.L. & O'Reilly, R.C. (1995). "Why There Are Complementary Learning Systems in the Hippocampus and Neocortex." *Psychological Review*, 102(3), 419–457.

Patil, S. et al. (2023). "Gorilla: Large Language Model Connected with Massive APIs." arXiv:2305.15334.

Qin, Y. et al. (2023). "ToolLLM: Facilitating Large Language Models to Master 16000+ Real-world APIs." arXiv:2307.16789.

Rao, A.S. & Georgeff, M.P. (1995). "BDI Agents: From Theory to Practice." *Proceedings of the First International Conference on Multi-Agent Systems (ICMAS-95)*.

Schick, T. et al. (2023). "Toolformer: Language Models Can Teach Themselves to Use Tools." *NeurIPS 2023*.
