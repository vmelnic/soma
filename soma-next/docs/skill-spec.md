# Skill Spec

## 1. Purpose

This document defines the normative skill contract for `soma-next`.

A skill is the primary executable behavior unit in `soma-next`. Skills are not programs, opcodes, conventions, or scripts. Skills are typed, reusable action policies that operate over resources through ports under runtime policy control.

Implementations MUST treat this spec as authoritative for skill loading, execution, validation, tracing, and policy enforcement.

## 2. Scope

This spec covers:

- the `SkillSpec` schema
- skill taxonomy
- execution semantics
- input and output binding rules
- preconditions, effects, observables, termination, and rollback
- composite, routine, and delegated skills
- validation and loading rules
- trace obligations
- policy hooks
- failure handling

This spec does not define:

- ports
- packs
- distributed peer behavior
- runtime belief storage internals
- session controller internals

Those are specified in adjacent normative documents.

## 3. Terminology

- `Skill`: a reusable executable behavior with explicit semantics.
- `Primitive Skill`: a skill that directly performs one bounded action through one or more ports.
- `Composite Skill`: a skill that orchestrates other skills under explicit control rules.
- `Routine Skill`: a compiled high-confidence skill used for repeated cases.
- `Delegated Skill`: a skill executed on another SOMA instance or remote capability provider.
- `Port`: a low-level integration endpoint used by skills.
- `Resource`: a typed entity managed by the runtime.
- `Belief State`: the runtime's current model of resources, facts, bindings, and uncertainty.
- `Observation`: structured output from a skill or port execution.
- `Effect`: a state transition caused by skill execution.
- `Precondition`: a required condition that must hold before execution.
- `Rollback`: a compensating or reversing action when execution fails or is aborted.
- `Termination Condition`: a rule that ends a skill or composite execution.

Normative keywords `MUST`, `MUST NOT`, `SHOULD`, `MAY` are used with RFC 2119 meaning.

## 4. Skill Taxonomy

### 4.1 Primitive Skills

A primitive skill is the smallest executable semantic unit.

Primitive skills MUST:

- bind their inputs against current belief state and/or explicit goal context
- execute one bounded action or a tightly coupled bounded set of actions
- return a structured observation
- declare their side effects and rollback behavior

Primitive skills SHOULD be preferred when a task can be completed without orchestration.

### 4.2 Composite Skills

A composite skill is a skill whose execution consists of subskill invocation, conditional branching, retries, and termination checks.

Composite skills MUST:

- declare subskill dependencies
- define step ordering or branching rules
- define success conditions over intermediate and final observations
- preserve traceability of each substep

Composite skills MUST NOT hide unbounded internal behavior.

### 4.3 Routine Skills

A routine skill is a fast-path skill derived from repeated successful episodes or explicit pack authoring.

Routine skills MUST:

- have explicit match conditions
- be bounded and deterministic under the declared conditions
- fall back to non-routine execution when the match is weak

Routine skills MAY bypass deeper deliberation if the match confidence and policy allow it.

### 4.4 Delegated Skills

A delegated skill executes remotely, either on another SOMA instance or a remote capability provider that exposes a compatible skill interface.

Delegated skills MUST:

- declare remote trust requirements
- declare remote capability and protocol requirements
- preserve local trace continuity
- report remote observations in structured form

## 5. SkillSpec

`SkillSpec` is the canonical skill declaration object.

### 5.1 Required Fields

Every `SkillSpec` MUST define:

- `skill_id`
- `pack`
- `kind`
- `name`
- `description`
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
- `version`

### 5.2 Field Semantics

- `skill_id`: stable unique identifier within the pack namespace.
- `pack`: owning pack identifier.
- `kind`: one of `primitive`, `composite`, `routine`, `delegated`.
- `name`: human-readable stable skill name.
- `description`: concise semantic summary.
- `inputs`: typed input schema.
- `outputs`: typed output schema.
- `required_resources`: resource types or identities required for execution.
- `preconditions`: conditions that MUST hold before execution.
- `expected_effects`: state transitions expected when the skill succeeds.
- `observables`: output fields used to verify success and update belief.
- `termination_conditions`: rules that end execution successfully or unsuccessfully.
- `rollback_or_compensation`: compensating action or reversal behavior.
- `cost_prior`: expected cost profile, including latency and resource use.
- `risk_class`: risk and side-effect classification.
- `determinism`: deterministic, partially deterministic, stochastic, or delegated-variant.
- `version`: skill definition version.

### 5.3 Optional Fields

Implementations MAY include:

- `tags`
- `aliases`
- `capability_requirements`
- `subskills`
- `guard_conditions`
- `match_conditions`
- `confidence_threshold`
- `telemetry_fields`
- `policy_overrides`
- `locality`
- `remote_endpoint`

## 6. Execution Semantics

### 6.1 Execution Model

Skill execution is a closed-loop operation:

1. bind inputs
2. validate preconditions
3. authorize against policy
4. execute through one or more ports or subskills
5. collect observations
6. apply effect patch to belief state
7. evaluate termination conditions
8. return result or failure

Execution MUST be observable step-by-step in trace output.

### 6.2 Boundedness

Every skill MUST be bounded by at least one of:

- explicit step limit
- explicit time limit
- explicit budget limit
- explicit termination condition

Skills MUST NOT run indefinitely.

### 6.3 Determinism

Deterministic skills MUST produce identical outputs for identical inputs and identical belief context, excluding time-varying external dependencies explicitly declared in the spec.

Partially deterministic and stochastic skills MUST declare the source of nondeterminism.

### 6.4 Observation Flow

Each execution MUST emit one or more `Observation` records.

Each observation MUST include:

- raw result
- structured result
- success flag
- failure class if any
- latency
- cost information
- effect patch if any

## 7. Input and Output Binding

### 7.1 Input Binding

Skill inputs MUST be bound from one or more of:

- explicit goal fields
- belief state resources
- prior observations
- current working memory
- remote observations
- pack-defined defaults if allowed

Binding MUST be typed.

Binding MUST reject incompatible types.

Binding MUST preserve provenance for every bound value.

### 7.2 Output Binding

Skill outputs MUST be written to:

- observation records
- belief state patches
- downstream bindings for composite execution

Output binding MUST preserve source identity and confidence.

### 7.3 Unresolved Bindings

If a binding is unresolved:

- the skill MUST fail validation, or
- the runtime MUST defer execution until the binding is resolved, or
- the runtime MUST choose an alternate candidate skill

The runtime MUST NOT silently coerce unresolved bindings into invalid values.

## 8. Preconditions, Effects, Observables

### 8.1 Preconditions

Preconditions are mandatory execution gates.

Preconditions MAY include:

- resource existence
- resource version match
- permission scope
- trust level
- policy state
- dependent skill readiness
- external availability
- budget constraints

Any failed precondition MUST be reported with a structured failure class.

### 8.2 Effects

Effects describe how the skill changes the world or the belief state.

Effects MUST be declared as a structured patch or effect descriptor.

Effects MAY be:

- creation
- update
- deletion
- emission
- scheduling
- notification
- delegation
- synchronization

### 8.3 Observables

Observables are the fields that prove or disprove successful execution.

Every skill MUST declare the observables required to:

- confirm success
- detect partial success
- detect ambiguity
- update belief confidence

## 9. Termination and Rollback

### 9.1 Termination Conditions

Every skill MUST define explicit termination conditions.

Termination conditions MUST cover:

- success
- failure
- timeout
- budget exhaustion
- policy denial
- external unrecoverable error
- explicit abort

### 9.2 Rollback and Compensation

Skills that can produce side effects MUST declare rollback or compensation behavior.

Rollback MAY be:

- full reversal
- compensating action
- logical undo
- no-op if irreversible, provided the risk is declared

If a skill is reversible, the runtime SHOULD prefer reversible strategies when candidate selection is otherwise equivalent.

If a skill is irreversible, that must be declared in `risk_class` and in policy handling.

## 10. Composite Skills

Composite skills orchestrate subskills under explicit control semantics.

Composite skills MUST:

- declare the subskill graph or ordering
- declare branch conditions
- declare stop conditions for each branch
- propagate observation and effect data upward
- preserve trace visibility for every substep

Composite skills MUST NOT obscure side effects behind opaque internal logic.

Composite skills MAY:

- retry substeps
- switch subskills
- backtrack
- delegate substeps
- stop early when goal conditions are satisfied

## 11. Routine Skills

Routine skills are cached high-confidence behaviors.

Routine skills MUST have:

- a match condition over goal/belief/resource context
- a confidence threshold
- a bounded execution path
- a fallback path when the match is weak or stale

Routine skills MUST be invalidated when:

- resource schema changes
- preconditions no longer hold
- policy changes
- pack version compatibility breaks
- confidence drops below the declared threshold

Routine skills are a performance mechanism, not a semantic replacement for skills.

## 12. Delegated Skills

Delegated skills execute remotely.

Delegated skills MUST:

- identify the remote peer or provider
- declare the remote trust requirement
- declare the remote capability contract
- preserve local session trace continuity
- include remote latency and failure in observations

Delegated skills MUST NOT weaken local policy.

Delegated execution MUST be rejectable by policy even if the remote side advertises capability.

## 13. Validation Rules

### 13.1 Static Validation

A skill MUST be rejected at load time if:

- required fields are missing
- schema types are invalid
- declared outputs do not match executable outputs
- preconditions are malformed
- rollback or compensation is missing for a declared side-effect class that requires it
- risk class is absent for destructive behavior
- remote references are malformed
- version or compatibility metadata is invalid

### 13.2 Semantic Validation

A skill SHOULD be rejected or demoted if:

- its preconditions and effects are inconsistent
- its observables cannot confirm success
- its termination conditions are incomplete
- its cost prior is unrealistically undefined
- its resource requirements are underdeclared

### 13.3 Runtime Validation

The runtime MUST revalidate before each execution:

- required resources
- permission scope
- policy state
- current resource versions where relevant
- budget state
- remote trust state for delegated skills

## 14. Loading Rules

### 14.1 Pack Load

Skills are loaded through packs.

Pack loading MUST:

- validate the pack first
- validate dependency order
- validate skill namespacing
- validate resource and port bindings
- register routines only after their base skill or schema is valid

### 14.2 Namespace Rules

Skill identifiers MUST be namespaced by pack.

Within a runtime, skill names MUST be globally unambiguous.

### 14.3 Compatibility Rules

A skill MAY be loaded only if:

- the owning pack is compatible with the runtime version
- the required resources are available
- the required ports are loaded and authorized
- policy allows exposure of the skill

## 15. Trace Obligations

Every skill execution MUST emit trace records sufficient to reconstruct:

- why the skill was selected
- which inputs were bound
- which preconditions passed or failed
- which ports or subskills were used
- what observations were produced
- what effect patch was applied
- why the skill terminated
- whether rollback or compensation was invoked

Trace records SHOULD be stable enough for replay and audit.

Trace records MUST preserve provenance and confidence for derived outputs.

## 16. Policy Hooks

Skill execution MUST pass through policy hooks at the following points:

- before candidate selection
- before binding final inputs
- before execution begins
- before each side-effecting step
- before delegation
- before rollback
- before remote exposure

Policy hooks MUST be able to:

- allow
- deny
- require escalation
- require confirmation
- constrain budgets
- downgrade trust
- force delegation rejection

Policy decisions MUST be traceable.

## 17. Failure Model

### 17.1 Failure Classes

Skill execution failures MUST be classified at least as:

- validation failure
- precondition failure
- policy denial
- binding failure
- port failure
- remote failure
- timeout
- budget exhaustion
- partial success
- rollback failure
- unknown/unclassified

### 17.2 Failure Handling

On failure, the runtime MUST:

- record the failure in trace
- update belief state with the failure outcome
- decide whether to retry, switch candidate, backtrack, delegate, or stop

The runtime MUST NOT treat a failed skill as successful unless observables support that conclusion.

### 17.3 Partial Success

Partial success is allowed only if explicitly declared.

Partial success MUST specify:

- which effects occurred
- which effects did not occur
- whether compensation is possible
- whether downstream execution may continue

## 18. Non-Goals

This spec does not define:

- ports
- packs
- peer protocol framing
- belief storage internals
- selector algorithms
- predictor model architecture
- UI rendering
- training pipelines
- ONNX or any specific learned backend

This spec also does not preserve legacy SOMA constructs such as:

- `Program`
- `ProgramStep`
- opcode catalogs
- convention routing

## 19. Normative Summary

Implementations of `soma-next` MUST treat skills as the primary behavior unit.

Skills MUST be typed, bounded, policy-aware, traceable, and loadable from packs.

Skills MUST execute against belief state and ports under session control.

Skills MUST support composite orchestration, routine acceleration, and delegated remote execution.

Skills MUST NOT be reduced to opaque program generation.
