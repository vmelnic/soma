# Terminal Multi-Tenancy

**Status:** design note. Not implemented. Reference material for a
future commit that re-opens the "how does `soma-project-terminal`
let multiple operators share the SOMA runtime without writing
wrapper code per port" question.

---

## The question

SOMA's core promise is: **the runtime IS the program**. The LLM
picks a port from a catalog, calls it, SOMA executes it. HelperBook
proves this end-to-end: multiple ports, many capabilities, zero wrapper
code in the application layer.

`soma-project-terminal` breaks that promise today. The app has a
`BRIDGE_PORTS` dispatch table that only allows one port
(`context_kv`) to be invoked from a generated pack. Every new
capability requires writing a handwritten wrapper on the backend
side — which is exactly what SOMA claims to eliminate.

The reason the wrapper table exists is multi-tenancy. The backend
runs **one** `soma-next --mcp` subprocess, shared by every
operator. Its `postgres` port is connected as one PG role that can
read every table. Exposing that raw through the bridge would let
operator A run `SELECT * FROM users` and see operator B's rows.
The handwritten wrapper is the author (me) doing tenancy isolation
by hand because the shared runtime can't do it natively.

The question this doc exists to answer: **what's the right shape
of tenancy for a multi-operator SOMA consumer that doesn't require
handwritten per-port wrappers?**

---

## The principle

Multi-tenancy is a **runtime boundary**, not a routing policy.
Tenancy that lives in application-layer wrappers is leaky by
default — the wrappers become the source of truth, and every new
port is a new policy decision. Tenancy that lives at the runtime
boundary is impermeable by construction — operator A's runtime
physically cannot observe operator B's state because the runtimes
don't share process memory, file descriptors, or database
connections.

SOMA's unit of runtime isolation is the `soma-next` process
itself. A `soma-next` process loads dylib ports from its pack,
initializes them from env vars, and executes `invoke_port` calls
against them. Two different processes with different env vars
have different ports, different configs, different connection
pools — and cannot reach each other's state via any port
operation.

Therefore: **the SOMA-native way to isolate multiple operators is
one `soma-next` process per operator.** Each operator gets their
own body. The backend becomes a lifecycle + routing layer over
a pool of operator-scoped SOMA processes.

This collapses the `BRIDGE_PORTS` wrapper table. The bridge route
becomes a thin pass-through:

```js
POST /api/contexts/:id/port/:portId/:capabilityId
  → lookupOperatorSoma(user.id).invokePort(portId, capId, input)
  → return PortCallRecord
```

No allow-list. No wrapper per port. Every port the operator's
subprocess has loaded is callable by their generated pack skills
for free. Adding a new port to SOMA means rebuilding `soma-ports`
once; the terminal requires zero changes.

---

## The hybrid

Not every port needs per-operator isolation. Two classes:

**Pure ports** — no shared state, no network egress, no
side-effects outside the call:

- `crypto` (sha256, random_string, hmac)
- `image` (resize, encode, decode — on bytes passed in)
- `geo` (distance, bounding box — pure math)
- parts of `timer` (delay, not schedule)

These are safe to expose from a **shared** `soma-next` subprocess
because an operator calling them cannot observe or influence
another operator. They can stay on the backend's main
`SomaMcpClient` and the bridge route routes them there.

**Stateful ports** — shared mutable state, network egress, or
side-effects affecting other tenants:

- `postgres` (SQL against a shared DB)
- `redis` (shared key/value)
- `smtp` (email to arbitrary recipients = spam vector)
- `http.fetch` (SSRF proxy; outbound rate limits)
- `filesystem` (read/write anywhere the process can)
- `s3` (bucket writes, quota burn)
- `push` (notification spam)

These route to the operator's own pooled subprocess. The
subprocess's env is configured so the ports connect to
operator-scoped resources:

```
POSTGRES_URL=...?options=-csearch_path=op_<uuid>
SMTP_ALLOWED_RECIPIENT=<operator's verified email>
FS_ROOT=/var/soma/op_<uuid>/
HTTP_ALLOWED_HOSTS=api.openai.com,api.anthropic.com,...
```

Tenancy is **configuration**, not code. Each port's existing init
logic handles the rest — `postgres` already takes
`SOMA_POSTGRES_URL` and connects there; `smtp` already takes
`SOMA_SMTP_*`. Adding `SMTP_ALLOWED_RECIPIENT` is a one-line
change to `soma-ports/smtp`, NOT a wrapper in the terminal.

---

## The taxonomy in code

`backend/brain.mjs`'s `BROWSER_PORTS` catalog gains a third scope
value. Today:

```js
scope: "wasm"            // dom, audio, voice — run in browser wasm
scope: "backend_bridge"  // context_kv — handwritten wrapper
```

Under the per-operator-subprocess design:

```js
scope: "wasm"              // in-browser wasm runtime
scope: "runtime:shared"    // main backend SomaMcpClient — pure ports
scope: "runtime:operator"  // pooled per-operator subprocess — stateful
```

The JS skill executor dispatches by scope. `backend_bridge`
disappears; `context_kv` becomes one of many `runtime:operator`
ports (or gets retired entirely in favor of raw `postgres`).

---

## Tradeoffs

All the numbers below are estimates, not measurements. A
prerequisite for actually landing this refactor is taking
measurements on real hardware and replacing these guesses. See
"What to measure" at the bottom.

### Memory cost per active operator

**Concern:** every subprocess loads binaries + dylibs +
connection pools.

**Estimate:** ~10-15 MB private dirty per subprocess. RSS is
40-50 MB but most of that is shared code pages (`.text` and
read-only `.rodata` of soma-next + dylibs) that the OS mmaps once
across all processes. At 10 concurrent operators, total private
footprint is ~100-150 MB. At 100, ~1-1.5 GB. The memory ceiling
is somewhere north of 500-1000 concurrent operators before it
becomes a bottleneck.

**Mitigation:** idle-reap subprocesses after N seconds of
inactivity. Active set is ALWAYS smaller than registered-user
set, often by orders of magnitude. Steady-state cost scales with
concurrent-active, not total users.

**Verdict:** not a real concern at the scale this project is
targeting (single developer, small teams, demos). Becomes a
concern past ~500 concurrent operators, at which point you'd
want option D below.

### First-call latency

**Concern:** lazy-spawning a subprocess on demand takes 200-500ms
before the first port call completes.

**Estimate:** 100-300ms for `child_process.spawn` to MCP
`initialize` reply on a warm machine. Additional 10-30ms for
`CREATE SCHEMA` / `CREATE ROLE` on first login.

**Mitigation:** defer spawn until the first bridge call (not
until login). Login stays 0-ms subprocess work. First skill RUN
pays the cold-start cost, which lands inside the "yes it's
working" window for a user who just clicked a button. Subsequent
RUN calls reuse the pooled subprocess (~5-10ms round-trip).

**Verdict:** acceptable. The cold-start cost is paid once per
operator per session, in the path where the user is already
expecting work to happen.

### Schema migration complexity

**Concern:** migrating N operator schemas requires iterating
over every operator, with rollback semantics for partial
failures.

**Mitigation:** split system tables (managed by backend,
migrated via `schema.sql`) from operator tables (managed by
operator skills, migrated by pack regeneration). The terminal's
own `users`, `sessions`, `contexts`, `messages`, etc. stay in
the `public` schema and migrate exactly the way they do today.
Operator-generated tables live in `op_<uuid>` schemas. Pack
skills create them via `CREATE TABLE IF NOT EXISTS`. When the
pack evolves, the operator regenerates the pack, which either
`ALTER TABLE`s idempotently or drops the old schema and starts
over.

**Verdict:** no backend-level migration iteration. Operator
tables are the operator's problem, not ours.

### Postgres role count

**Concern:** creating one PG role per operator scales poorly.

**Estimate:** Postgres handles millions of roles. Not a scale
concern.

**Mitigation:** one idempotent `provisionOperatorSchema(userId)`
helper in `verifyMagicToken`'s new-user branch. Creates schema
+ role + grants if missing, no-ops otherwise. Cleanup on user
delete is one `DROP OWNED BY` + `DROP ROLE`. ~30 lines total.

**Verdict:** operational chore, not an architectural problem.

### Operational complexity of the pool

**This is the real cost.** The preceding tradeoffs are mostly
non-issues when examined; this one is the substantive work.

The pool manager has to handle:

- **Lazy spawn** — first request for a given operator starts the
  subprocess, subsequent requests wait for the in-flight spawn
  rather than double-spawning.
- **Health checks** — periodic `list_skills` to detect a dead
  subprocess (postgres connection timeout, OOM kill, crash).
- **Idle reaping** — subprocess killed after 60s of no calls,
  respawned lazily on next call.
- **Crash recovery** — if a subprocess dies mid-call, the next
  call respawns it and retries.
- **Concurrent safety** — two simultaneous bridge calls for the
  same operator must not race the spawn.
- **Resource caps** — a runaway operator can't hold unbounded
  subprocesses; bounded pool with oldest-idle eviction.
- **Graceful shutdown** — backend SIGTERM reaps every child
  cleanly so tests/CI don't leak zombies.
- **Test harness integration** — the Playwright suite needs to
  work without test operators leaking subprocesses between runs.

This is ~300 lines of straightforward but careful JavaScript.
Not hard per se, but it's the bulk of the implementation work
and there's no way to handwave it away.

---

## Rejected alternatives

### A. Tenant-aware `invoke_port` at the SOMA runtime level

Extend `invoke_port` to carry an optional `tenant_id` header.
Tenant-aware ports (postgres, smtp) use it to scope their
behavior; pure ports ignore it.

This is the **most architecturally clean** long-term answer and
probably what "production SOMA multi-tenant" looks like. But it
requires modifying `soma-next`, `soma-port-sdk`, and
`soma-ports/postgres` + `soma-ports/smtp` + others. Out of scope
for a soma-*consumer* like the terminal — this project consumes
SOMA, it doesn't fork it. If we ever need to ship
multi-tenancy at scale (thousands of concurrent operators), this
is where we'd go.

### B. Row Level Security in a shared PG role

One role, all operator data in shared tables, RLS policies like
`USING (operator_id = current_setting('soma.operator_id'))`. On
every query, set `SET LOCAL soma.operator_id = ...`.

Catch: the `postgres` port's `invoke_port` doesn't expose a hook
for per-call session variables. You'd have to wrap every call
with `SET LOCAL ... ; <sql> ; RESET;` — which **is the same
wrapper-per-port pattern we're trying to eliminate**, just
transposed from JS to SQL. Doesn't solve the "no handwritten
wrapper code" problem. Rejected.

### C. Multiple postgres port instances in one subprocess

Pack declares `postgres_op_<uuid1>`, `postgres_op_<uuid2>`, ...,
each loading a separate `postgres` port instance with its own
init config. One shared subprocess, dynamic port additions per
operator.

Problem: SOMA pack manifests are static at `soma-next` startup.
Ports can't be added to a running subprocess after boot. Either
you'd need to know all operators in advance (impossible for a
public SaaS) or restart the subprocess every time a new operator
logs in (nonsense). Requires SOMA-next changes to support
dynamic port instantiation. Rejected.

### D. Shared runtime with capability routing at the API layer

Build a capability router that knows how to scope each port call
to the operator's tenant. Essentially options B + the generic
version of the `BRIDGE_PORTS` table. Every new port is a new
router entry. This IS the current `context_kv` wrapper approach,
generalized. Rejected for the same reason the user rejected the
wrapper approach in the first place: it's handwritten policy
code, not SOMA-native.

### E. In-process wasm SOMA per operator

Since `soma-next` already compiles to wasm for `soma-project-web`,
load it as an in-process wasm module in the Node backend. Per
operator = Map<userId, WasmSomaInstance>. No subprocess cost at
all.

Problem: the wasm build intentionally excludes native dylib ports
(postgres, smtp, crypto) via feature flags. To make this work
you'd need a wasm soma-next that can bridge to native dylibs via
some host-provided callback mechanism. That's a significant
`soma-next` engineering task — larger than option A. Rejected
for scope, but architecturally interesting if someone ever builds
a "wasm soma with native host callbacks" mode.

---

## Implementation sketch

If and when this gets built:

1. **`SomaMcpClient`** gains an optional `env` override in its
   constructor. The main client keeps its current behavior.
2. **`backend/operator-soma.mjs`** — new. A `Map<userId,
   SomaMcpClient>` keyed by user id, with `get(userId)` lazily
   spawning and returning a client. Handles lifecycle, idle
   reaping, crash recovery, graceful shutdown.
3. **`verifyMagicToken`** gets a `provisionOperatorSchema(userId)`
   call that creates `op_<short_id>` schema + role idempotently.
4. **Bridge route** (`/api/contexts/:id/port/:portId/:capId`)
   drops the `BRIDGE_PORTS` table. Inspects port scope from
   `BROWSER_PORTS`, dispatches to main or operator client
   accordingly. No allow-list.
5. **`backend/contextkv.mjs`** can be retired — operators now
   call `postgres.execute` directly — or kept as a convenience
   shim for packs that don't want to write DDL.
6. **`BROWSER_PORTS`** in `brain.mjs` gains every port the
   operator subprocess loads. Chat + pack prompt catalogs
   update. The chat brain can truthfully say "postgres,
   smtp, http, crypto are available" because they actually
   are.
7. **Tests**: isolation tests become proofs of
   **subprocess-level** isolation via actual SQL from operator
   A's subprocess against operator B's schema, which fails at
   the role-grant level. Stronger guarantee than wrapper-level
   isolation.

---

## What to measure before building this

Real numbers to replace the estimates in this doc:

- **Private-dirty RSS** of a fresh `soma-next --mcp` subprocess
  with crypto + postgres + smtp loaded. `pmap -x <pid>` on Linux,
  `vmmap <pid>` on macOS. Reality might be 5 MB or 25 MB; my
  estimate of 10-15 is a guess.
- **Cold-start spawn time** from `child_process.spawn` to first
  MCP `initialize` reply. Wrap `performance.now()` around
  `soma.start()`. Expect 100-500ms depending on dylib load
  cost and whether the binary is in the page cache.
- **Per-operator schema creation time** including `CREATE
  SCHEMA`, `CREATE ROLE`, and the grants. Probably 10-30ms
  total, but measurable.
- **First invoke_port round trip** after spawn. Expected ~5-10ms
  but measure.
- **Maximum sustainable concurrent subprocess count** on the
  target deployment. Spawn 100, 200, 500 operators in sequence
  and observe RSS, fd count, postgres connection count.
  Determines the per-machine scale ceiling.

Any of these can be wrong by an order of magnitude; the design
above is robust to a 3x error but not to a 30x one.

---

This doc captures the target state so the next iteration can
open with a clear picture instead of re-deriving the tradeoff
table from scratch. For the current implementation state of
the terminal, read the code in `soma-project-terminal/` — the
wrapper table, bridge route, and `BROWSER_PORTS` catalog make
the shortcomings concrete and will evolve as the design lands.
