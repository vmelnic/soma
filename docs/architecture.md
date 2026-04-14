# Architecture

soma-next is a goal-driven runtime. A caller submits a typed goal; the runtime selects skills, invokes ports, observes results, patches beliefs, and iterates until the goal is satisfied or budget is exhausted. The entire system compiles to a single binary.

## Layers

The runtime is organized into six layers, from innermost to outermost.

### 1. Runtime Logic

The core deliberation and execution engine. All components are trait-based and injected into the `SessionController` at bootstrap time.

| Component | Role |
|---|---|
| **SessionController** | Orchestrates the 16-step control loop per session. Owns all sessions. |
| **BeliefSource** | Builds initial belief state from a goal; applies patches after observations. |
| **SkillRegistry** | Enumerates and looks up skill candidates given goal, belief, schemas, and routines. |
| **SkillExecutor** | Binds inputs, checks preconditions, executes skills, invokes rollback. |
| **CandidatePredictor** | Scores skill candidates on predicted success, cost, latency, and information gain. |
| **Critic** | Evaluates observations and decides loop control flow (continue, revise, backtrack, delegate, stop). |
| **PolicyEngine** | Gates actions at 7 lifecycle hooks. Enforces budget, destructive-op confirmation, trust, and scope. |
| **GoalRuntime** | Parses user input into a validated `GoalSpec`. |
| **PortRuntime** | Manages port lifecycle (declare, load, validate, activate, quarantine, retire). Dispatches invocations with auth, policy, and sandbox checks. |
| **SkillRuntime** | Registers and resolves skills by ID, namespace, or tag. |
| **PackRuntime** | Validates pack manifests, checks dependency graphs, manages pack lifecycle states. |
| **ResourceRuntime** | Tracks typed resources with identity, versioning, and mutability rules. |
| **RuntimeMetrics** | Prometheus-style counters for sessions, steps, port calls, policy denials, skill invocations. |
| **Proprioception** | `SelfModel` snapshots: RSS, CPU, active sessions, loaded packs/skills/ports, uptime, peer count. |

### 2. Adapter Layer

Bridges standalone runtime components into the `SessionControllerDeps` trait system. Each adapter wraps one subsystem:

| Adapter | Wraps |
|---|---|
| `SimpleBeliefSource` | `DefaultBeliefRuntime` |
| `SkillRegistryAdapter` | `DefaultSkillRuntime` |
| `PortBackedSkillExecutor` | `DefaultPortRuntime` (maps skill capability requirements to port invocations) |
| `EpisodeMemoryAdapter` | `Arc<Mutex<dyn EpisodeStore>>` |
| `SchemaMemoryAdapter` | `Arc<Mutex<dyn SchemaStore>>` |
| `RoutineMemoryAdapter` | `Arc<Mutex<dyn RoutineStore>>` |
| `PolicyEngineAdapter` | `DefaultPolicyRuntime` + `max_steps` config |
| `SimpleCandidatePredictor` | Heuristic scorer (cost/latency/risk weighting) |
| `SimpleSessionCritic` | Success/budget/step-count evaluator |

### 3. Memory Stores

Three append-oriented stores, each with an in-memory default and a disk-backed variant.

| Store | Type | Purpose |
|---|---|---|
| **EpisodeStore** | `Episode` | Full session traces (steps, observations, outcome). Retrieved by goal fingerprint similarity. |
| **SchemaStore** | `Schema` | Reusable abstract control structures (subgoal graphs, skill orderings). Induced from repeated episodes or authored in packs. |
| **RoutineStore** | `Routine` | Compiled habitual shortcuts. High-confidence, bounded, deterministic. Bypass deeper deliberation. |

Disk persistence uses JSON files in a configurable `data_dir`. Each `Disk*Store` wraps the in-memory default, writes through on every mutation, and loads from disk on construction.

### 4. Interfaces

**CLI** (10 commands):

| Command | Action |
|---|---|
| `run` | Submit and execute a goal to completion |
| `inspect` | Show session state by ID |
| `restore` | Restore and resume a checkpointed session |
| `list_sessions` | List all sessions with status |
| `list_packs` | Show loaded pack manifests |
| `list_skills` | Enumerate registered skills |
| `metrics` | Dump runtime metrics (text, JSON, or Prometheus format) |
| `verify_port` | Verify Ed25519 signature of a port library |
| `dump` | Structured JSON dump for LLM context (sections: full, belief, episodes, schemas, routines, sessions, skills, ports, packs, metrics) |
| `repl` | Interactive mode |

**MCP Server** (27 tools, JSON-RPC 2.0):

*16 core*: `create_goal`, `inspect_session`, `inspect_belief`, `inspect_resources`, `inspect_packs`, `inspect_skills`, `inspect_trace`, `pause_session`, `resume_session`, `abort_session`, `list_sessions`, `query_metrics`, `query_policy`, `dump_state`, `invoke_port`, `list_ports`.

*3 distributed*: `list_peers`, `invoke_remote_skill`, `transfer_routine`.

*3 scheduler*: `schedule`, `list_schedules`, `cancel_schedule`.

*3 world state*: `patch_world_state`, `dump_world_state`, `set_routine_autonomous`.

*2 execution*: `trigger_consolidation`, `execute_routine`.

Also supports the MCP protocol methods `initialize`, `tools/list`, and `tools/call`.

### 5. Distributed Transport

Peer-to-peer communication for multi-instance coordination.

Modules: `transport` (TCP/TLS wire protocol with typed message envelopes), `ws_transport` (WebSocket), `unix_transport` (Unix Domain Socket), `peer` (registry and identity), `routing` (message routing), `delegation` (remote skill dispatch), `remote` (`RemoteExecutor` trait), `chunked` (resumable transfers with SHA-256), `streaming` (frame management), `rate_limit`, `heartbeat`, `queue` (offline messages), `sync` (state synchronization), `auth` (peer authentication), `trace` (distributed trace propagation).

Wire protocol message types: InvokeSkill, QueryResource, SubmitGoal, TransferSchema, TransferRoutine, ChunkedTransferStart.

### 6. Ports

Typed interfaces to external systems. Two built-in ports are instantiated directly; all others load dynamically.

| Port | Kind | Capabilities |
|---|---|---|
| `FilesystemPort` | Filesystem | File/directory operations |
| `HttpPort` | Http | HTTP client requests |
| Dynamic (`.dylib`/`.so`) | Any `PortKind` | Loaded from `plugin_path` directories via `DynamicPortLoader`, optionally requiring Ed25519 signatures |

---

## Type System

### Classification Enums

| Enum | Values | Purpose |
|---|---|---|
| `SideEffectClass` | None, ReadOnly, LocalStateMutation, ExternalStateMutation, Destructive, Irreversible | Classifies what a port or skill does to the world |
| `RiskClass` | Negligible, Low, Medium, High, Critical | Risk rating for skills and capabilities |
| `DeterminismClass` | Deterministic, PartiallyDeterministic, Stochastic, DelegatedVariant | Whether repeated calls yield the same result |
| `CostClass` | Negligible, Low, Medium, High, Extreme | Resource consumption rating (used in `CostProfile` for cpu, memory, io, network, energy) |
| `TrustLevel` | Untrusted, Restricted, Verified, Trusted, BuiltIn | Ordered trust classification for ports and packs |
| `CapabilityScope` | Local, Session, Tenant, Device, Peer, Public | Ordered scope breadth (Local=0 through Public=5). Enforced at dispatch time. |
| `IdempotenceClass` | Idempotent, NonIdempotent, ConditionallyIdempotent | Whether a capability can be safely retried |
| `RollbackSupport` | FullReversal, CompensatingAction, LogicalUndo, Irreversible | What undo mechanism is available |
| `CriticDecision` | Continue, Revise, Backtrack, Delegate, Stop | Control flow after observing a skill result |
| `FactProvenance` | Asserted, Observed, Inferred, Stale, Remote | Where a belief fact originated |
| `EffectType` | Creation, Update, Deletion, Emission, Scheduling, Notification, Delegation, Synchronization | What kind of side effect a skill produces |
| `TerminationType` | Success, Failure, Timeout, BudgetExhaustion, PolicyDenial, ExternalError, ExplicitAbort | Why a session or skill terminated |

### Port Failure Classes

| `PortFailureClass` | Meaning |
|---|---|
| ValidationError | Input failed schema check |
| AuthorizationDenied | Auth requirements not met |
| SandboxViolation | Sandbox constraints breached |
| PolicyDenied | Policy engine rejected the call |
| Timeout | Port call exceeded time limit |
| DependencyUnavailable | Required external service unreachable |
| TransportError | Network/transport-level failure |
| ExternalError | External system returned an error |
| PartialSuccess | Some effects applied, others did not |
| RollbackFailed | Compensation action failed |
| Unknown | Unclassified |

### Skill Failure Classes

| `SkillFailureClass` | Meaning |
|---|---|
| ValidationFailure | Skill-level input validation failed |
| PreconditionFailure | Declared preconditions not met |
| PolicyDenial | Policy engine blocked execution |
| BindingFailure | Required inputs could not be bound |
| PortFailure | Underlying port call failed |
| RemoteFailure | Delegated execution failed |
| Timeout | Execution time exceeded |
| BudgetExhaustion | Budget depleted mid-execution |
| PartialSuccess | Some effects applied (requires `partial_success_behavior` declaration) |
| RollbackFailure | Compensation action failed |
| Unknown | Unclassified |

### Invocation Outcome Types

These are recorded on every `PortCallRecord` for tracing:

| Type | Variants |
|---|---|
| `AuthOutcome` | Passed, Failed { reason }, NotRequired |
| `PolicyOutcome` | Allowed, Denied { reason }, RequiresConfirmation { reason } |
| `SandboxOutcome` | Satisfied, Violated { dimension, reason }, NotEnforced |

### Shared Structures

| Struct | Key Fields | Purpose |
|---|---|---|
| `LatencyProfile` | expected, p95, max (ms) | Latency bounds |
| `CostProfile` | cpu, memory, io, network, energy (CostClass each) | Multi-dimensional resource cost |
| `Budget` | risk_remaining, latency_remaining_ms, resource_remaining, steps_remaining | Per-session execution budget |
| `SchemaRef` | schema (JSON Value) | JSON Schema for input/output validation |
| `AuthRequirements` | methods, required | Auth methods: BearerToken, ApiKey, MTls, SignedCapabilityToken, LocalProcessTrust, DeviceAttestation, PeerIdentityTrust |
| `SandboxRequirements` | 8 dimensions | filesystem, network, device, process access + memory/cpu/time/syscall limits |
| `Precondition` | condition_type, expression, description | Guard that must hold before execution |
| `TerminationCondition` | condition_type, expression, description | When to end execution |
| `EffectDescriptor` | effect_type, target_resource, description, patch | Declared side effect |

---

## Skill Contract

A `SkillSpec` is the canonical declaration of an executable capability. Skills do not contain implementation -- they declare contracts that the runtime enforces and bind to ports for execution.

### SkillKind

| Kind | Behavior |
|---|---|
| `Primitive` | Single port invocation. The skill executor maps `capability_requirements` to a port call. |
| `Composite` | Ordered subskill graph. The session controller iterates `subskills`, evaluating `branch_condition` and `stop_condition` per step. Aggregate observation: success only if all required subskills succeed; latency summed; confidence is minimum. |
| `Routine` | Compiled shortcut from episodes/schemas. Match conditions re-validated at execution time. Confidence threshold enforced against current belief state. Falls back to full deliberation on invalidation. |
| `Delegated` | Dispatched to a remote peer via `RemoteExecutor` when `remote_endpoint` is set. Falls back to local execution when no remote executor is configured. |

### SkillSpec Fields

| Field | Type | Required | Purpose |
|---|---|---|---|
| `skill_id` | String | yes | Unique identifier |
| `namespace` | String | yes | Namespace (typically pack ID) |
| `pack` | String | yes | Owning pack |
| `kind` | SkillKind | yes | Primitive, Composite, Routine, or Delegated |
| `name` | String | yes | Human-readable name |
| `description` | String | yes | What the skill does |
| `version` | String | yes | Semver |
| `inputs` | SchemaRef | yes | JSON Schema for required inputs |
| `outputs` | SchemaRef | yes | JSON Schema for outputs |
| `required_resources` | Vec\<String\> | yes | Resource IDs needed |
| `preconditions` | Vec\<Precondition\> | yes | Guards checked before execution |
| `expected_effects` | Vec\<EffectDescriptor\> | yes | Declared side effects |
| `observables` | Vec\<ObservableDecl\> | yes | Fields to monitor in observation results (roles: ConfirmSuccess, DetectPartialSuccess, DetectAmbiguity, UpdateConfidence, General) |
| `termination_conditions` | Vec\<TerminationCondition\> | yes | When to consider execution complete |
| `rollback_or_compensation` | RollbackSpec | yes | Rollback support level + optional compensation skill |
| `cost_prior` | CostPrior | yes | Expected latency and resource cost |
| `risk_class` | RiskClass | yes | Risk rating |
| `determinism` | DeterminismClass | yes | Reproducibility class |
| `remote_exposure` | RemoteExposureDecl | yes | Remote accessibility (scope, trust, rate limits, replay protection, delegation, observation streaming) |
| `tags` | Vec\<String\> | no | Searchable tags |
| `aliases` | Vec\<String\> | no | Alternative names |
| `capability_requirements` | Vec\<String\> | no | Port capabilities needed (e.g. `port:filesystem/readdir`) |
| `subskills` | Vec\<SubskillRef\> | no | Composite skill graph (each ref has skill_id, ordering, required, branch_condition, stop_condition) |
| `guard_conditions` | Vec\<Precondition\> | no | Additional guards beyond preconditions |
| `match_conditions` | Vec\<Precondition\> | no | Conditions for routine matching |
| `confidence_threshold` | Option\<f64\> | no | Minimum belief confidence for routine execution |
| `locality` | Option\<SkillLocality\> | no | LocalOnly, RemoteAllowed, or RemotePreferred |
| `remote_endpoint` | Option\<String\> | no | Target peer for delegated skills |
| `fallback_skill` | Option\<String\> | no | Skill to try when a routine is invalidated |
| `invalidation_conditions` | Vec\<String\> | no | Conditions that invalidate a routine |
| `nondeterminism_sources` | Vec\<String\> | no | What makes this skill nondeterministic (required when determinism is Stochastic or PartiallyDeterministic) |
| `partial_success_behavior` | Option\<PartialSuccessDetail\> | no | Declares effects_occurred, effects_missing, compensation_possible, downstream_continuation |
| `policy_overrides` | Vec\<String\> | no | Pack-level policy override references |
| `telemetry_fields` | Vec\<String\> | no | Additional fields to record in telemetry |
| `remote_trust_requirement` | Option\<String\> | no | Trust level required for remote callers (delegated skills) |
| `remote_capability_contract` | Option\<String\> | no | Capability contract for remote delegation |

---

## Port Contract

A `PortSpec` declares a typed interface to one external system. The `Port` trait is what adapters implement.

### Port Trait

```rust
pub trait Port: Send + Sync {
    fn spec(&self) -> &PortSpec;
    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Result<PortCallRecord>;
    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> Result<()>;
    fn lifecycle_state(&self) -> PortLifecycleState;
}
```

The runtime calls `validate_input` before `invoke`. Implementations handle external failures and classify them into `PortFailureClass`.

### PortSpec Fields

| Field | Type | Purpose |
|---|---|---|
| `port_id` | String | Unique identifier |
| `name` | String | Human-readable name |
| `version` | Version (semver) | Port version |
| `kind` | PortKind | Filesystem, Database, Http, Queue, Renderer, Sensor, Actuator, Messaging, DeviceTransport, Custom |
| `description` | String | What the port connects to |
| `namespace` | String | Namespace |
| `trust_level` | TrustLevel | Trust classification |
| `capabilities` | Vec\<PortCapabilitySpec\> | Individual operations this port exposes |
| `input_schema` | SchemaRef | Port-level input schema |
| `output_schema` | SchemaRef | Port-level output schema |
| `failure_modes` | Vec\<PortFailureClass\> | Declared failure classes |
| `side_effect_class` | SideEffectClass | Port-level side effect classification |
| `latency_profile` | LatencyProfile | Expected latency |
| `cost_profile` | CostProfile | Resource cost |
| `auth_requirements` | AuthRequirements | Authentication requirements |
| `sandbox_requirements` | SandboxRequirements | 8 sandbox dimensions |
| `observable_fields` | Vec\<String\> | Fields to monitor |
| `validation_rules` | Vec\<ValidationRule\> | Per-field validation |
| `remote_exposure` | bool | Whether the port is remotely accessible |

### PortCapabilitySpec

Each capability within a port carries its own classification:

| Field | Type |
|---|---|
| `capability_id` | String |
| `name` | String |
| `purpose` | String |
| `input_schema` | SchemaRef |
| `output_schema` | SchemaRef |
| `effect_class` | SideEffectClass |
| `rollback_support` | RollbackSupport |
| `determinism_class` | DeterminismClass |
| `idempotence_class` | IdempotenceClass |
| `risk_class` | RiskClass |
| `latency_profile` | LatencyProfile |
| `cost_profile` | CostProfile |
| `remote_exposable` | bool |
| `auth_override` | Option\<AuthRequirements\> |

### Port Lifecycle

States: `Declared` -> `Loaded` -> `Validated` -> `Active` -> `Degraded` | `Quarantined` | `Retired`.

A port enters `Declared` when its spec is registered. `register_port` advances through Declared -> Loaded -> Validated atomically. `activate` moves to Active. Quarantined ports cannot be invoked. Retired ports are removed from dispatch.

### InvocationContext

Every port invocation carries an `InvocationContext` for tracing and access control:

| Field | Type | Purpose |
|---|---|---|
| `session_id` | Option\<Uuid\> | Originating session |
| `goal_id` | Option\<String\> | Associated goal |
| `caller_identity` | Option\<String\> | Local session ID or remote peer ID |
| `remote_caller` | bool | Whether this is a remote invocation |
| `pack_id` | Option\<String\> | Pack owning the triggering skill |
| `calling_pack_id` | Option\<String\> | Pack making the call (for cross-pack isolation) |

### PortCallRecord

Every invocation produces a `PortCallRecord`, even on failure:

| Field | Type |
|---|---|
| `observation_id` | Uuid |
| `port_id`, `capability_id` | String |
| `invocation_id` | Uuid |
| `success` | bool |
| `failure_class` | Option\<PortFailureClass\> |
| `raw_result`, `structured_result` | JSON Value |
| `effect_patch` | Option\<JSON Value\> |
| `side_effect_summary` | Option\<String\> |
| `latency_ms` | u64 |
| `resource_cost` | f64 |
| `confidence` | f64 |
| `retry_safe` | bool |
| `input_hash` | Option\<String\> (SHA-256) |
| `session_id`, `goal_id`, `caller_identity` | Tracing fields from InvocationContext |
| `auth_result` | Option\<AuthOutcome\> |
| `policy_result` | Option\<PolicyOutcome\> |
| `sandbox_result` | Option\<SandboxOutcome\> |

---

## Pack Contract

A `PackSpec` is the deployment unit. It bundles ports, skills, schemas, routines, policies, and resources into a versioned, namespaced manifest.

### PackSpec Fields

| Field | Type | Required | Purpose |
|---|---|---|---|
| `id` | String | yes | Unique pack identifier |
| `name` | String | yes | Human-readable name |
| `version` | Version (semver) | yes | Pack version |
| `runtime_compatibility` | VersionReq | yes | Required soma-next version range |
| `namespace` | String | yes | Namespace for all contained items |
| `capabilities` | Vec\<CapabilityGroup\> | yes | Grouped capabilities with scope |
| `dependencies` | Vec\<DependencySpec\> | yes | Other packs this pack depends on (with version range, required flag, capability needs, feature flags) |
| `resources` | Vec\<ResourceSpec\> | yes | Typed resources with identity rules, versioning, mutability |
| `skills` | Vec\<SkillSpec\> | yes | Skills this pack provides |
| `schemas` | Vec\<Schema\> | yes | Control structure schemas |
| `routines` | Vec\<Routine\> | yes | Compiled routines |
| `policies` | Vec\<PolicySpec\> | yes | Policy contributions |
| `exposure` | ExposureSpec | yes | What is locally/remotely exposed. Default deny for destructive capabilities. |
| `observability` | ObservabilitySpec | yes | 9 fields: health_checks, version_metadata, dependency_status, capability_inventory, expected_latency_classes, expected_failure_modes, trace_categories, metric_names, pack_load_state |
| `ports` | Vec\<PortSpec\> | no | Ports declared inline |
| `port_dependencies` | Vec\<PortDependency\> | no | Required port versions |
| `description`, `authors`, `license`, `homepage`, `repository` | metadata | no | Package metadata |
| `targets`, `build`, `checksum`, `signature`, `entrypoints`, `tags`, `deprecation` | various | no | Build, integrity, and lifecycle metadata |

### Pack Lifecycle

States: `Discovered` -> `Validated` -> `Staged` -> `Active` -> `Degraded` | `Quarantined` | `Suspended` | `Unloaded` | `Failed`.

### Pack Failure Classes

ManifestFailure, SchemaFailure, DependencyFailure, NamespaceCollision, PolicyFailure, PortFailure, SkillExecutionFailure, RemotePeerFailure, IntegrityFailure.

### Bootstrap

The `bootstrap()` function assembles a `Runtime` from config and pack manifest paths:

1. Configure the port runtime with a sandbox profile (host capabilities).
2. For each pack manifest path: parse JSON, register ports (create adapters by `PortKind`), register skills.
3. Resolve the data directory. Create memory stores (in-memory if no `data_dir`; disk-backed otherwise).
4. Build all adapter instances (belief, episode, schema, routine, skill registry, skill executor, predictor, critic).
5. Build the policy engine. Register host-level default safety policies. Register pack-declared policies (rejected on conflict).
6. Assemble `SessionControllerDeps` and construct `SessionController`.
7. Return the `Runtime` struct containing: session controller, goal runtime, skill runtime, port runtime, all memory stores, pack specs, metrics, and start time.

Built-in port adapters are instantiated by `PortKind`: Filesystem and Http are created directly. All other kinds fall through to `DynamicPortLoader`, which searches configured directories for `libsoma_port_<port_id>` shared libraries.

---

## Session Lifecycle

### GoalSpec

A `GoalSpec` is the typed input to the session controller:

| Field | Type | Purpose |
|---|---|---|
| `goal_id` | Uuid | Unique identifier |
| `source` | GoalSource | Origin: User, Api, Mcp, Peer, Scheduler, or Internal |
| `objective` | Objective | Description string + optional structured JSON |
| `constraints` | Vec\<Constraint\> | Restrictions on how the goal may be achieved |
| `success_conditions` | Vec\<SuccessCondition\> | What constitutes success |
| `risk_budget` | f64 | Maximum risk the session may spend |
| `latency_budget_ms` | u64 | Maximum wall-clock time |
| `resource_budget` | f64 | Maximum resource expenditure |
| `deadline` | Option\<DateTime\> | Hard deadline |
| `permissions_scope` | Vec\<String\> | Allowed capability scopes |
| `priority` | Priority | Low, Normal, High, Critical |

### ControlSession

The primary execution unit:

| Field | Type |
|---|---|
| `session_id` | Uuid |
| `goal` | GoalSpec |
| `belief` | BeliefState |
| `working_memory` | WorkingMemory (active bindings, unresolved slots, current subgoal, recent observations, candidate shortlist, branch state, budget deltas, output bindings) |
| `status` | SessionStatus |
| `trace` | SessionTrace (Vec\<TraceStep\>) |
| `budget_remaining` | Budget |
| `created_at`, `updated_at` | DateTime |

### SessionStatus

Created -> Running -> (Paused | WaitingForInput | WaitingForRemote | BlockedByPolicy) -> Completed | Failed | Aborted.

### The 16-Step Control Loop

Each call to `run_step()` executes one iteration:

| Step | Action |
|---|---|
| 1 | **Validate budget** -- check risk, latency, resource, and step counts. Check deadline. |
| 2 | **Capture belief snapshot** -- serialize current belief for trace. |
| 3 | **Retrieve episodes** -- query episode memory for nearest matches (limit 5). |
| 4 | **Retrieve schemas** -- find schemas whose trigger conditions match. |
| 5 | **Retrieve routines** -- find routines whose match/guard conditions are satisfied. |
| 6 | **Enumerate candidates** -- ask skill registry for valid candidates given goal, belief, schemas, and routines. Policy check at `BeforeCandidateSelection` hook. |
| 7 | **Prepare bindings** -- clear unresolved slots in working memory. |
| 8 | **Score candidates** -- predictor scores all candidates (predicted_success, predicted_cost, predicted_latency_ms, information_gain). |
| 9 | **Predict top candidates** -- select top 3 by score. |
| 10 | **Choose candidate** -- select the highest-scored skill. Run `check_skill_execution` policy gate. |
| 11 | **Execute skill lifecycle** -- the 8-step inner sequence (see below). |
| 12 | **Deduct budget** -- subtract latency and resource cost from remaining budget. Cost profile reduced to weighted scalar (CPU 0.3, memory 0.2, IO 0.2, network 0.2, energy 0.1). |
| 13 | **Evaluate observables** -- check declared observables against the observation result. Refine confidence, detect ambiguity or partial success. |
| 14 | **Critic evaluation** -- critic decides: Continue, Revise, Backtrack, Delegate, or Stop. |
| 15 | **Failure recovery** -- if the observation failed, `handle_failure` decides: Retry, SwitchCandidate, Backtrack, Delegate, or Stop. Based on failure class and remaining budget. |
| 16 | **Record trace and update state** -- persist the trace step, update session status and metrics. |

### Skill Lifecycle (Inner 8 Steps)

Executed in step 11 of the control loop via `execute_skill_lifecycle`:

1. **Capability scope enforcement** -- verify skill's declared scope is broad enough for the session's invocation context (User/Internal -> Local, Api/Mcp -> Session, Peer -> Peer).
2. **Bind inputs** -- draw values from goal fields, belief resources, prior observations, working memory, or pack defaults. Each binding records its `BindingSource`.
3. **Validate preconditions** -- evaluate all declared preconditions against current belief and working memory. Routine skills additionally re-validate `match_conditions` and `confidence_threshold`.
4. **Authorize** -- run policy hooks: `BeforeBindingFinalInputs`, `BeforeSideEffectingStep` (if skill has non-emission effects), `BeforeDelegation` (if Delegated kind), `BeforeRemoteExposure` (if remote_exposure enabled), `BeforeExecutionBegins`.
5. **Execute** -- dispatch by kind: Primitive -> port invocation, Composite -> subskill iteration, Delegated -> remote executor or local fallback, Routine -> same as Primitive. Undeclared PartialSuccess is downgraded to Unknown.
6. **Collect observation** -- store observation ID in working memory. Build output bindings from structured result keys (preserving source skill and confidence provenance).
7. **Apply belief patch** -- add observed facts to belief state.
8. **Evaluate termination** -- check declared termination conditions. If the skill failed and rollback is available, run `BeforeRollback` policy hook then `invoke_rollback`.

### Failure Recovery

| `SkillFailureClass` | Recovery Action |
|---|---|
| Timeout, BudgetExhaustion | Stop |
| PolicyDenial | Stop |
| RollbackFailure | Stop |
| BindingFailure, PreconditionFailure | SwitchCandidate |
| PortFailure, RemoteFailure | Retry (if budget allows), else SwitchCandidate |
| Unknown, ValidationFailure, PartialSuccess | Backtrack |

---

## Policy Engine

### PolicySpec

Policies are contributed by packs or the host. Each has:

| Field | Type | Purpose |
|---|---|---|
| `policy_id` | String | Unique identifier |
| `namespace` | String | "host" namespace has highest precedence |
| `rules` | Vec\<PolicyRule\> | Ordered rules |
| `allowed_capabilities` | Vec\<String\> | Explicitly allowed patterns |
| `denied_capabilities` | Vec\<String\> | Explicitly denied patterns |
| `scope_limits` | Option\<CapabilityScope\> | Maximum scope |
| `trust_classification` | Option\<TrustLevel\> | Required trust level |
| `confirmation_requirements` | Vec\<String\> | Capabilities requiring confirmation |
| `destructive_action_constraints` | Vec\<String\> | Constraints on destructive ops |
| `remote_exposure_limits` | Vec\<String\> | Remote exposure limits |

### PolicyRule

| Field | Type | Values |
|---|---|---|
| `rule_id` | String | Unique identifier |
| `rule_type` | PolicyRuleType | Allow, Deny, RequireConfirmation, RequireEscalation, ConstrainBudget, DowngradeTrust, ForceDelegationRejection, RateLimit |
| `target` | PolicyTarget | target_type (Skill, Port, Resource, Pack, Peer, Session, Capability), identifiers, scope, trust_level |
| `effect` | PolicyEffect | Allow, Deny, Constrain, RequireConfirmation, RequireEscalation |
| `conditions` | Vec\<PolicyCondition\> | Condition expressions |
| `priority` | i32 | Higher wins |

### Policy Hooks

The session controller calls the policy engine at 7 lifecycle points:

| Hook | When |
|---|---|
| `BeforeCandidateSelection` | Before finalizing the candidate list |
| `BeforeBindingFinalInputs` | After inputs are bound, before execution |
| `BeforeExecutionBegins` | Final gate before the executor runs |
| `BeforeSideEffectingStep` | Before skills with non-emission effects |
| `BeforeDelegation` | Before dispatching to a remote peer |
| `BeforeRollback` | Before invoking compensation |
| `BeforeRemoteExposure` | Before exposing capabilities remotely |

### Default Host Safety Policies

Registered at bootstrap under the "host" namespace:

1. **Destructive operation gate** -- `RequireConfirmation` for all skills when trust is at or below Verified. Triggered by Destructive or Irreversible side-effect class.
2. **Read-only allowance** -- `Allow` for all read-only operations unconditionally.
3. **Budget enforcement** -- handled directly by `PolicyEngineAdapter::check_budget()` (step count and resource depletion).
4. **Bounded loops** -- deny when step count exceeds `max_steps` from config.

Pack policies cannot widen what host policies restrict.

---

## Memory System

### Episodes

An `Episode` is a complete trace of a finished session:

| Field | Type |
|---|---|
| `episode_id` | Uuid |
| `goal_fingerprint` | String |
| `initial_belief_summary` | JSON |
| `steps` | Vec\<EpisodeStep\> (step_index, belief_summary, candidates, scores, selected skill, observation, belief patch, progress delta, critic decision) |
| `observations` | Vec\<Observation\> |
| `outcome` | EpisodeOutcome (Success, Failure, PartialSuccess, Aborted, Timeout, BudgetExhausted) |
| `total_cost` | f64 |
| `success` | bool |
| `tags` | Vec\<String\> |
| `embedding` | Option\<Vec\<f32\>\> |

Retrieval is by goal fingerprint similarity (longest common prefix in default implementation; production should use embeddings).

### Schemas

A `Schema` is an abstract control structure:

| Field | Type |
|---|---|
| `schema_id` | String |
| `namespace`, `pack`, `name` | String |
| `version` | Version (semver) |
| `trigger_conditions` | Vec\<Precondition\> |
| `resource_requirements` | Vec\<String\> |
| `subgoal_structure` | Vec\<SubgoalNode\> (subgoal_id, description, skill_candidates, dependencies, optional) |
| `candidate_skill_ordering` | Vec\<String\> |
| `stop_conditions` | Vec\<Precondition\> |
| `rollback_bias` | RollbackBias (Eager, Cautious, Minimal, None) |
| `confidence` | f64 |

Schemas can be authored in packs or induced from repeated episodes via `SchemaStore::induce_from_episodes`.

### Routines

A `Routine` is a compiled habitual shortcut:

| Field | Type |
|---|---|
| `routine_id` | String |
| `namespace` | String |
| `origin` | RoutineOrigin (PackAuthored, EpisodeInduced, SchemaCompiled, PeerTransferred) |
| `match_conditions` | Vec\<Precondition\> |
| `compiled_skill_path` | Vec\<String\> |
| `guard_conditions` | Vec\<Precondition\> |
| `expected_cost` | f64 |
| `expected_effect` | Vec\<EffectDescriptor\> |
| `confidence` | f64 |

Routines bypass deliberation when match confidence and policy allow. They can be compiled from schemas (`compile_from_schema`), invalidated by condition (`invalidate_by_condition`), and transferred between peers.

### Invalidation Reasons

| Reason | Behavior |
|---|---|
| ResourceSchemaChanged | Invalidate routines referencing the changed resource |
| PreconditionsNoLongerHold | Invalidate all routines |
| PolicyChanged | Conservative: invalidate all routines |
| PackVersionBreak | Invalidate routines whose skill path contains removed skill FQNs |
| ConfidenceDropped | Invalidate routines below the threshold |

### Persistence

When `data_dir` is configured, each store writes through to a JSON file on disk after every mutation. Files:
- `{data_dir}/episodes.json`
- `{data_dir}/schemas.json`
- `{data_dir}/routines.json`

On construction, existing data is loaded from disk. When `data_dir` is empty, stores are purely in-memory.

---

## Deployment Targets

The same architecture compiles to three deployment targets. The runtime logic, memory stores, and policy engine are source-compatible across all three; what changes is which ports load and which transports run.

### 1. Server (default)

A `~10 MB` `std` binary loaded from `cargo build --release -p soma-next`. Hosts the full 6-layer stack including the episodic memory pipeline (ring buffer → schemas → routines), the distributed transport layer, and all dynamically loaded `.dylib` / `.so` ports. This is the path every `soma-project-*` outside of `soma-project-esp32` uses. Cross-compiles cleanly to `aarch64-linux-android` (`10 MB` ELF via `cargo-ndk`) and `aarch64-apple-ios` (`9 MB` Mach-O via `xcrun`) with no source changes.

### 2. Embedded leaf (`no_std`)

The `soma-project-esp32` firmware is a `no_std` `cargo` workspace that deploys onto microcontrollers. It hosts a **leaf**, not a full runtime: the wire protocol surface (`InvokeSkill`, `ListCapabilities`, `TransferRoutine`, `RemoveRoutine`, `Ping`) and a composite dispatcher over a set of chip-agnostic port crates. There is no control loop, no skill selection, no goal runtime, no episodic memory, and no policy engine — all deliberation lives in a server SOMA reachable over TCP.

This "cognitive body / dumb body" split exploits the same brain/body separation the server architecture relies on. The leaf executes, senses, and adapts the hardware to commands from a capable peer but never decides what to do next.

**Key properties of the embedded leaf:**

| Property | Detail |
|---|---|
| Memory | `no_std`, single global allocator (`esp-alloc 0.6` with a vendored zero-byte guard), 96 KB heap when wifi is on. |
| Chip selection | Mutually-exclusive cargo features (`chip-esp32`, `chip-esp32s3`), enforced by `compile_error!` in `chip/mod.rs`. Adding a chip means dropping one file under `firmware/src/chip/` that implements a uniform interface (`NAME`, `TEST_LED_PIN`, `PinConfig`, `init_peripherals`, `register_all_ports`). `main.rs` and port crates are chip-agnostic. |
| Port crates | Chip-agnostic by design. Ports that need hardware (adc, pwm, board, display) take their hardware state as `Box<dyn FnMut(...)>` closures injected by the firmware at construction. The port crate never depends on `esp-hal` or any specific driver. |
| Pin assignments | Runtime-configurable. At boot, each chip module loads `pins.*` keys from a dedicated 4 KB flash sector and falls back to per-chip `DEFAULT_*` constants. Pin dispatch uses `AnyPin::steal(n)`; ADC uses a typed `match` because `AdcChannel` is only implemented for statically-known `GpioPin<N>`. Reconfiguring a pin is an MCP call to `board.configure_pin` followed by `board.reboot` — no reflash. |
| Shared I²C bus | The `i2c` port and the `display` port share I²C0 through `embedded_hal_bus::i2c::RefCellDevice`. The firmware wraps the `esp-hal I2c` instance in a `Box::leak`ed `&'static RefCell` and hands each consumer its own `RefCellDevice` handle. The `I2cPort` crate is generic over any `embedded_hal::i2c::I2c` implementor, so the same port works with either a raw bus or a shared-bus wrapper. |
| Distributed transport | smoltcp TCP listener on port 9100 after DHCP. mDNS responder via `edge-mdns` on `224.0.0.251:5353` announces `_soma._tcp.local.` so server SOMAs running `--discover-lan` find the leaf without static configuration. |
| Wire protocol | Identical to the server — length-prefixed JSON envelopes, same `TransportMessage` / `TransportResponse` enum. The leaf and server use the same `soma-esp32-leaf` crate for encode/decode. |
| Proven on hardware | Two chips (ESP32-S3 Sunton 1732S019, ESP32 LX6 WROOM-32D), both with and without wifi. Full discovery → configure → draw cycle verified end-to-end: `board.probe_i2c_buses` found an OLED at `0x3C`, `display.draw_text` rendered live thermistor readings on the physical panel under a brain-side 5-second loop. |

**What the leaf does NOT have:**

- No episodic memory pipeline. Schemas and routines are server-side concerns; the leaf only stores linear routines pushed via `TransferRoutine` and walks them on invocation.
- No skill selection. Every `InvokeSkill` names a specific primitive or stored routine.
- No policy engine. Safety enforcement happens on the server SOMA driving the leaf.
- No standard library. No filesystem, no threads, no dynamic linking, no heap growth beyond the fixed pool.

**Bridge from server to leaf.** A server SOMA reaches the leaf through the normal distributed transport layer. The MCP tool `invoke_remote_skill {peer_id, skill_id, input}` delegates to the leaf over TCP. `list_peers` returns mDNS-discovered leaves with IDs derived from the service instance name (`lan-soma-<chip>-<mac>`). An LLM driving the server SOMA via MCP can therefore invoke hardware skills on the leaf without any awareness that the body is running on a microcontroller — it's just another peer.

### 3. Multi-step autonomous routines (proven)

The autonomous path — episodes → schema induction → routine compilation → plan-following — is proven end-to-end against the real library in `soma-project-multistep`. Five phases cover: storing multi-step episodes, `PrefixSpan` inducing a 3-step `candidate_skill_ordering` with confidence `0.950`, `compile_from_schema` producing a 3-step `compiled_skill_path`, plan-following logic walking every step, and the real `SessionController.run_step()` walking a multi-step routine against `/tmp` with an injected `Binding { name: "path", value: "/tmp" }`. Trace: `stat → readdir → stat`. Final status: `Completed`.

What's proven: multi-step routines work when multi-step episodes exist. What's still open: producing those episodes organically from a single goal via the autonomous control loop — that requires the selector and critic to chain skills without explicit prompting, which is a separate question from the routine pipeline itself.
