# Port Specification

## Status

Normative.

This document defines the production contract for `Port`s in `soma-next`.

## Purpose

Ports are the runtime's low-level interface to the external world.

A port is not a cognitive unit.
A port does not plan, infer, select goals, or decide policy.

A port must:

- expose a typed capability
- validate inputs
- execute an external operation
- return a typed result
- emit a structured observation
- classify failure precisely
- declare side effects, latency, and trust requirements

## Scope

This specification covers:

- port identity and metadata
- capability declarations
- invocation and output contracts
- observation reporting
- failure classification
- side-effect declaration
- latency and cost reporting
- authentication and sandbox requirements
- execution guarantees
- validation rules
- lifecycle state
- isolation rules
- policy integration
- remote exposure constraints
- tracing obligations

This specification does not define:

- skill orchestration
- goal selection
- belief reasoning
- schema induction
- episode retrieval
- distributed session routing

Those are defined elsewhere in `soma-next`.

## Terminology

- `Port`: a typed adapter to an external system or device.
- `PortSpec`: the declared schema and policy metadata for a port.
- `Capability`: a callable operation exposed by a port.
- `Invocation`: one request to execute a capability.
- `Observation`: the structured result and side-effect summary of an invocation.
- `Effect`: a state change caused by a port call.
- `Side effect`: any change outside the runtime's internal memory and trace store.
- `Sandbox`: a runtime boundary that constrains what the port may access.
- `Trust level`: the policy classification attached to a port or its pack.

## Normative Requirements

The keywords `MUST`, `MUST NOT`, `SHOULD`, and `MAY` in this document are to be interpreted as normative requirements.

## Port Model

Each port represents one integration boundary.

Examples of ports include:

- filesystem
- database
- HTTP
- queue
- renderer
- sensor
- actuator
- messaging
- device transport

A port may expose one or more capabilities.

A capability may be primitive or composite, but the port wrapper must always present it as a typed external operation.

## PortSpec

`PortSpec` is the required declaration for every port.

### Required Fields

`PortSpec` MUST include:

- `port_id`
- `name`
- `version`
- `kind`
- `description`
- `capabilities`
- `input_schema`
- `output_schema`
- `failure_modes`
- `side_effect_class`
- `latency_profile`
- `cost_profile`
- `auth_requirements`
- `sandbox_requirements`
- `trust_level`
- `namespace`
- `observable_fields`
- `validation_rules`

### PortSpec Schema

Conceptually:

```text
PortSpec {
  port_id: String
  name: String
  version: SemVer
  kind: PortKind
  description: String
  namespace: String
  trust_level: TrustLevel
  capabilities: [PortCapabilitySpec]
  input_schema: SchemaRef
  output_schema: SchemaRef
  failure_modes: [FailureMode]
  side_effect_class: SideEffectClass
  latency_profile: LatencyProfile
  cost_profile: CostProfile
  auth_requirements: AuthRequirements
  sandbox_requirements: SandboxRequirements
  observable_fields: [String]
  validation_rules: [ValidationRule]
}
```

## Capability Model

### PortCapabilitySpec

Each capability MUST declare:

- `capability_id`
- `name`
- `purpose`
- `input_schema`
- `output_schema`
- `effect_class`
- `rollback_support`
- `determinism_class`
- `idempotence_class`
- `risk_class`
- `latency_profile`

### Capability Rules

- A capability MUST have a stable name within the port namespace.
- A capability MUST declare its input and output shapes.
- A capability MUST declare whether it is reversible, compensable, or irreversible.
- A capability MUST declare whether it mutates external state.
- A capability MUST declare whether repeated identical invocations are safe.
- A capability MUST declare if it can be exposed remotely.

## Input Contracts

Every invocation MUST provide typed inputs.

Inputs MUST be validated before execution.

Validation MUST check:

- schema conformance
- required fields
- type compatibility
- bounds and ranges
- enum membership
- referential integrity where applicable
- permission scope
- policy constraints

Ports MUST reject invalid input before external execution begins.

Ports MUST NOT silently coerce unsafe or ambiguous values.

## Output Contracts

Every invocation MUST return a typed output.

The output MUST distinguish:

- raw external result
- normalized structured result
- effect summary
- success/failure state
- latency and cost data
- confidence or uncertainty when applicable

Outputs MUST be stable enough for trace and session replay.

## Observation Model

Every invocation MUST emit an `Observation`.

An observation MUST include:

- `observation_id`
- `port_id`
- `capability_id`
- `invocation_id`
- `success`
- `raw_result`
- `structured_result`
- `effect_patch`
- `side_effect_summary`
- `latency_ms`
- `resource_cost`
- `failure_class`
- `confidence`
- `timestamp`

### Observation Rules

- Observation emission MUST happen even on failure.
- Observation emission MUST happen even when policy denies execution.
- Observation content MAY be redacted by policy, but the trace must still contain a classified outcome.

## Failure Model

Failures MUST be classified into explicit categories.

Required failure classes:

- `validation_error`
- `authorization_denied`
- `sandbox_violation`
- `policy_denied`
- `timeout`
- `dependency_unavailable`
- `transport_error`
- `external_error`
- `partial_success`
- `rollback_failed`
- `unknown`

### Failure Rules

- A port MUST surface the most specific failure class available.
- A port MUST not collapse all failures into a generic error.
- A port MUST support partial success reporting when a subset of effects completed.
- A port MUST report whether recovery or retry is safe.

## Side Effects

Every capability MUST declare its side-effect class.

Required classes:

- `none`
- `read_only`
- `local_state_mutation`
- `external_state_mutation`
- `destructive`
- `irreversible`

### Side-Effect Rules

- A capability MUST NOT under-declare side effects.
- A capability MUST expose whether it can be compensated or rolled back.
- A capability with irreversible side effects MUST be treated as high-risk by policy.

## Latency and Cost Reporting

Each capability MUST report latency and cost metadata.

Required fields:

- `expected_latency_ms`
- `p95_latency_ms`
- `max_latency_ms`
- `cpu_cost_class`
- `memory_cost_class`
- `io_cost_class`
- `network_cost_class`
- `energy_cost_class`

### Reporting Rules

- The port runtime MUST record actual latency for every invocation.
- The port runtime SHOULD maintain rolling latency summaries.
- The port runtime MUST expose cost metadata to selector and critic components.

## Authentication Requirements

Each port MUST declare auth requirements.

Auth may include:

- bearer token
- API key
- mTLS
- signed capability token
- local process trust
- device attestation
- peer identity trust

### Auth Rules

- A port MUST fail closed when required auth is missing.
- A port MUST not infer permissions from caller intent.
- A port MUST bind auth to capability scope, not just to port identity.

## Sandbox Requirements

Each port MUST declare sandbox requirements.

Required sandbox dimensions:

- filesystem access
- network access
- device access
- process access
- memory limit
- CPU limit
- time limit
- syscall limit where applicable

### Sandbox Rules

- A port MUST execute only inside its declared sandbox boundary.
- A port MUST be denied if the runtime cannot satisfy its sandbox requirements.
- A port MUST report sandbox violations as explicit failures.

## Execution Guarantees

Ports MUST provide the following runtime guarantees:

- typed invocation validation before external execution
- structured observation on completion or failure
- stable capability identity
- policy-aware admission control
- bounded timeout handling
- explicit failure classification
- traceable side-effect reporting

Ports SHOULD provide:

- deterministic wrapper behavior around nondeterministic external systems
- idempotence hints
- rollback or compensation where supported

Ports MUST NOT guarantee:

- external system correctness
- external determinism
- external availability

Those depend on the external system, not the runtime.

## Validation Rules

Before a port becomes active, the runtime MUST validate:

- `PortSpec` completeness
- capability uniqueness
- schema references
- auth metadata
- sandbox metadata
- trust level
- namespace collision
- remote exposure eligibility
- observability metadata

If validation fails, the port MUST NOT be activated.

## Lifecycle

Every port MUST follow a lifecycle.

Required states:

- `declared`
- `loaded`
- `validated`
- `active`
- `degraded`
- `quarantined`
- `retired`

### Lifecycle Rules

- A port in `loaded` state is not yet callable.
- A port in `validated` state may still be blocked by policy.
- A port in `degraded` state may continue to serve allowed capabilities with reduced confidence or capability set.
- A port in `quarantined` state MUST not be callable except for explicit recovery or inspection operations.
- A port in `retired` state MUST be removed from normal dispatch.

## Isolation

Ports MUST be isolated by capability scope.

Isolation dimensions:

- namespace isolation
- resource isolation
- process isolation
- network isolation
- device isolation
- trust isolation

### Isolation Rules

- A port MUST not gain access outside its declared capability set.
- A port MUST not read another port's private internal state.
- A port MUST not mutate runtime state outside of approved observation and trace channels.
- A port MAY publish structured observations and declared effects only.

## Policy Integration

The policy runtime MUST be able to:

- allow or deny a capability
- narrow the usable capability set
- require confirmation for risky capabilities
- require higher trust for remote exposure
- enforce quotas and rate limits
- force quarantine on repeated failure or violation

### Policy Rules

- Policy decisions MUST be made before external execution.
- Policy decisions MUST be recorded in trace.
- Policy MUST be able to block a port even if the skill selector prefers it.
- Policy MAY override capability exposure on a per-session, per-pack, or per-peer basis.

## Remote Exposure Constraints

Ports are local by default.

A port MAY be exposed remotely only if:

- its `PortSpec` explicitly allows remote exposure
- its auth requirements are satisfiable remotely
- its sandbox requirements remain enforceable
- its side effects are declared
- policy allows the exposure
- trace and audit obligations are preserved

### Remote Exposure Rules

- A remote-exposed port MUST preserve the same capability contract.
- A remote-exposed port MUST preserve observation semantics.
- A remote-exposed port MUST preserve failure classification.
- A remote-exposed port MUST preserve traceability.
- A remote-exposed port MUST not leak hidden capabilities.

## Tracing Obligations

Every port invocation MUST be traceable.

Trace records MUST include:

- session id
- goal id where applicable
- port id
- capability id
- invocation id
- caller identity or peer identity
- input hash or redacted input summary
- auth result
- policy result
- sandbox result
- latency
- success/failure
- failure class
- effect summary

### Trace Rules

- Trace MUST record denied invocations.
- Trace MUST record partial success.
- Trace MUST preserve enough data to replay the decision path at the runtime level.
- Trace MAY redact sensitive payloads, but not the existence of the attempt.

## Pack Registration

Ports are registered through packs.

Pack-level port registration MUST:

- bind the port to a namespace
- validate the port spec
- validate required dependencies
- validate auth and sandbox metadata
- publish capability metadata to the skill runtime and selector

The runtime MUST be able to enumerate active ports and their capabilities.

## Compatibility Requirements

Ports MUST be versioned.

Version compatibility MUST be explicit.

If a pack or skill depends on a port capability version range that is unsatisfied, the port MUST be considered unavailable for that dependency path.

## Non-Goals

This specification does not define:

- goal formation
- belief inference
- skill selection
- schema induction
- episode retrieval
- distributed routing
- domain-specific business logic

Ports are not the system's intelligence.
Ports are the system's typed contact surface with the world.
