# Distributed Runtime

## Purpose

This specification defines the distributed SOMA-to-SOMA subsystem for `soma-next`.

The distributed runtime MUST support peer coordination for:

- remote goal submission
- remote skill execution
- remote resource publication and query
- observation streaming
- belief synchronization
- session delegation and migration
- schema and routine transfer
- offline queueing and replay
- cost- and trust-aware routing
- trace and audit preservation

This subsystem is first-class. It MUST NOT be treated as a side protocol or a convenience feature.

## Scope

This specification covers:

- peer identity and capability advertisement
- trust and policy boundaries
- remote goals, skills, and resources
- observation transport
- belief/resource sync
- delegation and session migration
- schema/routine transfer
- offline delivery and replay
- routing and cost awareness
- trace, audit, and failure handling

This specification does not define:

- local skill semantics
- local pack semantics
- UI behavior
- application domain logic

Those are defined elsewhere in the `soma-next` architecture.

## Terminology

- `Peer`: a SOMA instance participating in distributed coordination.
- `Local Peer`: the SOMA instance currently making the decision.
- `Remote Peer`: another SOMA instance reachable over a transport.
- `Goal`: a desired state and its constraints.
- `Skill`: an executable behavior exposed by a peer or pack.
- `Resource`: a typed entity or state object exposed by a peer.
- `Observation`: a structured result emitted after execution or sync.
- `Belief`: the local runtime's current world model.
- `Session`: a live closed-loop control instance.
- `Schema`: an abstract reusable structure induced from episodes.
- `Routine`: a high-confidence reusable shortcut.
- `Delegation`: handing off a goal, subgoal, skill, or session to another peer.
- `Migration`: transferring ownership of an active session to another peer.
- `Replay`: later delivery of queued distributed messages or observations.

Normative keywords in this document use RFC 2119 style meanings.

## Architectural Rule

The distributed runtime MUST be designed around `Goal`, `Skill`, `Resource`, `Observation`, `Belief`, `Session`, `Schema`, and `Routine`.

The distributed runtime MUST NOT depend on legacy `Program` or `ProgramStep` semantics.

The distributed runtime MAY transport low-level execution details for debugging, but those details MUST NOT be the primary semantic contract.

## Peer Model

### Required Peer Identity

Each peer MUST have a stable `peer_id`.

Each peer MUST expose:

- identity
- version
- trust class
- supported transports
- reachable endpoints
- current availability
- policy limits
- exposed packs
- exposed skills
- exposed resources
- local latency class

### Peer Availability

Each peer MUST advertise one of the following availability states:

- `available`
- `degraded`
- `busy`
- `offline`
- `untrusted`
- `restricted`

Availability MUST be included in routing decisions.

### Capability Advertisement

Each peer MUST publish a capability advertisement that can be queried before delegation or remote execution.

The advertisement MUST include:

- peer identity
- trust class
- supported transports
- packs loaded
- skills exposed
- resources exposed
- policy constraints
- cost/latency profile
- current availability
- replay support
- observation streaming support

Advertisements MUST be versioned.

Advertisements MUST be cacheable with expiration.

## Trust Model

### Trust Classes

Peers MUST be classified into one of the following classes or equivalent local policy classes:

- `built_in`
- `trusted`
- `verified`
- `restricted`
- `untrusted`

The exact names MAY vary internally, but the system MUST support at least the distinction between strongly trusted peers, restricted peers, and untrusted peers.

### Trust Requirements

Trust class MUST affect:

- which goals may be delegated
- which skills may be invoked
- which resources may be read or written
- whether session migration is allowed
- whether schema/routine transfer is allowed
- whether offline replay is accepted

### Authentication

Remote peers MUST be authenticated by a mechanism strong enough for the deployment.

The implementation MUST support signed identity or an equivalent strong verification mechanism.

The implementation MUST reject unauthenticated peers for privileged operations.

### Authorization

Distributed actions MUST be authorized per:

- peer identity
- peer trust class
- requested action
- target resource
- target skill
- session policy
- local budget and safety policy

Authorization MUST be checked before execution and before accepting delegated work.

## Remote Skill Model

### Skill Advertisement

Each peer MUST expose the skills it is willing to execute remotely.

Every remote skill advertisement MUST include:

- `skill_id`
- `name`
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

### Remote Goals

A peer MUST be able to submit a goal to another peer.

Remote goal submission MUST specify:

- goal description or structured goal
- constraints
- budgets
- trust class expectations
- whether the sender requests a result, a trace, or both

The receiving peer MUST either:

- accept and create a remote session
- reject with a structured reason
- request stricter policy or more budget

### Remote Skill Invocation

A peer MUST be able to invoke a single remote skill on another peer.

Remote invocation MUST:

- validate trust and policy
- validate skill availability
- validate input binding
- return a structured observation
- preserve audit traceability

Remote invocation MUST NOT assume the remote peer can execute arbitrary local behavior.

## Resource Model

### Resource Advertisement

Each peer MUST advertise resources it is willing to expose.

Each resource advertisement MUST include:

- `resource_type`
- `resource_id`
- `version`
- `visibility`
- `access_mode`
- `mutation_mode`
- `sync_mode`
- `provenance`
- `staleness_bounds`

### Remote Resource Query

A peer MUST be able to query a remote resource if policy allows it.

The query response MUST include:

- resource snapshot or delta
- version
- provenance
- timestamp
- freshness/staleness information

### Remote Resource Publication

A peer MAY publish resource updates as a stream or as snapshots.

The publication mode MUST be declared in the capability advertisement.

## Observation Streaming

### Required Semantics

Distributed execution MUST support streaming observations.

An observation stream MUST be ordered within a session or stream identifier.

Each observation MUST carry:

- session or request identifier
- step or event identifier
- source peer
- skill or resource reference
- raw result
- structured result
- effect patch or equivalent state delta
- success or failure status
- latency
- timestamp

### Replayable Streams

Observation streams SHOULD be replayable when the transport or session policy allows it.

If replay is supported, the peer MUST be able to request replay from a known sequence point.

### Partial Delivery

The runtime MUST handle partial observation delivery.

If a stream is interrupted, the receiver MUST be able to detect:

- missing observations
- duplicate observations
- out-of-order observations
- stale replay data

## Belief and Resource Synchronization

### Belief Sync

Peers MAY synchronize belief summaries when useful for delegation, migration, or cooperative control.

Belief sync MUST preserve provenance and versioning.

Belief sync MUST distinguish:

- asserted fact
- observed fact
- inferred fact
- stale fact
- remote fact

### Resource Sync

Peers MUST support resource synchronization at least at one of these levels:

- snapshot
- delta
- event stream

The selected sync mode MUST be declared by the peer.

### Conflict Handling

When resource or belief conflicts exist, the implementation MUST support conflict-aware merging.

The system MUST be able to mark state as:

- confirmed
- tentative
- conflicting
- stale
- unresolved

The distributed runtime MUST NOT silently overwrite conflicting state.

### Staleness Detection

Each remote belief/resource result MUST include a freshness indicator or equivalent version information.

The local runtime MUST refuse to treat stale data as authoritative when policy requires freshness.

## Delegation

### Delegation Units

The runtime MUST support delegation of:

- a single skill
- a subgoal
- a full session
- a resource read or write operation
- a schema or routine lookup task

### Delegation Rules

Delegation MUST preserve:

- trace continuity
- session identity
- policy boundaries
- budget accounting
- attribution of actions and observations

The local peer MUST know what it delegated, to whom, under what policy, and with what remaining budget.

### Subcontracting

A peer MAY subcontract a subtask to another peer while retaining session ownership.

Subcontracted work MUST return structured observations.

The local peer remains responsible for the overall session unless ownership is explicitly migrated.

## Session Migration

### Migration Semantics

Session migration transfers ownership of an active session from one peer to another.

Migration MUST include:

- session identifier
- goal
- working memory
- belief summary
- pending observations
- current budget
- trace cursor
- policy context

### Migration Rules

Migration MUST only occur when:

- the receiving peer is authorized
- the receiving peer can satisfy the session policy
- the receiving peer can continue observation and trace integrity

Migration MUST fail closed if any required session data cannot be transferred.

### Mirror Mode

A session MAY be mirrored to another peer for redundancy.

Mirroring MUST NOT transfer authority unless migration is explicitly accepted.

## Schema and Routine Transfer

### Schema Transfer

A peer MAY share schemas with another peer if policy allows it.

Schema transfer MUST include:

- schema identity
- version
- trigger conditions
- expected subgoal structure
- candidate skill ordering
- stop conditions
- confidence

### Routine Transfer

A peer MAY share routines with another peer if policy allows it.

Routine transfer MUST include:

- routine identity
- match conditions
- compiled skill path
- guard conditions
- expected cost
- expected effect
- confidence

### Transfer Constraints

Schema and routine transfer MUST respect:

- trust class
- pack policy
- exposure policy
- confidentiality policy
- replay protection

## Offline Queueing and Replay

### Queueing

If a peer is temporarily unreachable, the runtime MAY queue distributed requests and observations for later delivery.

Queued items MUST be ordered.

Queued items MUST carry:

- sequence number
- origin
- destination
- creation time
- expiry or TTL
- replay eligibility
- priority

### Replay

Replay MUST preserve order within the replayable stream.

Replay MUST reject expired or policy-invalid entries.

Replay MUST be idempotent or detect duplicates.

### Expiration

Queued items MUST expire according to policy.

Expired items MUST be dropped or marked invalid, not silently delivered.

## Routing and Cost Awareness

### Routing Inputs

Routing decisions MUST consider:

- trust class
- skill availability
- resource availability
- peer latency
- peer load
- transport quality
- budget remaining
- policy constraints
- session priority
- freshness requirements

### Routing Policy

The runtime MUST prefer the least costly peer that still satisfies policy and correctness requirements.

The runtime MUST avoid routing to a peer if:

- the peer is untrusted for the operation
- the peer cannot satisfy the required freshness
- the peer lacks the required skill or resource
- the peer would violate policy or budget constraints

### Cost Model

The peer advertisement MUST expose at least:

- latency class
- availability
- replay support
- observation streaming support
- estimated cost class or equivalent routing weight

The local runtime MAY maintain its own learned routing scores, but it MUST preserve policy overrides.

## Policy and Security Boundaries

### Required Boundaries

Distributed actions MUST respect:

- peer trust class
- pack capability scope
- resource visibility
- skill visibility
- session policy
- local risk budget
- replay protection
- audit logging

### Confidentiality

Peers MUST NOT receive resources, schemas, routines, or observations outside their authorized scope.

### Destructive Operations

Delegated or remote destructive operations MUST require explicit policy authorization.

The runtime SHOULD require confirmation for destructive cross-peer actions when the deployment policy demands it.

### Isolation

Remote peers MUST be treated as untrusted by default unless policy elevates trust.

Any failure in trust validation MUST fail closed.

## Trace and Audit

### Trace Obligations

Every distributed action MUST be traceable.

The trace MUST record:

- origin peer
- destination peer
- action type
- session identifier
- goal identifier
- request identifier
- routing decision
- policy decision
- result
- failure reason if any
- timestamps

### Audit Obligations

The runtime MUST retain an audit trail for:

- remote goal submissions
- remote skill invocations
- resource queries
- observation streams
- delegation
- migration
- schema/routine transfer
- offline replay

Audit records MUST preserve attribution across peers.

### Correlation

Distributed traces MUST support correlation across peers through a stable request or session correlation key.

## Failure Modes

The distributed runtime MUST distinguish at least the following failures:

- peer unreachable
- transport failure
- authentication failure
- authorization failure
- trust validation failure
- unsupported skill
- unsupported resource
- stale data
- conflicting data
- replay rejection
- budget exhaustion
- timeout
- partial observation stream
- migration failure
- delegation refusal
- policy violation

### Failure Semantics

Failures MUST be structured, not opaque.

Failures MUST indicate whether they are:

- retryable
- delegatable to another peer
- terminal for the session
- terminal for the current action only

### Recovery

The runtime SHOULD attempt recovery by:

- retrying on another eligible peer
- switching to local execution
- resuming from replay
- continuing from the last confirmed observation
- aborting the session if policy requires it

Recovery MUST respect budget and policy limits.

## Compatibility Rules

Distributed peers MAY have different runtime versions.

Compatibility MUST be decided explicitly through advertised capabilities and version negotiation.

A peer MUST reject remote operations it cannot safely interpret.

## Implementation Requirements

Implementations of this spec MUST provide:

- peer identity
- capability advertisement
- trust-based authorization
- remote goals
- remote skills
- remote resources
- observation streaming
- belief/resource sync
- delegation
- session migration
- schema/routine transfer
- offline queueing and replay
- routing with cost/trust awareness
- trace and audit
- structured failure reporting

## Non-Goals

This specification does not define:

- local skill semantics
- local resource schemas
- UI rendering behavior
- model training
- ONNX inference
- legacy program execution
- domain application logic

Distributed SOMA-to-SOMA is infrastructure for cooperative control, not an application framework.
