# SOMA Next Pack Spec

## Purpose

This document defines the normative pack model for `soma-next`.

A pack is the unit of capability distribution, versioning, validation, isolation, and exposure.
Packs extend the runtime with skills, resources, schemas, routines, policies, and optional adapters.

The pack system is how `soma-next` remains domain-agnostic while still supporting arbitrary deployments such as web, mobile, desktop, IoT, robotics, messaging, commerce, automation, and peer SOMA collaboration.

## Scope

This specification covers:

- pack manifest structure
- pack contents
- dependency rules
- compatibility and versioning
- load-time validation
- namespace rules
- capability scoping
- policy contribution
- registration of skills, resources, schemas, and routines
- remote exposure rules
- observability metadata
- lifecycle
- failure handling

This specification does not define:

- application-specific business logic
- any single domain workflow
- any one transport implementation
- any one serialization format as a mandatory storage format

## Terminology

- `Pack`: a versioned, loadable bundle of runtime capabilities.
- `PackSpec`: the canonical manifest object describing a pack.
- `Skill`: a reusable behavior that operates on resources through ports.
- `Resource`: a typed entity known to the runtime.
- `Schema`: a reusable structural pattern for control.
- `Routine`: a compiled habitual skill path.
- `Port`: a low-level integration endpoint.
- `Namespace`: the qualified name prefix owned by a pack.
- `Capability`: any action, resource, policy, schema, routine, or adapter the pack contributes.
- `Exposure`: the explicit permission for a capability to be callable locally or remotely.
- `Runtime`: the `soma-next` binary and its built-in subsystems.
- `Host`: the runtime instance loading one or more packs.
- `Peer`: another SOMA runtime reachable over the distributed subsystem.

## Normative Language

The key words `MUST`, `MUST NOT`, `SHOULD`, `SHOULD NOT`, and `MAY` are normative.

## Pack Model

A pack is a self-contained unit that can be loaded, validated, activated, queried, isolated, and removed by the runtime.

A pack MUST be:

- versioned
- namespaced
- capability-scoped
- dependency-declared
- validation-complete before activation
- observable at runtime
- fail-safe under partial failure

A pack MUST NOT assume ownership of global runtime state outside its declared scope.

## PackSpec

`PackSpec` is the canonical manifest object.

An implementation MAY serialize `PackSpec` as TOML, JSON, or another deterministic format, but the logical schema defined here is normative.

### Required Fields

Every `PackSpec` MUST define:

- `id`: stable pack identifier
- `name`: human-readable pack name
- `version`: pack version
- `runtime_compatibility`: supported runtime version range
- `namespace`: primary namespace owned by the pack
- `capabilities`: declared capability groups
- `dependencies`: required packs and external capabilities
- `resources`: resource types exported by the pack
- `skills`: skills exported by the pack
- `schemas`: schemas exported by the pack
- `routines`: routines exported by the pack
- `policies`: policy contributions and constraints
- `exposure`: local and remote exposure rules
- `observability`: telemetry and health metadata

### Recommended Fields

An implementation SHOULD also define:

- `description`
- `authors`
- `license`
- `homepage`
- `repository`
- `targets`
- `build`
- `checksum`
- `signature`
- `entrypoints`
- `tags`
- `deprecation`

### Schema Shape

The manifest MUST support, at minimum, the following logical shape:

```text
PackSpec {
  id: string
  name: string
  version: semver
  runtime_compatibility: version_range
  namespace: namespace_id
  capabilities: CapabilityGroup[]
  dependencies: DependencySpec[]
  resources: ResourceSpec[]
  skills: SkillSpec[]
  schemas: SchemaSpec[]
  routines: RoutineSpec[]
  policies: PolicySpec[]
  exposure: ExposureSpec
  observability: ObservabilitySpec
}
```

## Pack Contents

A pack MAY contain:

- a manifest
- skill definitions
- resource definitions
- schema definitions
- routine definitions
- policy definitions
- port adapters
- tests
- fixtures
- metadata
- documentation
- examples
- telemetry descriptors

A pack SHOULD be organized so that the runtime can validate it without executing arbitrary pack logic.

### Minimum Required Artifact Set

A loadable pack MUST provide:

- `PackSpec`
- one or more skills, resources, schemas, or routines
- explicit namespace information
- dependency declarations
- exposure declarations
- observability declarations

## Namespace Rules

### Ownership

Each pack MUST own at least one namespace.

The namespace is the qualified prefix for all exported capabilities.

Example:

- `auth.create_session`
- `calendar.book_appointment`
- `iot.device.read_state`

### Uniqueness

Namespace identifiers MUST be unique within a host runtime.

The runtime MUST reject any load that introduces a namespace collision.

### Internal Names

A pack MAY define internal, private names.

Private names MUST NOT be exposed outside the pack unless explicitly exported by the manifest.

### Qualification

All externally addressable capabilities MUST be fully qualified by namespace.

The runtime MUST NOT allow ambiguous unqualified cross-pack resolution.

## Capability Scoping

Capability scope is the primary security boundary for packs.

### Scope Types

A capability MAY be scoped as one or more of:

- `local`
- `session`
- `tenant`
- `device`
- `peer`
- `public`

### Scope Rules

- A capability MUST only be callable within its declared scope.
- A capability MUST NOT gain broader scope at runtime without a manifest update and reload.
- A pack MUST declare the minimum required scope for each exported capability.
- The runtime MUST enforce scope before dispatch.

### Sensitive Capabilities

Sensitive capabilities include:

- destructive operations
- credential operations
- external network operations
- device actuation
- remote delegation

Sensitive capabilities MUST require explicit policy metadata and MAY require confirmation or additional trust checks.

## Skill Registration

Skills are the primary exported behaviors of a pack.

### SkillSpec Requirements

Each skill MUST define:

- `skill_id`
- `namespace`
- `kind`
- `inputs`
- `outputs`
- `preconditions`
- `expected_effects`
- `observables`
- `termination_conditions`
- `rollback_or_compensation`
- `cost_prior`
- `risk_class`
- `determinism`
- `remote_exposure`

### Skill Kinds

The runtime MUST support at least:

- `primitive`
- `composite`
- `routine`
- `delegated`

### Registration Rules

- Every skill MUST be registered under a fully qualified name.
- Every skill MUST validate against its declared input and output schema before activation.
- Every skill MUST be associated with at least one resource or port dependency.
- A composite skill MUST declare its subskills and evaluation boundaries.
- A routine skill MUST declare the conditions under which it is safe to shortcut execution.
- A delegated skill MUST declare remote execution constraints.

### Execution Semantics

A skill execution MUST produce:

- a success or failure result
- a structured observation
- a latency measurement
- a cost estimate or measured cost
- a trace record

A skill execution SHOULD also produce:

- a patch to belief state
- a confidence value
- a failure classification if unsuccessful

## Resource Registration

Resources are typed entities known to the runtime.

### ResourceSpec Requirements

Each resource MUST define:

- `resource_id`
- `namespace`
- `type_name`
- `schema`
- `identity_rules`
- `versioning_rules`
- `mutability`
- `relationships`
- `exposure`

### Resource Rules

- Resource types MUST be namespaced.
- Resource instances MUST be uniquely identifiable.
- Resource schema validation MUST occur before registration.
- Resource updates MUST be versioned or patchable.
- Resource visibility MUST respect exposure and policy.

## Schema Registration

Schemas are reusable structures for control and decomposition.

### SchemaSpec Requirements

Each schema MUST define:

- `schema_id`
- `namespace`
- `trigger_conditions`
- `resource_requirements`
- `subgoal_structure`
- `candidate_skill_ordering`
- `stop_conditions`
- `rollback_bias`
- `confidence`

### Schema Rules

- Schemas MUST be loadable without executing pack code.
- Schemas MUST be queryable by the selector and controller.
- Schemas MAY be induced from episodes or supplied by the pack author.
- A schema MUST be versioned independently of individual skill behavior.

## Routine Registration

Routines are compiled habitual paths derived from stable successful behavior.

### RoutineSpec Requirements

Each routine MUST define:

- `routine_id`
- `namespace`
- `match_conditions`
- `compiled_skill_path`
- `guard_conditions`
- `expected_cost`
- `expected_effect`
- `confidence`

### Routine Rules

- A routine MUST only be used when its guard conditions pass.
- A routine MUST be cheaper or simpler to select than full deliberation.
- A routine MUST be invalidated when its underlying dependencies change incompatibly.

## Port Registration

Packs MAY contribute port adapters when the runtime supports dynamic ports.

### PortSpec Requirements

Each port MUST define:

- `port_id`
- `namespace`
- `kind`
- `capabilities`
- `input_schema`
- `output_schema`
- `failure_modes`
- `side_effect_class`
- `latency_profile`
- `auth_requirements`
- `sandbox_requirements`

### Port Rules

- Ports MUST be typed.
- Ports MUST NOT perform unscoped actions.
- Ports MUST return structured observations.
- Ports MUST expose deterministic failure classes where possible.
- Ports MUST report latency and side-effect class.

## Dependency Rules

### Declaration

Every pack MUST declare its dependencies explicitly.

Dependencies MAY include:

- other packs
- external ports
- runtime feature flags
- version ranges
- capability prerequisites

### Validation

The runtime MUST reject a pack when:

- any required dependency is missing
- any dependency version is incompatible
- any required capability is unavailable
- any dependency violates policy

### Load Ordering

The runtime MUST resolve pack dependencies before activation.

If dependency order cannot be resolved, the load MUST fail.

### Optional Dependencies

Optional dependencies MAY be declared.

Optional dependencies MUST NOT be required for activation.

## Compatibility and Versioning

### Pack Version

A pack MUST use a stable semantic version.

### Runtime Compatibility

A pack MUST declare the runtime versions it supports.

### Compatibility Checks

The runtime MUST check:

- pack version compatibility with runtime
- dependency version compatibility
- namespace conflicts
- schema compatibility
- skill signature compatibility
- resource schema compatibility
- policy compatibility

### Upgrade Rules

- A backward-compatible pack update MAY be hot-loaded if validation passes.
- A breaking change MUST require a version bump and explicit compatibility confirmation.
- A removed capability MUST invalidate routines or schemas that depend on it.

### Downgrade Rules

A downgrade MUST be treated as a compatibility event and validated like a fresh load.

## Load Validation

Validation is mandatory before activation.

### Validation Stages

The runtime MUST validate, in order:

1. manifest integrity
2. namespace uniqueness
3. dependency availability
4. version compatibility
5. resource schemas
6. skill schemas
7. schema schemas
8. routine schemas
9. policy constraints
10. exposure rules
11. observability metadata

### Validation Outcomes

The runtime MUST produce one of:

- `accepted`
- `rejected`
- `quarantined`
- `degraded`

### Rejection Conditions

A pack MUST be rejected when:

- required metadata is missing
- schemas are invalid
- dependencies are unsatisfied
- namespace collisions exist
- policy requirements fail
- exposures violate policy
- integrity checks fail

## Isolation Model

Pack isolation is mandatory.

### Isolation Requirements

A pack MUST be isolated by:

- namespace
- capability scope
- policy
- resource visibility
- port access
- remote exposure
- failure containment

### Isolation Semantics

- A pack MUST NOT access undeclared capabilities.
- A pack MUST NOT access private resources of another pack.
- A pack MUST NOT invoke another pack without a declared dependency or explicit allowed exposure.
- A pack MUST NOT bypass policy through internal indirection.

### Failure Containment

Failure in one pack MUST NOT corrupt unrelated packs.

The runtime MUST support quarantine or disabling of a failing pack without taking down the host.

## Policy Contribution

Packs MAY contribute policy metadata.

### PolicySpec Requirements

Policy contribution MUST describe:

- allowed capabilities
- denied capabilities
- scope limits
- trust classification
- confirmation requirements
- destructive-action constraints
- remote exposure limits

### Policy Rules

- Pack policy MUST be merged with host policy, not replace it.
- Host policy MUST have precedence over pack policy.
- Pack policy MUST NOT widen privilege beyond the host policy.
- Any policy contradiction MUST resolve toward the more restrictive rule.

## Remote Exposure Rules

Packs MAY expose capabilities to remote peers if explicitly declared.

### Remote Exposure Requirements

Every remotely exposable capability MUST declare:

- remote scope
- peer trust requirements
- serialization requirements
- rate limits
- replay protection requirements
- observation streaming support
- delegation support

### Remote Exposure Constraints

- A capability MUST NOT be remotely exposed by default.
- A capability MUST NOT be peer-exposed without explicit manifest declaration.
- A capability exposed to peers MUST still obey host policy.
- A remote-exposed capability MUST be traceable and auditable.

### Remote Safety

Remote exposure SHOULD be denied by default for:

- destructive operations
- credentials
- secrets
- device actuation
- policy mutation

## Observability Metadata

Each pack MUST provide observability metadata.

### ObservabilitySpec Requirements

The observability block MUST include:

- health checks
- version metadata
- dependency status
- capability inventory
- expected latency classes
- expected failure modes
- trace categories
- counters or metric names
- pack load state

### Observability Rules

- The runtime MUST expose pack health at runtime.
- The runtime MUST attribute failures and latency to the pack and capability.
- The runtime SHOULD expose pack-level metrics separately from global runtime metrics.
- A pack MUST be introspectable without executing pack logic.

## Lifecycle

### States

A pack MUST support the following lifecycle states:

- `discovered`
- `validated`
- `staged`
- `active`
- `degraded`
- `quarantined`
- `suspended`
- `unloaded`
- `failed`

### Transitions

The runtime MUST support:

- discover
- validate
- stage
- activate
- suspend
- resume
- quarantine
- unload
- reload
- upgrade
- rollback

### Activation Rules

- A pack MUST NOT become active until validation completes successfully.
- A pack MUST NOT expose capabilities before activation.
- A pack SHOULD be able to be revalidated on reload.

## Failure Handling

### Failure Classes

The runtime MUST distinguish at least:

- manifest failure
- schema failure
- dependency failure
- namespace collision
- policy failure
- port failure
- skill execution failure
- remote peer failure
- integrity failure

### Required Host Responses

When a pack fails, the runtime MUST choose one or more of:

- reject
- quarantine
- suspend
- disable specific capabilities
- degrade to a narrower capability set
- retry if the failure is transient and safe
- record a trace and metric event

### Partial Failure

A pack MAY be partially usable after failure if and only if:

- the failing capability is isolated
- the remaining capabilities validate
- policy allows continued use
- the host marks the pack degraded

### Recovery

The runtime SHOULD support:

- revalidation after configuration changes
- replay of transient remote failures
- reload after pack replacement
- rollback after failed upgrade

## Contents of a Pack

A pack directory or artifact SHOULD contain:

- manifest
- resources
- skills
- schemas
- routines
- policies
- optional port adapters
- tests
- fixtures
- examples
- telemetry metadata

The runtime MAY require additional host-specific files, but those MUST be declared in the manifest.

## Non-Goals

This specification does not define:

- application code patterns
- a specific product domain
- a specific database schema
- a specific UI framework
- a specific transport implementation
- a specific serialization format as a mandatory storage choice
- a specific learning algorithm for optional predictor components
- a migration path from legacy SOMA

The pack system is a production capability substrate.
It is not an application template system.
