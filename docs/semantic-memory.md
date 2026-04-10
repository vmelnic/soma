# Semantic Memory (Design Proposal)

A "second brain" tier for SOMA — declarative knowledge storage alongside the existing procedural memory pipeline. This document captures research and design thinking, not implemented behavior.

## Status

**Proposal — not yet implemented.** This document exists to record the design rationale for a future memory tier. None of the structures, traits, or behaviors described here exist in the codebase today.

## Motivation

SOMA's current memory system (memory/episodes.rs, memory/schemas.rs, memory/routines.rs) covers **procedural memory** — what SOMA did, patterns of behavior, compiled action sequences. In neuroscience terms:

| SOMA component | Brain analogue | Memory type |
|---|---|---|
| Episodes | Hippocampus | Episodic |
| Schemas | Neocortex (pattern induction) | Procedural pattern |
| Routines | Basal ganglia | Procedural / motor |
| WorkingMemory | Prefrontal cortex | Working / short-term |

What's missing: **declarative/semantic memory** — facts about the world, entities, relationships, accumulated understanding. The neocortex stores this in distributed associative networks. SOMA has no equivalent.

Today, application data lives in PostgreSQL (or Redis, or S3) accessed via ports. That works for ground-truth storage but provides no associative structure, no spreading activation, no consolidation. The runtime executes against external stores but never *understands* what it's doing.

A semantic memory tier would let SOMA accumulate knowledge from observations the same way it already accumulates schemas from episodes — automatically, without explicit programming.

## Design

### Conceptual structure

An associative memory network — typed graph with weighted edges and activation dynamics. Closer to spreading-activation models from cognitive science (Collins & Loftus, ACT-R declarative memory) than to graph databases like Neo4j.

```
Nodes      → entities SOMA knows about (users, sessions, concepts, facts)
Edges      → typed associations (is_a, has, caused_by, related_to)
Weights    → strength of association, built from observation frequency
Activation → accessing one node activates connected nodes via spreading activation
Decay      → unused associations weaken over time
Confidence → how sure SOMA is about a node or edge
```

Mapping to neuroscience:
- Nodes = neural assemblies (concept representations)
- Edges = synaptic connections
- Weight strengthening = long-term potentiation (LTP)
- Decay = synaptic pruning
- Spreading activation = neural firing propagation
- Consolidation = systems consolidation from hippocampus to neocortex

### Type sketches

```rust
// memory/knowledge.rs (proposed)

pub struct KnowledgeNode {
    pub id: NodeId,
    pub entity_type: String,           // "user", "appointment", "concept" — domain-specific
    pub properties: serde_json::Value, // flexible JSON properties
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
    pub access_count: u64,
    pub confidence: f64,               // 0.0 - 1.0
    pub source: KnowledgeSource,       // which observation created this node
}

pub struct KnowledgeEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub relation: String,              // "is_a", "has", "booked", "prefers"
    pub weight: f64,                   // 0.0 - 1.0
    pub created_at: DateTime<Utc>,
    pub reinforced_count: u64,
}

pub enum KnowledgeSource {
    Observation { episode_id: Uuid, port_id: String, capability_id: String },
    Inferred { from_nodes: Vec<NodeId>, rule: String },
    Declared { caller: String },       // explicit fact assertion via MCP
}

pub trait KnowledgeStore: Send + Sync {
    fn upsert_node(&mut self, node: KnowledgeNode) -> Result<NodeId>;
    fn upsert_edge(&mut self, edge: KnowledgeEdge) -> Result<()>;
    fn get_node(&self, id: &NodeId) -> Option<&KnowledgeNode>;
    fn neighbors(&self, id: &NodeId, max_depth: usize) -> Vec<(NodeId, f64)>;
    fn activate(&mut self, id: &NodeId) -> Vec<NodeId>;  // spreading activation
    fn search_by_type(&self, entity_type: &str) -> Vec<&KnowledgeNode>;
    fn consolidate(&mut self) -> ConsolidationReport;
    fn decay(&mut self, threshold: f64) -> usize;
}
```

### Pipeline integration

Semantic memory parallels the existing procedural pipeline:

```
Procedural:  Observation → Episode → PrefixSpan → Schema → Routine
Semantic:    Observation → Fact extraction → Knowledge node/edge → Consolidation → Core knowledge
```

Both pipelines start from the same source: `PortCallRecord` observations produced by every port invocation. The procedural side already feeds these into episodes. The semantic side would feed the same observations into a fact extractor that produces knowledge nodes.

```
PortCallRecord (port=postgres, capability=query, structured_result={rows: [...]})
   ↓
Procedural path:                     Semantic path:
  Episode created                      Fact extraction (per pack manifest entity schemas)
  ↓                                    ↓
  PrefixSpan over episodes             Knowledge nodes upserted
  ↓                                    ↓
  Schema induced                       Edges created/reinforced
  ↓                                    ↓
  Routine compiled                     Consolidation cycle
```

### Spreading activation

Accessing a node activates connected nodes proportional to edge weight, with decay per hop:

```
activation(node, hop=0) = 1.0
activation(neighbor, hop=1) = weight(edge) * 0.5
activation(neighbor_of_neighbor, hop=2) = weight * 0.5 * 0.25
...stops when activation drops below threshold (e.g., 0.1)
```

This is how related knowledge surfaces without explicit search. Query "Alice" → activation spreads to her sessions, appointments, preferences, recent activity. The LLM gets back not just the node but the contextually-relevant subgraph.

### Consolidation

Mirrors the existing episode → schema cycle. Periodically (or after N observations):

1. **Reinforce frequently co-observed pairs** — edges that fire together get heavier weights.
2. **Prune weak edges** — edges below a decay threshold are removed.
3. **Promote core knowledge** — nodes with high access_count and reinforced_count get marked as "consolidated" — survive longer, harder to invalidate.
4. **Lower confidence on contradictions** — new observations that contradict existing nodes don't overwrite, they reduce confidence. The node persists with lowered certainty.

### Persistence

Same pattern as `memory/persistence.rs` for episodes:
- In-memory `KnowledgeStore` for active operations
- Disk-backed `DiskKnowledgeStore` for durability
- Checkpoint/restore for recovery
- Serializes to JSON (small graphs) or binary format (larger graphs)

### MCP exposure

Three new MCP tools (in addition to the existing 19):

| Tool | Purpose |
|---|---|
| `query_knowledge` | Query the knowledge graph by entity type, properties, or relationship |
| `assert_fact` | Explicitly declare a fact (caller-provided knowledge) |
| `dump_knowledge` | Return the full graph or a subgraph for LLM consumption |

`dump_state` would also include a knowledge summary — node count by type, top entities by access frequency, recent additions.

## Use cases

### 1. Cross-port understanding

PostgreSQL knows about users. Redis knows about sessions. Auth knows about tokens. They don't know about each other. Semantic memory links them:

```
Node(user:alice) ──has_session──► Node(session:abc)
                ──has_token────► Node(token:xyz)
                ──booked──────► Node(appointment:42)
```

One graph, cross-port understanding. No JOINs across systems — SOMA already knows the relationships from past observations.

### 2. Richer dump_state

Today `dump_state` returns runtime state (loaded ports, sessions, recent episodes). With semantic memory, it returns *what SOMA understands about the domain*:

```json
{
  "knowledge_summary": {
    "total_nodes": 247,
    "total_edges": 891,
    "by_type": { "user": 13, "appointment": 47, "review": 22, "service": 18 },
    "core_entities": [
      { "id": "user:alice", "access_count": 142, "confidence": 0.98 },
      { "id": "service:cleaning", "access_count": 89, "confidence": 0.95 }
    ],
    "recent_facts": [...]
  }
}
```

An LLM calling `dump_state` starts with understanding, not blank.

### 3. Routines + knowledge

A routine says: "when goal matches X, execute skill A then B then C."
Knowledge says: "user Alice prefers morning appointments, has 4 reviews, last active yesterday."

Combined: the routine executes the procedure, knowledge supplies the context. Procedural and semantic memory working together — like how the hand knows how to write while the brain knows what to write.

### 4. Cache layer above ports

First query for "all providers" hits Postgres. SOMA observes the result, extracts knowledge nodes for each provider. Second time the same kind of question comes up, SOMA's semantic memory already has it. No port call needed. The graph is a learned cache between the LLM and the external stores.

## Open problems

### Fact extraction

The runtime must extract knowledge from observations **without being told what the entities mean**. Pack manifests describe what a port *does* — capabilities, effects, costs. They must not describe what the data *means*. Meaning is the brain's job. Adding entity declarations to manifests would put domain knowledge in the runtime layer, violating the body/brain separation that SOMA's architecture depends on.

The runtime doesn't need to understand "user" or "appointment." It only needs to notice **structure** — and structure is observable without semantics.

**Structure-driven extraction** uses purely syntactic signals from the JSON in `PortCallRecord.structured_result`:

| Signal | What it produces | How it's detected |
|---|---|---|
| **Recurring shape** | Entity type | Same set of field names appears across observations → cluster as a type |
| **Identifier-shaped values** | Node candidates | UUIDs, integer ids, primary-key-like fields (`*_id`, `id`) → become node IDs |
| **Field co-occurrence** | Node properties | Fields that appear in the same record become properties of that node |
| **Cross-reference** | Edges | An ID value from observation A appearing as a field in observation B → edge between A's node and B's node |
| **Temporal ordering** | Causal hints | Observation O2 follows O1 in the same session → weak temporal edge |
| **Repetition frequency** | Edge weight | Same cross-reference observed N times → reinforced edge |
| **Input/output flow** | Provenance edges | Capability input field X used to produce output containing X → provenance link |

The runtime extracts the *graph topology* mechanically. It does not know that a node represents a user — it knows that this node has these properties, is referenced by these other nodes, and was produced by these capabilities.

**Where labels come from:** The LLM (brain). When the LLM interacts via MCP, it can label nodes via `assert_fact` or via `query_knowledge` results that the LLM annotates and re-asserts. Labels accumulate over time as the brain explains what the body has been observing. The body builds the structure; the brain supplies the meaning.

This mirrors how human semantic memory actually works. A baby learns object permanence, spatial relationships, and causal regularities before learning the words for things. The associative substrate is built from sensory pattern detection. Labels come later, from interaction with caregivers (the brain telling the body what things are called).

This approach is also consistent with SOMA's existing memory pipeline. PrefixSpan extracts schemas from episodes by detecting recurring sequences — pure pattern detection, no semantic input. The same principle applies to semantic memory: detect recurring entities, recurring relationships, recurring co-occurrences. The runtime sees structure, not meaning.

**LLM-assisted labeling (optional, not required).** During consolidation cycles, an LLM can be invoked occasionally to label clusters of structurally-similar nodes ("nodes with shape X are users"). This is a quality improvement, not a requirement. The graph works without labels — labels just make it more useful for the LLM that will eventually query it.

### Staleness and invalidation

The knowledge graph is a cache of reality. Reality changes — someone deletes a user from Postgres, the graph still has them. Cache invalidation is the hardest problem in computer science. Options:

- **TTL per node** — knowledge expires unless reinforced
- **Confidence decay** — confidence drops over time, new observations reset it
- **Inverse observations** — when a query returns *fewer* results than expected, lower confidence on missing entities
- **Explicit invalidation** — port effects (writes, deletes) explicitly mark related nodes stale

None of these are perfect. The honest answer: the graph is *useful but not authoritative*. For ground truth, query the port. For context and associations, use the graph.

### Scale

An associative graph with spreading activation is O(edges) per traversal. Fine for thousands of nodes. Bad for millions. Postgres handles millions trivially.

The knowledge graph is not a database. It's a memory of *what has been observed*. For an application with 13 users (HelperBook scale), the graph fits in memory and traversal is fast. For an application with 13 million users, only the *active* subset is in the graph — recently accessed entities, with the rest evicted via decay.

This is consistent with how human semantic memory works: you don't remember every person you've ever met, you remember the people you've recently interacted with and the ones you've reinforced over time.

### ACID and concurrency

The knowledge graph has no transactions, no isolation, no durability guarantees comparable to a real database. For application data that *matters* (orders, payments, audit logs), you still need Postgres.

The graph is for understanding, not for storing. Authoritative state lives in ports.

## What semantic memory replaces and what it doesn't

| Use case | Knowledge graph | PostgreSQL |
|---|---|---|
| "What does SOMA know about Alice?" | Yes — spreading activation surfaces all associations | Requires knowing which tables to query |
| "Insert 10,000 orders atomically" | No | Yes |
| "What's related to this appointment?" | Yes — graph traversal, weighted | Requires JOINs and schema knowledge |
| "Complex aggregation over 1M rows" | No | Yes |
| "What patterns has SOMA noticed?" | Yes — consolidation surfaces patterns | No — Postgres stores data, doesn't notice patterns |
| "Transactional business logic" | No | Yes |
| "Cross-port relationships (user → session → token)" | Yes | Requires explicit modeling |
| "Cold data, never accessed" | No (decays out) | Yes |

Semantic memory is a **new tier**, not a Postgres replacement. The architecture becomes:

```
Procedural memory:  Episodes → Schemas → Routines     (how to do things)
Semantic memory:    Observations → Facts → Knowledge   (what is known about the world)
External stores:    Postgres, Redis, S3                 (ground truth, scale, transactions)
```

The knowledge graph sits between the LLM (brain) and the external stores (world). It's the *memory of having used the stores* — a learned associative cache that builds itself from observations.

## Comparison with other approaches

**Mem0, Zep, MemGPT:** These store conversation memory or key-value facts. None have spreading activation, none integrate with a runtime's observation pipeline, none have consolidation cycles. Closest analogue but much simpler.

**Knowledge graphs (Neo4j, ArangoDB):** General-purpose graph databases. Powerful but require explicit schema and explicit insertion. SOMA's semantic memory builds itself from observations and uses cognitive-science-inspired dynamics (activation, decay, consolidation) that graph databases don't provide.

**Vector stores (Pinecone, Weaviate):** Embedding-based similarity. Good for "find similar text" but no relational structure, no typed edges, no symbolic reasoning. Complementary, not equivalent — vectors could be used as node embeddings within the knowledge graph for similarity-based recall.

**ACT-R / SOAR:** Cognitive architectures with declarative memory chunks and activation. Closest theoretical analogue. Academic, not deployed in agent systems. SOMA's design borrows the activation/decay/consolidation mechanics.

**Cyc, Wikidata:** Hand-curated knowledge bases. Manually built, not learned from observation. Different problem.

As of this writing, no production agent framework has an associative memory network with spreading activation, consolidation, and decay integrated into a runtime's observation pipeline.

## Relationship to brain analogy

SOMA's existing tagline is "the runtime IS the body." Procedural memory (episodes/schemas/routines) is body memory — how the hand knows to type, how the leg knows to walk. Semantic memory is mind memory — what you know about the world.

Adding semantic memory to SOMA doesn't break the brain/body separation. The brain (LLM) still decides what to do with the knowledge. SOMA's semantic memory is more like the *unconscious associative substrate* — the network of associations you draw on without thinking. The LLM uses it; SOMA stores and structures it.

In the existing analogy:

| Layer | Role | Component |
|---|---|---|
| Brain | Decides what to do | LLM (or autonomous control loop) |
| Mind's eye | What is known | Semantic memory (proposed) |
| Body | Executes | SOMA runtime + ports |
| Muscle memory | Compiled procedures | Routines |
| Episodic recall | What happened | Episodes |

## Implementation phases

**Phase 1: Storage primitive.** `KnowledgeStore` trait, in-memory implementation, basic node/edge CRUD, persistence to disk. No activation, no consolidation. Manual fact insertion via MCP `assert_fact`.

**Phase 2: Structure-driven extraction.** The session control loop, after recording an episode, runs the structure-driven extractor over `PortCallRecord.structured_result`. Identifier-shaped values become nodes, recurring shapes become entity type clusters, cross-references become edges. Pack manifests stay unchanged. No automatic consolidation yet.

**Phase 3: Activation and decay.** Spreading activation on `query_knowledge`. Decay cycle runs alongside episode consolidation. Confidence dynamics.

**Phase 4: Consolidation.** Frequently co-accessed nodes get reinforced edges. Weak edges prune. Core knowledge is marked and survives decay.

**Phase 5: LLM integration.** `dump_state` includes knowledge summary. Routines can reference knowledge nodes as preconditions ("execute this routine when user is a known provider"). The autonomous control loop consults knowledge during goal interpretation.

Each phase is independently useful. Phase 1 alone gives SOMA a queryable knowledge store. Phase 2 makes it self-populating. Phases 3-5 add the cognitive dynamics.

## Honest assessment

**Strengths:**
- Completes the brain metaphor (procedural + semantic + episodic + working)
- Architecturally consistent with existing memory pipeline (mirrors episode → schema → routine extraction)
- Structure-driven extraction preserves the body/brain separation — runtime sees topology, brain supplies meaning
- No production agent framework has this combination — genuine differentiator
- Reduces external store dependencies for known/cached facts
- Spreading activation is theoretically sound (cognitive science, decades of research from Collins & Loftus through ACT-R)
- Self-populating from existing observation pipeline — no new data sources needed
- Bounded working set is a feature, not a limitation: mirrors how human semantic memory works (recently accessed and reinforced entities persist; cold knowledge decays)

**Real trade-offs:**

| Concern | Reality |
|---|---|
| **Staleness** | The graph is associative, not authoritative. Stale knowledge lowers confidence; the LLM checks ground truth via ports when needed. The graph is *memory of having used the world*, not the world itself. |
| **Scale** | Working-set bounded — like human memory. A 13-user app keeps 13 users in the graph. A 13M-user app keeps the recently-active subset. Cold entities decay out. This is correct behavior, not a limitation. |
| **Runtime complexity** | Real, but consistent with what SOMA already builds. The semantic pipeline parallels the existing procedural pipeline; both extract patterns from the same observation stream. The complexity is incremental, not architectural. |
| **Label sparsity (early stage)** | Without LLM interaction, the graph has structure but no labels. Useful for spreading activation and provenance tracking; less useful for explanation. Labels accumulate as the LLM interacts with the graph over time. |

This is a natural extension of SOMA's existing architecture, not a pivot. The procedural pipeline (episodes → schemas → routines) already proves that pattern extraction from observations works. Semantic memory applies the same principle to a different dimension of what observations contain — structure rather than sequence. Same philosophy, same approach, complementary output.
