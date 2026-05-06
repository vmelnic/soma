# Compiled Routines as Products

Ship applications as pre-compiled soma routines instead of hand-written
code. The frontier LLM acts as the developer during compilation. The
shipped product runs deterministically on routines, with LLM fallback
server-side for novel situations that compile into new routines over time.

## The thesis

Traditional software: a developer writes controllers, services, database
queries, error handling. Ships as source code compiled to a binary.

SOMA software: a frontier LLM executes scenarios against soma-next until
successful patterns compile into routines via BMR gating. Ships as a
soma-project with pre-compiled routines + port adapters.

The routine IS the controller. Ports ARE the service layer. Input
bindings ARE parameter extraction. The manifest IS the API spec.

```
Traditional:  developer writes code → compile → binary → deploy
SOMA:         frontier LLM executes → episodes → schemas → routines → deploy
```

Both produce deterministic execution. The difference: SOMA's shipped
product can keep learning in production.

## Architecture

```
clients (web / mobile / bot / API)
  → REST API (nginx + TLS)
    → soma-next
      → compiled routines (handle known paths — deterministic, no LLM)
      → ports (postgres, redis, auth, email, payments...)
      → frontier LLM (novel paths only → episode → compiles into routine)
```

From the client's perspective: a normal API. No LLM awareness. Clients
call `execute_routine("book_appointment", {provider_id, time})` the same
way they'd call a REST endpoint.

From the server's perspective: 95% compiled routines, 5% LLM fallback
that shrinks as traffic teaches the system new patterns. The LLM is a
backend operational cost that asymptotically approaches zero.

## Example: a HelperBook endpoint

### Traditional (Express.js)

```js
app.post('/appointments', auth, async (req, res) => {
  const user = await db.find('users', req.userId);
  const provider = await db.find('users', req.body.provider_id);
  const conflicts = await db.query(
    'SELECT * FROM appointments WHERE provider_id = $1 AND time_range && $2',
    [provider.id, req.body.time_range]
  );
  if (conflicts.length > 0) return res.status(409).json({error: 'conflict'});
  const appt = await db.insert('appointments', {...});
  await db.insert('notifications', {user_id: provider.id, ...});
  await redis.publish('notifications', JSON.stringify({...}));
  res.status(201).json(appt);
});
```

### SOMA routine (compiled data)

```
routine: book_appointment
  step 1: auth:session_validate → extract user_id
  step 2: postgres:find(users, {id: $provider_id})
  step 3: postgres:find_many(appointments, {provider_id, time_range})
  step 4: [branch on observation]
    conflict_found → respond(409, "conflict")
    no_conflict:
      step 5: postgres:insert(appointments, {...})
      step 6: postgres:insert(notifications, {...})
      step 7: redis:publish(notify_channel, {...})
      step 8: respond(201, $appointment)
```

Same logic. One is source code; the other is compiled behavior. Both
deterministic. The routine version is portable across clients, inspectable
as data, and can evolve without redeployment.

## What compiled routines give you over code

### Genuine advantages

**Self-evolution.** A novel request hits no compiled routine. The server-side
LLM handles it, the episode records the successful path, BMR compiles it
into a new routine. Next time: no LLM needed. The app learns new
capabilities without a code change or app store update.

**Self-healing.** Email notification fails. LLM fallback tries SMS, which
works. That episode compiles a fallback branch into the notification
routine. Next failure routes to SMS automatically, no developer involved.

**Portable logic.** Web, mobile, Telegram bot, REST API all call
`execute_routine("book_appointment", {...})`. Business logic lives in
routines, not in any client's source code. Add a new client: zero logic
duplication.

**Inspectable as data.** "Which routines touch the appointments table?" is
a structured query. "What breaks if the redis port goes down?" — trace
routine dependencies. Impact analysis without AST parsing.

**Behavior updates without binary updates.** Routine sync pushes new
compiled behaviors to clients without app store review or forced updates.
The binary stays the same; capabilities evolve.

### Not novel (honest)

Portability is what any backend API gives you. Inspectability is what
OpenAPI gives you. These are nice but not differentiating.

The differentiator is self-evolution + self-healing: a backend that
programs itself from usage. Code doesn't do that.

## Offline-first with routine sync

Ship soma-next + SQLite port inside a mobile app. Pre-compiled routines
run locally with zero network. When connectivity returns, sync both data
and routines:

```
┌───────────────────────┐         ┌───────────────────────┐
│  Mobile App            │         │  Backend               │
│  soma-next + SQLite    │  sync   │  soma-next + Postgres  │
│  compiled routines     │ ◄─────► │  compiled routines     │
│  local episodes        │         │  + frontier LLM        │
│  offline-first         │         │  + new routines        │
└───────────────────────┘         └───────────────────────┘
```

**Data sync:** local SQLite ↔ remote Postgres (CRDTs or last-write-wins).

**Routine sync:** backend compiles new routines from LLM encounters →
pushes to app on next connect. The app gains new capabilities without
an app store update.

## Partial updates across clients

Routines are server-side. Update the appointment routine to add waitlist
logic — all clients get it immediately. For client-specific behavior,
routines branch on `{client: "app"}` vs `{client: "bot"}`. No client
code changes needed.

## Development workflow

### Phase 1: Compile

Use a frontier LLM + soma-next to execute every scenario the app needs.
Successful patterns compile into routines via BMR (confidence >= 0.7).
This is the "development" phase — the LLM is the developer.

### Phase 2: Ship

Package the soma-project: pre-compiled routines + port adapters + manifest.
The `data/` directory contains all routines (like soma-project-kitchen).
Ship to production.

### Phase 3: Evolve

Production traffic hits compiled routines (fast, deterministic, free).
Novel requests fall through to LLM (expensive, stochastic, but handles
anything). Successful novel paths compile into new routines automatically.
LLM cost decreases over time.

## Cost model

| Phase | LLM cost | Behavior |
|---|---|---|
| Day 1 (fresh deploy) | Moderate — pre-compiled routines cover designed scenarios, LLM handles edge cases | Mostly deterministic |
| Month 1 | Low — most edge cases have compiled into routines | Almost fully deterministic |
| Month 6 | Near zero — LLM fires only for genuinely unprecedented situations | Effectively static but can still learn |

Compare to traditional: developer salary is constant regardless of traffic
patterns. SOMA's "developer" (the LLM) works less as the system matures.

## Routine DSL

The routine pseudo-code above is not just documentation — it's a DSL
that parses into soma-next's existing `CompiledRoutine` data structures.

```
routine: book_appointment
  input: $provider_id, $time_range, $token
  
  step auth:session_validate {token: $token} → $user_id
  step postgres:find users {id: $provider_id} → $provider
  step postgres:find_many appointments {provider_id: $provider_id, time_range: $time_range} → $conflicts
  
  branch $conflicts.count
    > 0:
      respond 409 {error: "conflict"}
    0:
      step postgres:insert appointments {user_id: $user_id, provider_id: $provider_id, time: $time_range} → $appointment
      step postgres:insert notifications {user_id: $provider_id, type: "new_appointment", ref: $appointment.id}
      step redis:publish notify_channel {event: "appointment_created", appointment: $appointment}
      respond 201 $appointment
```

### What makes it different from workflow DSLs

Existing workflow languages (Step Functions, Temporal, BPMN, GitHub
Actions) define step sequences with explicit conditionals. This DSL
differs in three ways:

**Port-native vocabulary.** Every instruction is `port:capability`. The
language is constrained to what the body's ports expose. You can't write
arbitrary code — only orchestrate existing capabilities. This is a
feature: the DSL is exactly as powerful as the ports, no more.

**Observation-driven branching.** `branch` inspects the observation
returned by the previous step. The runtime matches `DataCondition`
patterns against `observation.structured_result` — first match wins.
Branches can also use `on_success`/`on_failure` for binary outcomes.
Supports `Goto` (loops with iteration limits), `CallRoutine` (push
sub-routine onto plan stack), `Complete`, and `Abandon` (fall back
to LLM deliberation). Deterministic pattern matching, not probabilistic.

**Dual authoring.** The same DSL artifact is produced two ways:
1. Human (or LLM) writes it directly given port manifests as context
2. The compilation loop emits it from episodes → schemas → BMR gating

Path 1 collapses the "execute 100 scenarios to compile" workflow into
"generate routine from spec." Path 2 handles patterns that emerge from
production traffic. Both produce the same `CompiledRoutine` struct.

**$-bindings are declarative data flow.** `→ $user_id` captures the
step's output. `{token: $token}` references a prior capture or input.
No variables, no mutation — just data threading through the pipeline.

### Relation to soma-next

The DSL is a text representation. It parses into the existing Rust
structures: `CompiledRoutine`, `CompiledStep`, `CompiledStep::SubRoutine`
for branches. No runtime changes needed — the DSL is a frontend to
what already exists.

## App Builder: natural language to running app

The routine DSL + server-driven UI definitions enable a builder interface
where the creator describes what they want in natural language and gets
a running app — no code at any step.

### The loop

```
Creator: "I want an appointment booking flow. The user picks a provider,
         sees available time slots, books, and gets a confirmation.
         If there's a conflict, show the next available slot."

  → LLM compiles routine DSL:
      book_appointment, list_providers, get_availability, ...

  → LLM derives UI definitions from routine inputs/outputs/branches:
      provider_list (data ← list_providers)
      time_picker (data ← get_availability)
      booking_form (actions → [validate, book_appointment])
      confirmation_card (data ← get_appointment)
      conflict_notice (trigger ← branch conflict_found)

  → Creator reviews rendered preview, tweaks ("make the time picker
    show weekly view", "add provider ratings to the list")

  → Both routines + UI definitions stored as soma-project data

  → Any client (web, mobile, bot) renders the same app
```

### What each piece is

**Routine DSL** — business logic. Every step is port:capability with
$-bindings. Constrained grammar, deterministic, diffable. LLMs generate
this reliably because the vocabulary is tight (port manifests define what
exists).

**UI definitions** — presentation. A grammar of components (form, list,
card, modal, table, chart...) wired to routine references for data and
actions. The client is a generic renderer that knows how to draw each
component type but doesn't know what the app does.

**Builder interface** — just another soma client. Calls `author_routine`
for logic and stores UI definitions. The creator sees a rendered preview
and natural language descriptions, never the DSL (unless they want to).

### How this differs from code generators

v0, Bolt, Lovable generate React/Next.js source code. The output is a
codebase that needs a developer to maintain, debug, and deploy. Each
generation produces different code. The generated code diverges from the
spec the moment someone edits it.

SOMA generates behavioral data. The output IS the running app — no
compilation, no deployment pipeline, no divergence between spec and
implementation. Edit a routine: behavior changes. Edit a UI definition:
presentation changes. The app evolves from conversation, not from
coding.

### Risk assessment

| Component | Risk | Why |
|---|---|---|
| Routine DSL → soma-next execution | Low | Maps directly to existing CompiledRoutine structs |
| LLM → routine DSL | Low | Constrained grammar + port manifests as context |
| UI grammar for real apps | Medium | Must be expressive enough for production UIs but constrained enough for reliable LLM generation |
| LLM → UI definitions | Medium | Depends on how well the UI grammar is designed |
| Builder interface UX | Known | Hard work but solved design space |

The UI grammar requires design work — choosing component types, properties,
and layout primitives. Every design system (Material, Ant, Shadcn) defines
~30-50 component types that cover 95% of apps. The UI grammar is those
component types + data bindings to routines + action bindings to routines.
Airbnb's SDUI uses ~40 component types for their entire production app.
Not a research problem — design work with known solutions.

## What already exists in soma-next

The runtime already supports everything the routine DSL and execution
model require:

| Capability | Status | Location |
|---|---|---|
| `author_routine` — create routines with structured steps | Done | `mcp.rs` |
| `execute_routine` — run with input bindings, plan-following | Done | `mcp.rs` |
| `DataCondition` — branch on observation.structured_result | Done | `routine.rs` |
| `CompiledStep::SubRoutine` — hierarchical composition | Done | `routine.rs` |
| `NextStep::Goto` — loops with iteration limits | Done | `session.rs` |
| `NextStep::CallRoutine` — push sub-routine onto plan stack | Done | `session.rs` |
| `NextStep::Abandon` — fall back to LLM deliberation | Done | `session.rs` |
| Plan stack depth 16, loop tracking | Done | `session.rs` |
| Input bindings via belief.active_bindings | Done | `session.rs` |
| Episode → Schema → Routine compilation (BMR) | Done | `memory/` |

What needs to be built:

| Component | What it is |
|---|---|
| DSL parser | Text DSL → `CompiledStep` JSON (frontend to `author_routine`) |
| UI grammar | Component type definitions + data/action bindings to routines |
| HTTP bridge | Route mapping: `POST /appointments` → `execute_routine("book_appointment", ...)` |
| Generic renderer | Client that renders UI definitions from server (web, mobile) |
| Builder interface | NL conversation → routine DSL + UI definitions |
| Routine sync | Push new compiled routines from server to offline clients |

## What this is NOT

**Not code generation.** The LLM doesn't write source code that gets
compiled. It executes actions through soma-next, which observes outcomes
and compiles patterns. The artifact is behavioral data, not source code.

**Not a no-code platform.** There is no drag-and-drop UI. The "developer"
is a frontier LLM executing scenarios. The expertise is in designing the
right scenarios and ports.

**Not replacing all software.** Ports still require traditional code
(Rust cdylib crates). The claim is narrower: the orchestration layer —
which controller calls which services in what order with what error
handling — compiles from execution instead of being hand-written.
