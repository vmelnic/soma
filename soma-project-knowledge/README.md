# soma-project-knowledge

Proves SOMA as a **knowledge metabolism** runtime — a system that learns HOW to retrieve, not just WHAT to retrieve.

RAG retrieves the same way every time. SOMA metabolizes retrieval strategies into compiled habits that get faster and cheaper through use.

## What it proves

A seven-phase proof against real pgvector and the soma-next runtime:

| Phase | What | Result |
|-------|------|--------|
| 1 | pgvector connection + 128-dim hash embeddings for 22 documents | All 4 capabilities verified |
| 2 | Deliberative retrieval (Tier 2) — 15 queries across 3 strategy types | 15 episodes captured |
| 3 | Schema induction via PrefixSpan | 3 schemas discovered (0.950 confidence) |
| 4 | Routine compilation via Bayesian Model Reduction | 3 routines compiled |
| 5 | Routine execution (Tier 1) — plan-following, no skill selection | 9 queries via compiled plans |
| 6 | Metabolic metrics — Tier 2 vs Tier 1 comparison | Economic model reported |
| 7 | Real SessionController — automatic routine activation | 3/3 sessions Completed |

## Three retrieval strategies learned

- **engineering_lookup**: vector_search → get_document (semantic similarity)
- **policy_lookup**: keyword_search → category_filter → get_document (full-text + narrowing)
- **hr_benefits_lookup**: category_filter → keyword_search → get_document (category-first)

Each strategy is discovered from episodes, compiled into a routine, and executed automatically when a matching goal arrives — no LLM in the loop.

## Prerequisites

- Docker (for pgvector)
- Rust (edition 2024)
- soma-next and soma-ports built

## Run

```bash
docker compose up -d
sleep 3
cargo run --release
docker compose down
```

## Architecture

```
KnowledgePort (implements soma_next::runtime::port::Port)
  ├── vector_search   — pgvector cosine similarity
  ├── keyword_search   — tsvector full-text search
  ├── category_filter  — WHERE category = $1
  └── get_document     — fetch by ID

HashEmbedder (128-dim FNV-1a)
  └── Same algorithm as soma-next's built-in embedder

Episode → Schema → Routine pipeline (soma-next)
  ├── DefaultEpisodeStore    — stores 15 episodes from real queries
  ├── DefaultSchemaStore     — PrefixSpan discovers 3 skill orderings
  └── DefaultRoutineStore    — BMR compiles 3 executable routines

SessionController (soma-next bootstrap)
  ├── SkillRegistryAdapter   — 4 knowledge skills registered via pack manifest
  ├── PortBackedSkillExecutor — maps skill → port capability invocation
  ├── RoutineMemoryAdapter   — matches goals to routines by fingerprint
  └── SimpleSessionCritic    — min_steps success condition for multi-step plans
```

## Key design decisions

- **Port trait from soma-next directly** (not SDK) because `SdkPortAdapter` is private to soma-next internals.
- **bootstrap() with temp manifest** rather than `bootstrap_from_specs()` because the latter sets `network_access: false` in its sandbox profile, blocking pgvector connections.
- **Empty observable_fields** on the port spec because `get_document` returns different fields than `vector_search` — the runtime's output contract validation is port-level, not per-capability.
- **min_steps success condition** on goals so the critic walks all plan steps before stopping.
