# Vision

## What SOMA Is

SOMA (Greek *soma* = body). A computational paradigm where the runtime IS the
program. No application source code. A goal-driven runtime receives intents,
selects skills, invokes ports (external system adapters), and orchestrates
execution. Like a brain is to walking -- the neural architecture IS the program.

The runtime is a single Rust binary. It boots from declarative pack manifests
that describe ports, skills, schemas, routines, and policies. An LLM (or any
other caller) drives it through MCP. The runtime drives external systems through
dynamically loaded port libraries. There is no generated code, no intermediate
artifact, no compilation step between intent and execution.

## Why SOMA Exists

Current AI-assisted coding is a Rube Goldberg machine: a system that understands
intent produces source code that doesn't understand intent, then feeds it into a
dumb executor. The entire middle layer -- source files, build tools, dependency
managers, deployment pipelines -- exists because traditional runtimes cannot
accept intent directly.

SOMA eliminates the intermediate artifact. Intent in, execution out.

This is not "low-code" or "no-code." Those still produce application artifacts.
SOMA produces nothing. The runtime receives a goal, reasons about it, selects
skills, calls ports, observes results, and adapts. The application exists only
as a running instance: its pack manifests, its belief state, its accumulated
episodes.

## Web 4: Neural Execution

| Era   | Model                    | What exists                        |
|-------|--------------------------|------------------------------------|
| Web 1 | Static pages             | HTML files served by Apache        |
| Web 2 | User-generated content   | Databases behind application code  |
| Web 3 | Decentralized ledgers    | Smart contracts on blockchains     |
| Web 4 | Neural execution         | Runtime instances, not source code |

Web 4 applications are runtime instances. They have no repository of application
source code. An LLM provides the intent layer; SOMA provides the execution
layer. The application is the conversation between them -- a live, adaptive,
observable process.

This means:

- **No source to maintain.** Behavior changes by updating pack manifests, skills,
  or policies -- not by editing, reviewing, and deploying code.
- **No context loss.** The runtime persists its own state: belief, episodes,
  schemas, sessions. Any LLM, any session, full understanding via `dump_state`.
- **No deployment pipeline.** The runtime is always running. New capabilities
  arrive as loaded packs or ports.

## How It Works Today

**soma-next** -- Rust runtime (~15K lines, single binary). Goal-driven control
loop with selection, prediction, criticism, and policy enforcement. Typed skills
and ports. Belief state and resource tracking. Episode memory with persistence.
Session checkpoints and restore. Distributed peer transport (TCP, TLS, WebSocket,
Unix sockets). MCP server with 14 tools. CLI with 10 commands.

**soma-ports** -- 11 dynamically loaded port adapters in a Rust workspace:
postgres, redis, auth, smtp, s3, crypto, geo, image, push, timer, plus an SDK
crate. Each port is a shared library exporting `soma_port_init`. Ed25519
signature verification for untrusted ports.

**soma-project-*** -- Self-contained proof projects (smtp, s3, postgres). Each
proves that a real-world integration works end-to-end through the SOMA paradigm:
pack manifest, port, skills, goal execution.

**soma-helperbook** -- First real application. Service marketplace with a
19-table PostgreSQL schema, Redis sessions, Express frontend, and three loaded
ports. Users, connections, messages, appointments, reviews -- all managed through
SOMA goals, not application code.

## The LLM Context Problem Solved

Traditional approach: an LLM reads 20,000 lines of application code to
understand what the application does. It loses context across sessions. It
hallucinates about code it hasn't read. Every new session starts from scratch.

SOMA approach: an LLM calls `dump_state` and receives complete runtime context
in ~5KB. Loaded ports, registered skills, active sessions, recent episodes,
current metrics, belief state. Zero context loss across sessions. Any LLM, any
session, full understanding in one call.

This is not a side benefit. It is the enabling mechanism. SOMA applications are
operable by LLMs precisely because the runtime is self-describing. The runtime
knows what it can do (proprioception), what it has done (episodes), and what it
believes (belief state). It exposes all of this through a single MCP tool.

## Key Principles

**Runtime is the program.** There is no source code artifact. The runtime, its
pack manifests, and its accumulated state ARE the application.

**Embodiment.** Each SOMA instance knows its own capabilities -- which ports are
loaded, which skills are registered, what resources are available. This is
proprioception: the runtime's self-model.

**Intent as input, execution as output.** Goals go in. Port calls, observations,
and outcomes come out. The control loop handles selection, sequencing, error
recovery, and adaptation.

**Observation-driven.** Every port call produces a typed record: what was called,
what was returned, how long it took, whether it succeeded. Episodes accumulate.
The runtime learns from its own execution history.

**Self-adaptive.** Belief state updates from observations. The policy engine
enforces risk budgets, latency budgets, and resource limits. Sessions can be
paused, inspected, resumed, or aborted. The runtime is always inspectable.

**Universal.** The same runtime handles email, databases, object storage,
authentication, geolocation, image processing, cryptography, push notifications,
and timers. New capabilities arrive as ports, not as application rewrites. The
architecture is domain-agnostic; the packs are domain-specific.

## What This Changes

A developer building a new application writes:

1. A pack manifest declaring ports, skills, schemas, and policies.
2. Port adapters if the needed ones don't exist yet.
3. Nothing else.

No controllers. No routes. No ORM mappings. No serialization layers. No build
configuration. No CI pipeline for the application itself. The runtime already
exists. The ports already exist. The application is the manifest plus the running
instance.

This is the bet: that the intermediate artifact -- source code -- is a
historical accident of dumb executors, and that intelligent runtimes make it
unnecessary.
