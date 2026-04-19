# Memory Fusion: Procedural × Semantic (Design Proposal)

What happens when SOMA's existing procedural pipeline (episodes → PrefixSpan → schemas → routines) combines with the proposed structure-driven semantic memory ([semantic-memory.md](semantic-memory.md)). This document captures the research thinking on combining the two memory systems and what falls out of the combination.

## Status

**Proposal — not yet implemented.** Builds on the existing procedural memory pipeline (which is implemented and tested) and the proposed semantic memory tier in [semantic-memory.md](semantic-memory.md) (which is also a proposal). None of the combined behaviors described here exist in the codebase today.

## The two axes

SOMA's procedural memory and the proposed semantic memory are not competing systems. They're orthogonal views of the same observation stream:

| Axis | Pipeline | Operates on | Produces |
|---|---|---|---|
| **Temporal / sequence** | episode → PrefixSpan → schema → routine | Order of skill invocations across episodes | Frequently repeated procedures |
| **Structural / entity** | observation → structure extraction → knowledge graph | Contents of `PortCallRecord.structured_result` (JSON shapes, identifiers, cross-references) | Entities, properties, relationships |

PrefixSpan operates on the temporal axis. Structure-driven extraction operates on the structural axis. They draw from the same `PortCallRecord` source but extract different signals. Combined, they give the runtime a 2D pattern space that's qualitatively more powerful than either pipeline alone.

## The biggest payoff: entity-parameterized routines

**Routines today** (sequence only):

```
Routine: ["query_users", "create_appointment", "send_email"]
Match: when goal pattern matches the routine's signature
```

**Fused routines** (sequence + structure):

```
Routine:
  steps: [query_users, create_appointment, send_email]
  entity_flow:
    step_1: produces User
    step_2: consumes User, produces Appointment
    step_3: consumes Appointment, User
  shape: User → Appointment → notification
  applies_to: goals that reference a User entity
```

This stops being a compiled motor sequence and becomes a **compiled procedure with typed inputs and outputs**.

In neuroscience terms, it's the difference between:

- **Motor memory**: "tap finger 1, tap finger 2, tap finger 3" — rigid, plays back identically every time
- **Skilled action**: "grasp THIS object, lift to THAT location, place it" — generalized, parameterized by what it operates on

The basal ganglia analogue gets richer. Routines stop being brittle replays. They generalize over the entities they operate on. Same routine, different entities.

## What falls out of the combination

### 1. PrefixSpan sees TYPED sequences, not raw skill IDs

Current PrefixSpan input:

```
[skill_a, skill_b, skill_c]
[skill_a, skill_b, skill_d]
[skill_a, skill_b, skill_c]
→ pattern: skill_a → skill_b → ?
```

Fused PrefixSpan input:

```
[query(User), create(Appointment←User), notify(User)]
[query(User), create(Order←User), notify(User)]
[query(User), create(Appointment←User), notify(User)]
→ pattern: query(User) → create(?←User) → notify(User)
```

The pattern is **generalized over entity types**. The induced schema becomes "given a User, query them, create some entity referencing them, notify the user." That's a higher-level abstraction than "skill_a then skill_b."

Schemas become about *what kinds of entities flow through what kinds of procedures*, not just *which skills tend to follow which*.

### 2. Causal chains separate from coincidental sequences

PrefixSpan alone finds: "skill_a often precedes skill_b."

Structure extraction adds: "skill_a's output contains entity X. skill_b's input references entity X."

Combined: **skill_a → skill_b is causally linked through entity X**. This is a stronger pattern than pure sequence — it has a mechanism.

The runtime can now distinguish:

- **Coincidental** sequences (A often before B but no shared data) — weak pattern, may not generalize
- **Causal** sequences (A produces entity, B consumes it) — strong pattern, definitely generalizes

Causal sequences become routines first. Coincidental ones get pruned during consolidation.

### 3. Three consolidation axes instead of one

Episodes can be clustered by:

| Axis | Mechanism | What it finds |
|---|---|---|
| **Sequence** | PrefixSpan over skill IDs | Frequently repeated procedures |
| **Embedding** | HashEmbedder cosine similarity | Semantically similar goals |
| **Entity** | Shared entity nodes in graph | Episodes that touched the same things |

The third axis is new. "All episodes that involved user Alice" or "all episodes that touched the appointments table" become first-class clusters. Schema induction can run per-entity-cluster, finding patterns specific to *kinds of entities*, not just kinds of goals.

### 4. Goals self-enrich from the graph

Today: `goal_utils.rs` extracts filesystem paths because `/tmp` is syntactically recognizable. That's the only enrichment.

Fused: when a goal arrives, the runtime checks the knowledge graph for entity references. "schedule meeting with Alice" → graph lookup → Alice is a known User, has properties X/Y/Z, has relationships to entities P/Q/R. The goal arrives at the selector already enriched with everything the runtime knows about Alice.

The brain (LLM) doesn't have to ask "who is Alice?" The body remembers. The brain says what to do, the body supplies the context.

This stays within the body/brain separation. The body isn't *interpreting* — it's *recognizing* (Alice is a known node) and *enriching* (here's what I've observed about her). The interpretation of what to do with that context still belongs to the brain.

### 5. Working memory persists across sessions

Current `WorkingMemory` is transient — `active_plan`, `plan_step`, clears at session end.

Fused: the entities involved in a session persist as nodes in the knowledge graph. Next session's `WorkingMemory` is hydrated with relevant entities from the graph. Like how you remember the people in your life across days — you don't re-introduce yourself to your colleague every morning.

The control loop's working memory becomes a *projection* of the knowledge graph for the current session — the active subset, not a separate transient store.

### 6. Predictor learns expected entity shapes

`SimpleCandidatePredictor` currently scores skills. With structural data available, it also learns the *expected output shape* of each skill:

```
predictor.expected_output("postgres.query") = {
  shape: { rows: [{ id: ?, name: ?, ... }], count: int },
  entity_type: cluster_id_42 (mostly Users),
  confidence: 0.95
}
```

When an observation comes back, the predictor can detect:

- Same shape, expected entities → high confidence
- Same shape, new entity type → cluster boundary expanding
- Different shape than expected → anomaly, lower confidence in this skill's stability

This gives SOMA **anomaly detection for free** — no new code path, just comparing observation structure to learned expectations.

### 7. The schema induction loop becomes 2D

Current pipeline:

```
episodes → cluster by embedding → PrefixSpan per cluster → schemas
```

Fused pipeline:

```
episodes → cluster by (embedding × entity-overlap) → PrefixSpan over typed sequences per cluster → entity-parameterized schemas → typed routines
```

Each cluster is now defined by *what entities it touches* AND *what goals it pursues*, not just one or the other. The induced schemas are higher-fidelity because they're conditioned on more dimensions.

## Concrete walkthrough

```
First time SOMA sees "schedule appointment with Alice":
  - LLM decides what to do
  - Runtime executes: query user → check availability → create appointment → notify
  - Episode recorded (procedural pipeline)
  - Structure extraction: User node (Alice), Appointment node, edge User→Appointment

After 5 similar episodes:
  - PrefixSpan finds the sequence pattern
  - Entity overlap finds: all episodes involve a User and produce an Appointment
  - Fused schema: "User → check_availability → create_appointment(User) → notify(User)"
  - Confidence high → routine compiled
  - Routine is parameterized: applies to any User, produces an Appointment

Next time "schedule appointment with Bob":
  - Goal arrives, references "Bob"
  - Graph lookup: Bob is a User (entity_type matches routine's typed parameter)
  - Routine matches, plan-following activates
  - Bypass deliberation, walk the compiled path with Bob substituted for the User parameter
  - LLM not needed
```

The autonomous path becomes truly autonomous. Not just "replay this exact sequence" but "apply this learned procedure to a new entity of the same type."

This is what generalization looks like. Procedural memory alone can compile motor sequences. Procedural + semantic together can compile *skilled action* — procedures that operate on the kinds of things SOMA has learned about.

## Architectural mapping

The fusion preserves SOMA's body/brain separation and integrates with existing components:

| Existing component | Today | After fusion |
|---|---|---|
| `memory/episodes.rs` | Stores episodes by sequence | Same — episode storage is unchanged |
| `memory/sequence_mining.rs` (PrefixSpan) | Mines skill ID sequences | Mines (skill, entity_type) sequences |
| `memory/schemas.rs` | Embedding clustering + PrefixSpan | Embedding × entity-overlap clustering + typed PrefixSpan |
| `memory/routines.rs` | Compiled skill paths | Compiled skill paths with entity flow |
| `memory/working.rs` | Transient session state | Hydrated from knowledge graph for active entities |
| `runtime/predictor.rs` | Skill scoring | Skill scoring + expected output shape |
| `runtime/selector.rs` | Skill selection from goal | Skill selection from goal + entity context |
| `runtime/session.rs` | 16-step control loop | Same loop, plan-following can match by entity type |
| `runtime/goal.rs` | Goal parsing | Goal parsing + graph lookup for referenced entities |
| `memory/knowledge.rs` (proposed) | — | New: structure-driven extraction, knowledge graph |

The runtime gains one new module (`knowledge.rs`) and several existing modules gain a structural awareness. None of the existing behavior is removed — the procedural pipeline still works exactly as it does today. The fused pipeline adds capabilities, doesn't replace any.

## Why this is consistent with SOMA's principles

**Body/brain separation preserved.** The runtime doesn't interpret meaning. It detects structure (recurring JSON shapes, cross-referenced IDs) and structure becomes the basis for typed clustering. The LLM (brain) still decides what to do — it just has richer context to decide from. Labels for entity types come from the brain over time, not from the body upfront.

**Domain-agnostic.** No port-specific code anywhere. The fusion mechanism works for any port that returns structured JSON. Postgres, REST APIs, filesystem, SMS, sensors — they all produce `PortCallRecord.structured_result`, and structure extraction works on all of them mechanically.

**Self-populating.** No new data sources, no new manifest fields, no developer effort. The fused pipeline draws from the existing observation stream. Pack manifests stay unchanged. The runtime learns more from the same observations it already records.

**Consistent with existing pattern extraction.** PrefixSpan is already doing pattern extraction without semantic input — it detects recurring sequences purely structurally. Adding entity extraction is the same principle applied to a different dimension. Same philosophy, same approach, complementary output.

## What this gives SOMA

A runtime that learns **typed procedures over typed entities** — automatically, from observation, without being told what the entities mean. The body sees structure (what shapes recur, what IDs cross-reference) and the body sees sequences (PrefixSpan), and combining them yields generalized procedural knowledge.

This is closer to how the basal ganglia and neocortex actually cooperate. The basal ganglia compiles procedures (routines). The neocortex stores entity knowledge (semantic memory). Neither alone is enough — procedures are *applied to* entities, and entities are *operated on by* procedures.

In SOMA terms: the autonomous path stops being "replay learned sequences" and becomes "apply learned procedures to new entities of known types." That's a qualitative leap in what the runtime can do without LLM involvement.

## Scope note: what's in and what's out

This section establishes the bar for the rest of the document. Memory fusion is treated under **enterprise-grade rigor** — correctness, scale, performance, concurrency, reliability, observability, backward compatibility, testability, and operability. These are real constraints and the analysis below reflects them.

**Out of scope**: PII handling, GDPR compliance, security exposure of stored inputs, audit logging for regulatory purposes, right-to-deletion guarantees. These are universal problems that every database, observability tool, and audit system handles the same way (or doesn't). They are not SOMA-specific blockers, and pretending they are is enterprise-cargo-culting. If SOMA ever ships in a regulated environment, these decisions get made then. Today they don't gate the architecture.

Everything else (correctness, scale, performance, concurrency, reliability, observability, backward compatibility, testability, operability) is in scope and treated rigorously.

## Input data: content-addressed dedup, not a new field

The fusion benefits depend on the runtime being able to see what flowed through it — including inputs, not just outputs. Today, `PortCallRecord.input_hash` stores only a SHA-256 of the input, not the input itself. This is intentional and the right default.

The wrong solution: add an `input` field to `PortCallRecord`. This breaks the SDK ABI, balloons episode storage proportional to observation count, changes the s2s wire format, and forces every dynamically loaded port to ship inputs whether or not the runtime needs them.

The right solution: a separate **content-addressed input store**, indexed by the existing `input_hash`.

```rust
// memory/input_store.rs (proposed)
pub trait InputStore: Send + Sync {
    fn put(&mut self, hash: &str, input: &serde_json::Value);
    fn get(&self, hash: &str) -> Option<&serde_json::Value>;
    fn evict_unused(&mut self, referenced_hashes: &HashSet<String>);
}
```

Properties:

- **Zero ABI change.** `PortCallRecord` is unchanged. The SDK contract is preserved. Existing dynamically loaded ports work as-is.
- **Deduplication.** Same input from a thousand observations is stored once. Real workloads have high redundancy: identical SQL templates, identical token validations, identical filesystem reads. Expected dedup ratio: 10-100x.
- **Optional.** The runtime functions without the input store. Structure extraction degrades gracefully — output→output cross-references still work, only input→output detection is lost.
- **Backward compatible.** Old persistence formats and old peers don't see the store. New ones populate and consume it.
- **Tunable per port.** Some ports produce large inputs (postgres SQL); others produce trivial inputs (timer). Per-port enable/disable is a config decision, not an ABI decision.

### InputStore consistency requirements (real engineering)

Adding "a separate store" sounds simple. Under enterprise rigor it is not. The store must satisfy:

| Requirement | Reason |
|---|---|
| **Atomic write coupling** | Episode and input must be visible together. If episode persists but input store doesn't, structural extraction sees a hash with no payload. Need a write barrier. |
| **Crash recovery** | Partial writes during crash → orphan inputs (hash referenced by no episode) or dangling references (episode referencing missing input). Need GC for orphans, fallback for danglers. |
| **Concurrent dedup** | Two sessions write the same hash simultaneously. Last-write-wins is fine for content addressing (same hash → same value), but `put()` must handle the race without corruption. |
| **Eviction races** | Structural extractor reading input X while eviction removes it. Need read locks or epoch-based reclamation. |
| **Persistence sync** | Per-write fsync is slow; periodic is lossy on crash. Same trade-off as the existing episode store, doubled. |
| **Size accounting** | Must track total bytes (not just unique-hash count) to enforce bounds. Per-port size policies. |

This is a small storage system with real consistency requirements, not a `HashMap` wrapped in an `Arc<Mutex>`.

### Storage cost reductions on top of dedup

Even with deduplication the store can grow. Tunable knobs:

1. **LRU eviction with TTL.** Keep inputs for the last N hours; evict older. Structure extraction runs incrementally on fresh observations, so inputs are visible while still in the store. Old episodes lose input lookup but retain the hash for identity checks.
2. **Compression.** JSON compresses 5-10x with zstd. Compress on disk persist.
3. **Per-port size cap.** `postgres` inputs can be large; `timer` inputs are tiny. Per-port maximum input size, drop or truncate beyond.
4. **Sampling.** Store inputs for X% of observations. Patterns still emerge over the sampled subset; storage drops by 1/X.

These are tunable per workload. Defaults are conservative.

## Determinism is non-negotiable

The existing memory pipeline is deterministic: same observations in the same order produce the same schemas, the same routines, the same compiled plans. The fusion must preserve this property. Two specific risks:

### Shape clustering must be content-addressed

If cluster IDs are assigned by observation order (`cluster_0`, `cluster_1`, ...), the same shape observed in different runs gets a different cluster ID. The same typed sequence then produces a different PrefixSpan output, which produces a different schema, which produces a different routine. Two SOMA peers processing the same observation stream in different orders would compile incompatible routines. Routine peer transfer would behave inconsistently.

The fix: **content-addressed cluster IDs** computed from a normalized representation of the shape itself (sorted field paths + value types). Same shape always produces the same cluster ID, regardless of observation order or run timing. This couples the cluster identity to the shape, not to the temporal order in which it was first seen.

This forces the shape similarity metric to be:

- **Normalized** — `{a, b}` and `{b, a}` produce the same fingerprint
- **Stable under additions** — adding observation N+1 must not change cluster IDs for observations 1..N
- **Reproducible** — same JSON inputs always produce the same cluster IDs across runs and across peers
- **Testable** — specific JSON pairs produce known cluster IDs in unit tests

The "set of field names" heuristic does not satisfy these properties uniformly (it ignores nesting and value types). The right answer is a structural fingerprint over `(field_path, value_type)` pairs, sorted, hashed.

### Fused PrefixSpan correctness depends on stable cluster IDs

Once cluster IDs are content-addressed, the typed PrefixSpan input becomes deterministic. The same typed sequences in the same order produce the same patterns. Schema induction is reproducible. Routine compilation is reproducible. Cross-peer routine transfer becomes meaningful: peer A and peer B looking at the same data produce identical typed routines.

If cluster IDs are not content-addressed, the entire fusion is non-reproducible and the resulting routines cannot be transferred between peers safely.

## Async extraction, eventual consistency, episode retention watermark

Structural extraction runs on every observation. The control loop has latency budgets. Inline (synchronous) extraction adds latency to every port call, which the budget cannot absorb. Async extraction is required.

Async extraction creates a gap: an observation is recorded, but the graph hasn't been updated yet. During this gap:

- **Plan-following may match a stale routine.** The graph state needed to invalidate the match isn't there yet.
- **Schema induction may run on incomplete graph state.** Patterns conditioned on entity types may miss observations that would have changed the cluster.
- **Eviction may discard observations before extraction processes them.** The current ring buffer evicts the oldest episode when full. If extraction hasn't caught up, that observation's structural information is lost forever.

The resolution is a **retention watermark**: episodes are not eligible for eviction until the structural extractor has processed them. The eviction policy becomes "evict the oldest episode whose structural processing is complete." The extractor reports its position; the eviction policy respects it.

This adds coupling between the procedural and semantic subsystems. They were independent in the original proposal; they aren't independent under enterprise rigor. The watermark is small but cross-cutting — it touches `memory/episodes.rs`, `memory/persistence.rs`, and the new structural extractor.

Latency targets the extractor must meet:

- **P50**: sub-millisecond per observation (small JSON, common case)
- **P99**: under 10ms (large or deeply nested JSON)
- **Backpressure**: if the extractor falls behind, the watermark stalls and episodes accumulate. The runtime must surface this as a metric and either drop low-priority observations or block new ones.

## Knowledge graph at scale

"Bounded working set" is an answer; it isn't a measurement. Real targets must be set:

| Dimension | Target | Implication |
|---|---|---|
| **Node count** | 100K — 1M | Beyond `HashMap<NodeId, KnowledgeNode>`. Needs a backing store with indexes. |
| **Avg node degree** | 10 — 100 | Spreading activation cost is O(edges × hops). At degree 100 with 3-hop activation, traversal touches a substantial fraction of the graph. |
| **Per-node memory** | 200 — 500 bytes | 1M nodes → 200-500 MB resident. Too large for ESP32; acceptable on mobile and server. |
| **Persistence size** | < 200 MB serialized | JSON is too verbose. Binary format (postcard, bincode, custom) required. |
| **Restart time** | < 5 seconds for 100K nodes | Lazy loading or memory-mapped persistence. Synchronous load at startup blocks initialization. |
| **Index requirements** | by entity type, by property value, by recency | Ad-hoc HashMap is insufficient. Indexes must be maintained on every write. |

The proposed `KnowledgeStore` trait sketch in [semantic-memory.md](semantic-memory.md) is a minimum viable interface. The real implementation needs a backing store (sled, rocksdb, or a custom format with indexes), not in-memory hash tables.

This is not "engineering work, just labor." It is a real storage engine choice with real consequences for restart time, query latency, persistence cost, and memory footprint.

## Concurrency model

Multiple sessions, multiple peers, and possibly multiple LLMs all read and write the knowledge graph concurrently. The current `Arc<Mutex<dyn Store>>` pattern (used for SchemaStore and RoutineStore) does not survive enterprise scrutiny:

- **Read/write separation.** Structural extraction is write-heavy. Routine matching is read-heavy. Single global lock causes contention. Per-node locks risk deadlock. Reader-writer locks help only if writes are rare.
- **Snapshot reads.** Routine matching needs a consistent view of the graph. If the matcher reads while extraction is mid-update, it sees partial state. Snapshots (MVCC or copy-on-write) are the right answer; both add memory overhead and persistence complexity.
- **Cross-peer ordering.** Peer A and peer B both extract from their local observations. Even with content-addressed cluster IDs they might have observed different subsets. When they exchange routines, whose graph state is canonical?
- **Conflict resolution.** Peer A asserts `Node(alice, type=user)`; peer B asserts `Node(alice, type=provider)` later. Which assertion wins? Last-write? Highest-confidence? Per-peer namespacing? Each policy has different consequences for cross-peer cooperation.

Honest answer: the knowledge graph needs MVCC or copy-on-write semantics for read consistency, plus an explicit conflict resolution policy for cross-peer assertions. Neither exists in the proposal yet. Both are real architecture decisions.

## Backward compatibility for Schema and Routine

Adding `entity_flow` to `Schema` and `Routine` is a wire protocol break:

- **Field versioning.** `serde(default)` lets new code read old data, but old code reading new data drops the field silently. Across peers, this means routine semantics differ — receiver sees a routine without entity flow even though sender uses entity flow. The receiver may execute the routine without entity matching, producing incorrect results.
- **Capability negotiation.** Peers must announce protocol version on connect, refuse incompatible routines, or downgrade gracefully (transfer the procedural part, drop the entity flow).
- **Migration.** Existing on-disk routines and schemas need to load with empty `entity_flow` and either be re-derived (if the originating episodes survive) or marked as legacy (no entity matching).
- **Wire test coverage.** Every serialization round-trip needs tests for old↔new compatibility. The s2s test suite grows substantially.

This is a multi-month engineering item, not a small wire change.

## Operability

Operators must be able to inspect, intervene, and recover. None of the following exists in the proposal yet, and each is a non-trivial MCP tool or REPL command:

- **Inspect cluster state** — show entity types, member counts, defining shapes
- **Reset the knowledge graph** — clustering went bad; start over (with or without preserving manual assertions)
- **Override clustering** — merge `cluster_42` and `cluster_47`; split a cluster; reassign a node
- **Tune extraction thresholds at runtime** — minimum observations per cluster, decay rate, activation threshold
- **Diff graphs across versions** — what entity types existed yesterday that don't today; what changed
- **Export graphs for offline analysis** — dump to a file format consumable by graph tools
- **Set per-port extraction policies** — disable extraction for specific ports, set per-port size caps
- **Watermark visibility** — current extraction lag, episodes pending processing, eviction backlog
- **Health metrics** — extraction throughput, queue depth, conflict rate, cluster stability score

Each of these is small individually. Collectively they are a significant addition to the MCP and CLI surfaces.

## Re-ranked blockers (enterprise-grade)

Earlier drafts of this document treated several items as "engineering work, just labor." Under enterprise rigor that label was wrong. The honest ranking:

| Rank | Blocker | Type | Effort |
|---|---|---|---|
| 1 | Multi-step routines unproven (fusion has nothing to fuse without them) | Sequencing | Foundation work |
| 2 | InputStore consistency, durability, crash recovery | Engineering (real systems) | Medium |
| 3 | Async extraction architecture + episode retention watermark | Architecture | Medium-large |
| 4 | Deterministic shape similarity with content-addressed cluster IDs | Design + small research | Medium |
| 5 | Knowledge graph at scale (backing store, indexes, persistence format) | Storage engineering | Large |
| 6 | Backward-compatible Schema/Routine wire format and migration | Engineering | Medium |
| 7 | Concurrency model (MVCC, conflict resolution, cross-peer reconciliation) | Architecture | Large |
| 8 | Determinism preservation in fused PrefixSpan (depends on #4) | Design | Small once #4 lands |
| 9 | Operability surface (inspect, reset, override, tune, export) | Engineering | Medium |
| 10 | Identifier and cross-reference detection heuristics | Design + small research | Small |

PII / GDPR / security / compliance are explicitly absent from this list. They are universal, not SOMA-specific. See the scope note above.

## Build sequencing under the new ranking

```
Multi-step routines proven (foundation)
  ↓
InputStore added (content-addressed dedup, optional, with consistency guarantees)
  ↓
Identifier detection + per-skill expected shape learning in predictor
  (uses input/output structure without needing entity types)
  ↓
Content-addressed shape fingerprinting (deterministic cluster IDs)
  ↓
Knowledge graph storage primitive with backing store + indexes
  ↓
Async extraction pipeline + episode retention watermark
  ↓
Stable entity type emergence (clustering with thresholds and stability checks)
  ↓
Fused PrefixSpan over typed sequences (deterministic output)
  ↓
Entity-parameterized routine compilation
  ↓
Entity-aware plan-following in the session control loop
  ↓
Backward-compatible Schema/Routine wire format and peer migration
  ↓
Operability tools (inspect, reset, override, tune, export)
  ↓
Concurrency model hardening (MVCC, cross-peer conflict resolution)
```

Each step is independently testable. Steps 1-4 add real capability without depending on entity types. Steps 5-8 are the heart of the fusion. Steps 9-12 are productionization.

## Honest summary

The fusion is buildable. The architecture is sound. But under enterprise-grade rigor, the build is bigger and harder than a casual reading suggests:

- **Two real architecture decisions** that didn't exist in the casual version: async extraction with retention watermark, and the concurrency/MVCC model
- **One real systems engineering item**: the InputStore is a small storage system with consistency requirements, not a `HashMap`
- **One real design item**: content-addressed cluster IDs are necessary for determinism and cross-peer correctness; the casual "set of field names" heuristic doesn't suffice
- **One real storage engine choice**: the knowledge graph at scale needs a backing store with indexes, not in-memory hash tables
- **A multi-month wire compatibility item**: adding `entity_flow` to schemas and routines breaks the s2s wire format and needs migration

None of these are blockers in the sense of "cannot be done." They are blockers in the sense of "must be designed and built before the fusion produces correct, scalable, reproducible results." Skipping them produces a system that works on a developer's laptop with a thousand observations and breaks the moment it sees real load, real concurrency, or a real peer with a different observation history.

The fusion is worth building. But it's a research project layered on top of a research project, with a small distributed storage system underneath, and it deserves to be planned that way.

## Relation to existing docs

- [architecture.md](architecture.md) — describes the current 6-layer runtime, 16-step control loop, and existing memory pipeline. The fusion proposal does not change the architecture; it extends specific components (predictor, selector, schemas, routines, working memory).
- [semantic-memory.md](semantic-memory.md) — proposes the structure-driven semantic memory tier in isolation. This document is the follow-up that combines it with the existing procedural pipeline.
- [vision.md](vision.md) — the runtime IS the program. Memory fusion is the mechanism that lets the runtime accumulate not just procedural skill, but generalized procedural knowledge applicable across novel entities of known types. The body becomes skilled, not just trained.
