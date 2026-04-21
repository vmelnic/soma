# Brain as Component

How SOMA absorbs the brain into the body's control loop, what this
changes, and what it enables.

## The problem with brain-as-driver

SOMA's MCP interface puts the brain in the driver's seat. The brain
must discover 45+ tools, choose the right one, construct JSON params,
interpret structured observations, decide what to do next, and repeat.
This is a full agent orchestration loop. Frontier models handle it.
Small models (gpt-4-mini, local 7B, quantized edge models) do not —
they pick wrong tools, construct malformed params, and lose coherence
across multi-step sessions.

The harness ecosystem (2026) tries to fix this by wrapping the model:
managing its context window, injecting memory, filtering tools,
validating outputs. But a harness around a weak model is still a weak
model with scaffolding. The orchestration burden remains on the brain.

SOMA already solves this problem on the body side. The 16-step control
loop in `SessionController` does everything a harness does — retrieves
memory, filters candidates, scores skills, binds inputs, executes,
evaluates results, patches belief. It just does it for the body's
autonomous path, not for the MCP-driven path.

The insight: instead of making the brain a better orchestrator, make
the brain a component inside the body's loop.

## The inversion

Current architecture (brain calls body):

```
LLM (brain) → MCP tool call → SOMA (body) → execute → return observation
               45+ tools
               complex JSON
               multi-turn reasoning
```

Proposed architecture (body calls brain via MCP):

```
SOMA control loop
  → Step 6: enumerate 3 candidate skills
  → Step 8: body can't resolve inputs → WaitingForInput
  → External brain reads state via inspect_session / inspect_belief_projection
  → External brain provides inputs via provide_session_input
  → Step 11: body executes with brain-provided bindings
  → Step 12: body evaluates
  → Step 13: body patches belief
  → loop
```

The brain never sees the full tool surface. It never constructs MCP
calls to 45+ tools. It never orchestrates. It answers narrow questions
exposed by the body: given this goal and this belief state, what value
should fill this missing input slot?

## The external brain principle

The brain is NOT embedded in soma-next. No LLM client code, no API
keys, no provider dependencies in the runtime. The body is
domain-agnostic — it doesn't know how to generate SQL, compose emails,
or reason about business logic. That's the brain's job.

The brain is any external process that can read SOMA's state and
provide inputs via MCP:

- Claude Code (the operator's IDE) calling MCP tools
- A Python script using the Anthropic SDK
- A local Llama model with an MCP client wrapper
- A human operator typing values
- Another SOMA instance acting as a higher-level brain

The body exposes what it needs. Any brain can provide it. This is the
hand/brain metaphor: a hand doesn't decide where to reach — it
provides proprioception so the brain can decide, regardless of what
kind of brain it is.

## The mechanism: WaitingForInput + provide_session_input

The control loop already pauses when it can't bind inputs. The missing
piece is an MCP tool that lets the external brain provide those inputs.

### Flow

1. Operator submits goal via `create_goal` (or `create_goal_async`)
2. Control loop selects a skill (heuristic predictor or routine)
3. `bind_inputs` fails — required slot "sql" has no value in belief or
   working memory
4. Session enters `WaitingForInput` with structured metadata:
   - Which skill was selected
   - Which slots are missing
   - Each slot's schema (type, description)
   - The goal objective
   - The projected belief (TOON-encoded)
5. External brain calls `inspect_session` → sees `WaitingForInput`
6. External brain calls `inspect_belief_projection` → gets minimal
   context (TOON-projected belief, orders of magnitude smaller than
   full JSON)
7. External brain decides and calls `provide_session_input`:
   ```json
   {
     "session_id": "...",
     "bindings": {
       "sql": "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)"
     }
   }
   ```
8. Session resumes with brain-provided bindings injected into working
   memory
9. Skill executes with real inputs → observation → belief update → loop
   continues

### What the external brain sees

The body pre-filters everything. The brain never sees 45+ tools or
raw session internals. Through the MCP tools it sees:

- **Goal**: one sentence ("create a sqlite table and insert data")
- **Projected belief**: TOON-encoded, top-5 facts above 0.3
  confidence, top-10 bindings, short keys
- **Missing slots**: structured metadata (name, type, description)
- **Available skills**: the pre-filtered candidate list

This is narrow enough for a 3B model, a gpt-4-mini call, or a human.

## Belief projection pipeline (proven)

The body compresses belief state before exposing it:

```
BeliefState (full Rust struct, 9 fields)
  → serde_json::to_value() (serialize once)
    → JMESPath expression (filter top facts, top bindings, drop metadata)
      → serde_json::Value (minimal, ~5 facts + ~10 bindings)
        → toon_format::encode_default() (tabular compression)
          → String (TOON-encoded)
```

**JMESPath expression** (compiled once at init):
```
{
  facts: facts[?confidence > `0.3`]
    | sort_by(@, &confidence)
    | reverse(@)
    | [:5]
    | [].{s: subject, p: predicate, v: value, c: confidence},
  bindings: active_bindings
    | sort_by(@, &confidence)
    | reverse(@)
    | [:10]
    | [].{n: name, v: value, s: source, c: confidence}
}
```

**Proven**: the projection achieves 90–99% size reduction depending on
belief complexity. Call `inspect_belief_projection` on a live session
for current numbers. TOON round-trips losslessly to JSON. The
projection strips: belief_id, session_id, resources, uncertainties,
provenance, world_hash, updated_at, and all facts below 0.3
confidence.

**Implementation**: `src/runtime/belief_projection.rs` (BeliefProjector),
`src/interfaces/mcp.rs` (`inspect_belief_projection` tool).

## Dead-end detection (proven)

The critic detects repeated identical failures and stops the loop:

- Extracts error messages from `raw_result`, `structured_result`, or
  `failure_detail` (fallback chain)
- After 3 identical consecutive errors (DEAD_END_THRESHOLD), returns
  `CriticDecision::Stop`
- The session controller honors the critic's Stop even during failure
  recovery — the critic's dead-end detection takes precedence over
  `handle_failure`'s retry/backtrack logic

**Before fix**: 15 identical failures burned the full step budget.
**After fix**: 3 failures, loop stops. Proven via MCP `create_goal`.

## Three plug points

### 1. Skill selection — via predictor or external brain

**When**: Every step. The heuristic predictor scores candidates by
keyword relevance and failure history. If the top score < 0.3 and
a `BrainFallback` is configured, the body defers.

**With external brain**: The session enters `WaitingForInput` or the
external brain preemptively provides a skill selection through
`provide_session_input`. The body validates that the brain's pick
is in the candidate list.

**Current code**: `adapters.rs` (`SimpleCandidatePredictor`),
`session.rs` (fallback invocation in `run_step`).

### 2. Input binding — via belief, working memory, or external brain

**When**: `SkillExecutor::bind_inputs` can't resolve a required slot
from belief or working memory. Currently a hard error that puts the
session in `WaitingForInput`.

**With external brain**: This is the primary interaction point. The
body exposes what's missing, the brain fills it.

**Fallback chain**:

```
working_memory binding → belief binding → WaitingForInput → external brain → hard error
```

The brain is the last resort before failure. Bindings from belief
and working memory are preferred because they have provenance and
don't cost a brain call.

### 3. Critic evaluation — via heuristic or external brain

**When**: The observation is ambiguous — not a clear success or
failure. The heuristic critic uses binary logic plus dead-end
detection.

**With external brain**: The external brain can call
`inspect_session` to see the latest observation, then influence the
next step by providing adjusted bindings or a different skill
selection. The body doesn't need a `BrainCritic` trait — the
external brain's influence comes through input provision, not
direct critic override.

**Override order**:

```
plan-following decision (always wins)
  → dead-end detection (budget protection)
    → heuristic critic (default)
```

## The brain trains itself out of a job

Every brain-provided input produces an observation. Observations
become episodes. Episodes compile into schemas. Schemas compile into
routines. Routines activate plan-following mode, which bypasses the
predictor and the binder entirely.

The lifecycle:

```
1. First encounter:
   Body pauses (WaitingForInput) → brain provides SQL →
   body executes → observation recorded → episode stored

2. Second encounter (similar goal):
   Episode memory returns nearest match → predictor scores higher
   → binder finds SQL pattern in prior observation output bindings
   → brain NOT called → execution succeeds

3. After N encounters:
   Schema induced → routine compiled → plan-following activates →
   brain is never called → execution is deterministic, fast, free
```

The expensive brain (frontier model during bootstrapping, or even a
small model during early operation) is a cost that amortizes to zero
as the body learns. This is the basal ganglia pattern: deliberate
decisions become automatic habits.

## Real-world analogies

**The cerebellum.** The cortex says "reach for the cup." The
cerebellum translates that into precise muscle activations. It doesn't
decide where to reach — it makes reaching accurate. When the
cerebellum can't resolve a motor plan (novel movement), it asks the
cortex a narrow question. As the movement is repeated, the cerebellum
builds a motor program and stops asking.

**A sous chef.** The head chef provides judgment: "sichuan pepper, not
chili flake." The sous chef handles timing, plating, prep, and every
other decision. As the dish becomes a regular menu item, the sous chef
knows the recipe and stops asking. The head chef is freed for new
dishes.

**Autopilot.** The autopilot flies the plane — altitude, heading,
speed, turbulence. The pilot intervenes at decision points: "weather
ahead, deviate left or right?" Once the route is known, the autopilot
handles everything. The pilot monitors.

In all three cases:

1. The body runs the loop
2. The brain answers narrow questions at decision points
3. Repeated decisions compile into automatic behavior
4. The brain's role shrinks as the body learns

## Operator experience

### Bootstrapping phase (brain active)

Operator assembles ports, writes pack manifests, and uses any LLM
as an external brain via MCP. Submits goals. The body runs
autonomously, pausing at decision points. The brain provides inputs
through MCP tools. Episodes accumulate.

```
# Terminal 1: SOMA runtime
soma --mcp --packs ./packs

# Terminal 2: External brain (any LLM)
# Reads: inspect_session, inspect_belief_projection, inspect_skills
# Writes: provide_session_input
# Or simply: Claude Code connected via .mcp.json
```

The operator doesn't write code. Doesn't hardcode LLM providers.
Any brain that speaks MCP can drive the bootstrapping. Claude Code
itself is a natural fit — it's already connected.

### Production phase (brain optional)

Routines have been compiled from episodes. The body handles known
workflows without pausing. The operator can downgrade to a cheaper
model, a local model, or no model at all.

For novel goals that don't match existing routines, the body still
pauses at `WaitingForInput`. A minimal brain (local 3B model, a
webhook, or even a human) provides the missing inputs.

### Edge deployment (brain minimal)

SOMA on constrained hardware. Ports talk to sensors, GPIO, databases.
Routines cover the operational domain. A tiny local model handles
rare novel decisions. Most execution is routine-driven, brain-free.

## What this is NOT

**Not the body making decisions.** The body runs the loop, the brain
provides judgment. The body never interprets goal semantics or
generates novel plans — it presents structured gaps and the brain
fills them. This preserves body ≠ brain.

**Not an embedded LLM client.** No API keys in soma-next. No
provider dependencies. No `--brain openai:gpt-4o-mini` flag. The
brain is external, communicating through MCP. Any LLM, any provider,
any language, any deployment model.

**Not a harness.** A harness wraps the model and manages its context
externally. Here the body owns the loop and exposes minimal questions
via MCP. The brain has no memory, no context management, no multi-
turn state inside SOMA. It reads state, provides input, and forgets.

**Not fine-tuning.** The brain is a general-purpose LLM answering
narrow questions. No training, no custom weights. The body does the
"fine-tuning" equivalent through episode → schema → routine
compilation.

**Not a replacement for the MCP interface.** The MCP surface remains
for operators who want direct control, for frontier models that can
orchestrate, and for authoring/observation. Brain-as-component is the
autonomous execution path — what happens inside `create_goal`.

## Implementation surface

All changes are in `soma-next/src/`. No port changes. No manifest
changes. No LLM client code.

### Done

| Change | File | Status |
|---|---|---|
| `BeliefProjector` (JMESPath + TOON) | `runtime/belief_projection.rs` | Proven |
| `inspect_belief_projection` MCP tool | `interfaces/mcp.rs` | Proven |
| Dead-end detection fix (critic + handle_failure) | `adapters.rs`, `runtime/session.rs` | Proven |
| `FailureDetail::message()` method | `types/observation.rs` | Done |
| `provide_session_input` MCP tool | `interfaces/mcp.rs` | Done |
| `PendingInputRequest` + `MissingSlot` metadata | `types/session.rs` | Done |
| `BindingSource::BrainProvided` | `types/session.rs` | Done |
| `inject_brain_input` + auto-resume | `runtime/session.rs`, `interfaces/mcp.rs` | Done |

### Optional (in-process optimization)

The traits `BrainFallback`, `BrainBinder`, `BrainCritic` remain
valid as an optimization for low-latency scenarios where MCP
round-trips are too slow. An in-process adapter could implement
them by calling a local model directly. But the primary path is
external-brain-via-MCP.

## Cost model

With brain-as-driver (current MCP path), every step requires a full
tool-use turn: the LLM sees the entire tool catalog, conversation
history, and system prompt. Token cost scales linearly with steps.

With brain-as-component via MCP, the external brain sees only:
- TOON-projected belief (a few hundred bytes)
- Missing slot metadata
- Goal objective

Each decision point sends orders of magnitude less context than a
full tool-use turn. Call `inspect_belief_projection` for exact sizes.

When routines cover the goal, the cost is zero — no brain calls.

## Open questions

**Brain call latency.** MCP round-trip adds latency compared to
in-process calls. For latency-sensitive goals, the in-process
`BrainFallback` trait (already wired) could use a local model. But
for most goals, MCP latency is acceptable — the body is already
doing port calls that take 10-1000ms each.

**Critic integration.** The external brain influences the loop
through input provision, not direct critic override. If the brain
wants to abort or redirect, it could provide a special binding or
call `abort_session`. Whether a formal `BrainCritic` trait adds
value beyond this needs empirical evidence.

## Resolved

**Async brain interaction.** Proven via `soma-project-brain`:
`create_goal_async` + `get_goal_status` polling +
`notifications/goal/trace_step` push notifications. The brain polls
or streams, provides input via `provide_session_input`, and the
session resumes on the next step.

**Multi-slot binding.** `provide_session_input` accepts multiple
bindings in a single call. All missing slots are exposed at once
in the `WaitingForInput` metadata. Proven via `soma-project-brain`.
