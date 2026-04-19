# Tradeoffs

Honest analysis of where SOMA wins, where conventional approaches win,
and the architectural costs of the design decisions.

## Where SOMA wins

### No application code to maintain

A traditional application encodes behavior in source files that must be
reviewed, tested, versioned, deployed, and maintained. SOMA encodes
behavior in pack manifests (declarative), compiled routines (learned),
and the runtime's control loop (fixed). When requirements change, you
update a manifest or let the runtime learn a new routine. There is no
codebase to refactor.

**The cost:** Pack manifests are simpler than source code but still
require authoring. SQL strings in skill declarations are domain logic in
a different notation. The autonomous path (compiled routines) eliminates
even this — but only after enough episodes accumulate.

### Permanent state across LLM sessions

Every LLM agent framework loses context when the conversation ends.
SOMA persists belief state, episodes, schemas, routines, and world state
across sessions, models, and providers. `dump_state` returns the full
runtime context in one call. A new LLM session picks up exactly where
the last one left off.

**The cost:** State accumulates. Episodes fill the ring buffer. World
state facts grow. Routines compile but may become stale. Disk-backed
stores grow over time. No automatic garbage collection beyond the
episode ring buffer eviction and consolidation cycle.

### Compiled routines bypass the LLM

After a behavior pattern is observed three or more times, it compiles
into a deterministic routine that executes without LLM reasoning. The
system gets faster and cheaper as it learns. This is fundamentally
different from every LLM agent framework where every action requires a
model call.

**The cost:** Routine compilation requires multiple successful episodes
with the same skill sequence. Novel situations always need the LLM (or
the autonomous selector). The transition from "LLM-dependent" to
"routine-driven" takes time and repetition.

### Domain-agnostic runtime

The same binary handles databases, email, S3, hardware sensors, HTTP
APIs, and any future port. Adding a capability means loading a new port
library, not modifying the runtime. The control loop, policy engine,
memory pipeline, and distributed transport are universal.

**The cost:** Universality means the runtime doesn't optimize for any
specific domain. A purpose-built invoice system will be faster and more
polished than a SOMA runtime that learned invoicing. SOMA trades
specialization for generality.

### Observation-grounded execution

Every port call produces a typed `PortCallRecord`. The runtime makes
decisions based on what actually happened, not on what the LLM thinks
happened. Hallucination is structurally impossible for port invocations
— if the capability doesn't exist, the call is rejected.

**The cost:** The LLM can still hallucinate when interpreting results
or deciding what to do next. Grounding applies to execution, not to
reasoning. The brain is still probabilistic; only the body is
deterministic.

## Where conventional apps win

### Startup time and simplicity

A Python script with `psycopg2` queries a database in 3 lines. A SOMA
runtime bootstraps from config, loads ports, initializes stores,
and starts background threads. For a single query, the script is
simpler. SOMA's value compounds over time; it doesn't pay off for
one-shot tasks.

### Visual interfaces

SOMA has no view layer. Products where the UI IS the product (design
tools, games, dashboards with complex visualizations) need a frontend
framework. SOMA can power the backend (port calls, data management,
scheduled operations) but the visual interface must be built separately.

### Deterministic, auditable business logic

Traditional source code is reviewable line by line. A compliance officer
can read a function and verify it implements the rule correctly. SOMA's
compiled routines are inspectable (they're data structures, not opaque
weights), but the path from "3 episodes → PrefixSpan → schema → routine"
is harder to audit than a hand-written function. For regulated domains
requiring line-by-line code review, traditional code is more auditable.

### Existing ecosystem integration

The JavaScript/Python ecosystems have libraries for everything. Need
Stripe integration? `npm install stripe`. Need PDF generation?
`pip install reportlab`. SOMA integrates via ports, which must be
written or found. The port ecosystem is growing but cannot match
the scale of npm or PyPI.

### Performance-critical paths

The control loop, port dispatch, and observation recording add overhead
to every operation. For inner loops processing millions of events per
second, this overhead is unacceptable. Traditional compiled code
(Rust, C, Go) operates on bare data structures without runtime
abstraction.

## Architectural tradeoffs

### Body/brain separation

**Benefit:** The runtime (body) is deterministic and persistent. The LLM
(brain) is replaceable and disposable. Switching models loses zero
context.

**Cost:** Two-way communication happens over MCP (JSON-RPC stdio), which
adds serialization overhead and limits throughput. The brain cannot
directly read runtime memory — everything goes through tool calls.

### Episode ring buffer (1024)

**Benefit:** Bounded memory. Oldest episodes are evicted when the buffer
is full. No unbounded growth.

**Cost:** Important episodes can be evicted before consolidation extracts
their patterns. Salience weighting mitigates this (high-value episodes
contribute more to PrefixSpan) but doesn't prevent eviction.

### Plan-following bypasses deliberation

**Benefit:** Compiled routines execute fast — no skill scoring, no critic
evaluation, no policy checks per step (only at entry).

**Cost:** If the environment changed since the routine was compiled, the
routine may execute a stale plan. Invalidation triggers (resource schema
change, precondition failure, confidence drop) catch some cases but not
all. A routine that worked yesterday may fail today if the external
system changed.

### World state as a global fact store

**Benefit:** The reactive monitor can fire autonomous routines when
conditions match. Webhooks, scheduler results, and observations all
patch the same global state.

**Cost:** World state facts accumulate without automatic TTL. Stale
facts (from a webhook received hours ago) may trigger routines
inappropriately. Manual cleanup or TTL policies are needed but not yet
implemented.

### Auto-discovery (--pack auto)

**Benefit:** Zero configuration. The runtime discovers all available
ports from the dylib search path. One command, no manifest.

**Cost:** Auto-discovered ports skip schema validation (the SDK's
self-reported spec may have issues). Observable fields are cleared.
This is pragmatic but means auto-discovered ports have weaker contracts
than manifest-declared ones.

## When to use SOMA

Use SOMA when:

- The application is mostly CRUD, orchestration, and scheduled workflows
- You want the system to learn and get faster over time
- You need permanent state across LLM sessions
- You're building for multiple external systems (database + email + storage + APIs)
- You value the operator experience of "describe what you want" over "click through forms"

Use conventional code when:

- The task is a one-shot script
- The product IS its visual interface
- Performance is measured in microseconds
- Regulatory compliance requires line-by-line code review
- The problem is well-defined and unlikely to change
