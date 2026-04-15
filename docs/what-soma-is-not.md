# What SOMA Is Not

SOMA occupies unfamiliar territory. People encounter it and pattern-match
to things they already know. These clarifications prevent the most common
misunderstandings.

## SOMA is not a code generator

AI coding tools (Copilot, Cursor, Claude Code) generate source code that
humans review, test, deploy, and maintain. The artifact remains. SOMA
generates no code. The runtime receives goals, selects skills, invokes
ports, observes results, and compiles routines from experience. There is
no source file at the end. The closest analogy is not "AI writes the
code for you" but "the program was never written."

## SOMA is not an LLM wrapper

LangChain, CrewAI, AutoGen — these are orchestration layers around LLM
API calls. Remove the LLM and you have nothing. SOMA runs without an
LLM. The autonomous path (create_goal) selects skills, executes through
ports, and compiles routines entirely through its own control loop. The
LLM is one possible brain, not a dependency. Switch LLMs, restart
conversations, change providers — the runtime's belief state, episode
history, and compiled routines persist. The body doesn't forget when the
brain sleeps.

## SOMA is not a workflow engine

YAML-based workflow systems (n8n, Airflow, Prefect, Temporal) define
directed graphs of tasks that execute in order. The developer authors
the graph. SOMA has no workflow graph to author. Skills are declared in
pack manifests; the runtime's control loop decides the execution order
at each step based on scoring, prediction, belief state, and critic
evaluation. When routines compile from experience, the execution order
emerged from observation — nobody designed it.

## SOMA is not merely an MCP server

SOMA exposes an MCP interface (29 tools over JSON-RPC 2.0), but the MCP
server is one of six architectural layers. Underneath it: a 16-step
control loop with budget-constrained deliberation, a three-tier episodic
learning pipeline, a policy engine with seven lifecycle hooks, and a
distributed transport layer. Calling SOMA "an MCP server" is like
calling a database "a TCP listener." The interface is real; the
substance is elsewhere.

## SOMA is not a replacement for systems programming

The runtime itself is written in Rust. Port adapters are compiled shared
libraries. The embedded leaf firmware targets bare-metal ESP32. These are
systems programming. SOMA eliminates application source code — the layer
above the runtime where developers traditionally write controllers,
routes, ORM mappings, serialization, and business logic. The runtime and
its ports are the last programs that need to be hand-written for a given
domain. Everything above them is the runtime's job.

## SOMA is not a database or a knowledge graph

The world state stores facts. The episode store records execution traces.
The schema store holds induced patterns. None of these replace
PostgreSQL, Redis, or Neo4j. SOMA's memory tiers are working memory for
the runtime — what it has observed, what patterns it has detected, what
routines it has compiled. Authoritative data lives in external systems
accessed through ports. SOMA remembers what it has done; the database
stores what is true.

## SOMA is not a chatbot

The soma-project-terminal looks like a chat interface, and an LLM does
the talking. But the terminal is a thin communication layer — it forwards
operator messages to the brain and renders responses. The substance is
in the runtime: port invocations against real systems, episodes
accumulating, routines compiling, the scheduler firing background
actions, the reactive monitor executing autonomous routines. The chat is
the steering wheel, not the engine.

## Where SOMA is a bad fit

SOMA works well for: data-driven applications, CRUD operations, API
orchestration, IoT automation, multi-service coordination, scheduled
workflows, and any domain where behavior is repetitive enough to compile
into routines.

SOMA is a poor fit for:

- **Performance-critical inner loops.** A compiled routine dispatches
  through the control loop and port abstraction. For microsecond-level
  operations (game physics, signal processing, high-frequency trading),
  the overhead is unacceptable. Write those in Rust, C, or CUDA.

- **Rich visual interfaces.** SOMA has no view layer, no component
  model, no rendering pipeline. A product that IS its visual design
  (Figma, Photoshop, a game) needs a UI framework. SOMA can power the
  backend but cannot replace the frontend.

- **Systems requiring formal verification.** The policy engine enforces
  safety constraints, and compiled routines are deterministic. But SOMA
  does not provide mathematical proofs of correctness. For avionics,
  medical devices, or nuclear controls, you need formally verified
  systems, not adaptive runtimes.

- **Tiny scripts.** If the task is "run this SQL query once," a 3-line
  Python script is simpler than bootstrapping a SOMA runtime. SOMA's
  value compounds over time through learning. One-shot tasks don't
  benefit from the learning pipeline.
