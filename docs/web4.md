# Web 4: Neural Execution

**A vision document for SOMA and the next evolution of the web.**

---

## The Evolution of the Web

The web has reinvented itself three times. Each generation changed what software *is*.

**Web 1.0 (1990s): Static pages.** HTML files served over HTTP. Read-only. A human writes a page, uploads it to a server, the world reads it. The web as a library.

**Web 2.0 (2000s-present): Dynamic applications.** JavaScript, databases, user-generated content. The web as a platform. But this came at a cost: React, Angular, Vue, Next.js, Webpack, TypeScript, REST, GraphQL, Docker, Kubernetes. Every generation of tooling adds complexity. Every framework demands specialists. Every codebase accumulates technical debt.

**Web 3.0 (2010s): Decentralized infrastructure.** Blockchain, smart contracts, crypto. Changed where data lives and who controls it, but did not change how software is built. You still write Solidity. You still debug code. You still maintain codebases.

**Web 4.0: Neural execution.** No application code. The program *is* the neural architecture. Human intent goes in. Execution comes out. No codebases. No frameworks. No technical debt. The seventy-year translation layer between human intent and machine execution disappears.

### The Translation Problem

Every program ever written follows the same pattern: a human understands what needs to happen, then translates that understanding into code. The code is an intermediate artifact -- a lossy encoding of the human's intent in a formal language the machine can execute.

This translation has defined computing for seventy years. It requires specialized training. It produces artifacts (codebases) that grow in complexity, accumulate technical debt, and eventually become unmaintainable. The gap between "what I want" and "what the machine does" is bridged by millions of developers writing billions of lines of code.

AI code generation (Copilot, Cursor, Claude Code) accelerates the translation. You still get code. You still maintain it. You still debug it. The gap narrows, but it remains.

SOMA eliminates the gap. Not by generating code more efficiently -- but by removing application code from the equation entirely.

---

## The Architecture

SOMA (from Greek *soma*, "body") is a computational organism with four components, each with a distinct role:

- **LLM** -- the brain. Temporary, replaceable. Understands human language. Claude today, ChatGPT tomorrow, a local Ollama model next week. Any LLM works.
- **SOMA** -- the body and memory. Permanent. Executes programs, holds state, adapts from experience. A single Rust binary.
- **MCP** -- the nerve between brain and body. Model Context Protocol. The LLM calls SOMA via MCP tool calls. SOMA exposes its full state back via MCP.
- **Synaptic Protocol** -- SOMA-to-SOMA communication. Binary wire protocol, 22 bytes overhead per signal. How SOMAs talk to each other, not to LLMs.

```
+-------------------------------------------------+
|  Human (builder, user, operator)                |
+------------------------+------------------------+
                         | natural language
                         v
+-------------------------------------------------+
|  LLM (Claude, ChatGPT, Ollama, etc.)           |
|  Understands human -> decomposes into calls     |
+------------------------+------------------------+
                         | MCP (tool calls + state queries)
                         v
+-------------------------------------------------+
|  SOMA Core (single Rust binary)                 |
|  +----------+ +----------+ +--------------+     |
|  |   Mind   | | Synaptic | | MCP Server   |     |
|  |  Engine  | | Protocol | | (LLM <> SOMA)|     |
|  +----+-----+ +----+-----+ +------+-------+     |
|  +----+-------------+-------------+---------+   |
|  |  Plugin Manager                          |   |
|  +----+------------------------------------+    |
|  +----+------+  +--------------+                |
|  |  Memory   |  |Proprioception|                |
|  | (LoRA +   |  | (self-model) |                |
|  | checkpoint)|  +--------------+                |
|  +-----------+                                  |
+-------------------------------------------------+
         |                        |
    +----+----+              +----+----+
    | Plugins |              |  Peer   |
    | (body)  |              |  SOMAs  |
    +---------+              +---------+
```

The key insight: **SOMA is a pure executor.** It does not converse. It does not ask questions. It does not understand product specifications or generate natural language. It receives structured intents, generates programs, executes them, and returns structured results.

Conversational intelligence -- understanding, reasoning, explaining -- belongs to the external LLM layer. This is a deliberate design choice, not a limitation. Conversational understanding requires 1B+ parameters. SOMA must run on an ESP32 with 50K parameters. Keeping conversation external keeps SOMA small, universal, and focused on what it does best: execute.

See *SOMA Whitepaper, Sections 1-3* for the full architectural rationale.

---

## No Application Code

This is the hardest concept to internalize. SOMA does not generate code -- it eliminates code entirely.

The Mind (a trained neural model -- BiLSTM encoder + GRU decoder) maps structured intents to **programs**: sequences of plugin convention calls with resolved arguments. These programs are ephemeral internal data structures. They are never serialized to files. They are never written in a programming language. They exist only during execution, then they are gone.

Example -- "list files in /tmp and cache the result":

```
Step 0: fs.list_dir("/tmp")              <- literal string argument
Step 1: redis.set("files:tmp", $0, 300)  <- $0 refs step 0 result, 300 is TTL
Step 2: EMIT($0)                         <- return step 0 result to caller
Step 3: STOP
```

This is not code. It is a data structure produced by neural inference. The Mind learned to compose these programs from training data. No human writes or maintains them.

### The Comparison

| Aspect | Traditional Software | AI Code Generation | SOMA |
|---|---|---|---|
| Artifact | Source code | Source code (AI-generated) | Neural weights (no code) |
| Maintenance | Manual | Regenerate (may break) | Adapts from experience |
| Context | Developer's head | LLM context window (lost) | SOMA state (permanent) |
| Platform | Framework-specific | Same | Plugin-based (swap renderer) |
| Scaling | Rewrite/refactor | Regenerate (risky) | Add plugins, grow model |
| Embedded | Separate toolchain | Poor embedded support | Same architecture, smaller model |
| Collaboration | Git, PRs, meetings | Prompt sharing | Query same SOMA state |

### What This Means in Practice

- **No codebases to maintain.** There is no `src/` directory. There are no 200,000-line repositories. There are neural weights, plugin binaries, and state.
- **No technical debt.** Programs are ephemeral. Every execution produces a fresh program from current weights. There is no legacy code because there is no code.
- **No frameworks.** No React. No Next.js. No Spring Boot. No Django. Capabilities come from plugins. The Mind orchestrates them.
- **No build pipelines.** No Webpack. No Docker builds for the application. The SOMA binary is pre-built. Plugins are pre-built. The Mind model is pre-trained by the Synthesizer (a Python build tool, used once, not at runtime).

The only things that are written and maintained: the SOMA core runtime (a single Rust binary), plugins (Rust shared libraries), and the Synthesizer (Python, build-time only). Everything else -- every application, every feature, every workflow -- lives as neural weights and plugin state.

See *SOMA Whitepaper, Sections 2.1 and 4* for the program structure and neural architecture.

---

## Permanent State, Ephemeral Brain

### The Context Loss Problem

LLMs lose context. This is their fundamental limitation for persistent work:

- After 100 messages, the LLM forgets what was said at message 1.
- After the conversation closes, everything is gone.
- If you switch LLMs, context does not transfer.
- If you paste your project state into a new conversation, it is lossy and manual.

Today, the LLM tries to be both brain and memory. It fails at memory. Context degrades over time. Work gets repeated. Decisions are forgotten.

### The Paradigm Inversion

```
Today's paradigm:
  LLM is brain AND memory -> memory degrades -> context lost -> work repeated

SOMA paradigm:
  LLM is brain (temporary, replaceable)
  SOMA is body + memory (permanent, queryable)
  -> memory never degrades
  -> context always available
  -> LLMs are interchangeable
```

SOMA holds the truth permanently:

| SOMA (permanent, queryable) | LLM (ephemeral, per-session) |
|---|---|
| Database schema and data | Current conversation thread |
| Plugin configuration | Human's conversational tone |
| Business rules (experiential memory) | Reasoning behind current question |
| Decision log (why things were built) | Creative suggestions in progress |
| Execution history | Draft ideas not yet committed |
| LoRA adaptations | -- |
| Render state (UI structure) | -- |
| Checkpoints (full snapshots) | -- |
| Connected peers and topology | -- |

**Rule: if it matters beyond the current conversation, it goes in SOMA. If it is only relevant right now, it stays in the LLM.**

### soma.get_state() -- One Call, Full Context, One Second

When any LLM starts a new session with a SOMA, it makes one call:

```json
{
  "soma_id": "helperbook-backend",
  "version": "0.1.0",
  "uptime": "47h 23m",
  "plugins": {
    "loaded": [
      {"name": "postgres", "conventions": 12, "healthy": true},
      {"name": "redis", "conventions": 14, "healthy": true},
      {"name": "auth", "conventions": 12, "healthy": true}
    ],
    "total_conventions": 53
  },
  "database": {
    "tables": {
      "users": {"columns": ["id", "phone", "name", "role", "bio"], "row_count": 1234}
    },
    "total_tables": 15
  },
  "decisions": {
    "recent": [
      {"when": "2h ago", "what": "added waitlist table",
       "why": "walk-in clients need separate queue"},
      {"when": "1d ago", "what": "created review system",
       "why": "both parties review after service completion"}
    ],
    "total": 47
  },
  "experience": {
    "total_executions": 4521,
    "success_rate": 0.97,
    "adaptations": 89
  }
}
```

One call. The LLM knows everything. Takes approximately one second.

### LLM Switching -- Zero Context Loss

```
Session 1 (Monday, Claude):
  Human: "Create a users table with phone, name, role"
  Claude -> SOMA: soma.execute("CREATE TABLE users (...)")
  SOMA: table created, state updated

Session 2 (Wednesday, ChatGPT):
  Human: "Add a bio field to the users table"
  ChatGPT -> SOMA: soma.get_schema()
  SOMA returns: {users: {columns: [id, phone, name, role]}}
  ChatGPT knows the full schema without ANY prior conversation
  ChatGPT -> SOMA: soma.execute("ALTER TABLE users ADD COLUMN bio TEXT")

Session 3 (Friday, local Ollama):
  Human: "What does the users table look like?"
  Ollama -> SOMA: soma.get_schema()
  SOMA returns: {users: {columns: [id, phone, name, role, bio]}}
  Ollama: "The users table has: id, phone, name, role, and bio."
```

Three different LLMs. Three different sessions. Zero context loss. SOMA holds the truth.

See *SOMA Whitepaper, Section 2.6* and *[MCP Interface](mcp-interface.md), Section 4* for the full treatment.

---

## The Interface: Pure Rendering

In Web 4, the frontend is not written. It is rendered by a neural mind from semantic data.

### Interface SOMA

The Interface SOMA is a SOMA instance that runs on the user's device -- browser, phone, or tablet. Its body is the display, input methods, and device sensors. Its purpose: receive semantic signals from Backend SOMAs and render adaptive interfaces.

It is NOT a frontend framework. It is NOT a template engine. It is NOT a conversational partner. It is a neural mind that composes visual output from semantic data, using its device as its body.

### Semantic Signals, Not HTML

Backend SOMAs send *meaning*, not markup:

```json
{
  "view": "contact_list",
  "data": [
    {"name": "Ana M.", "service": "Hair Stylist", "rating": 4.8,
     "online": true, "distance_km": 2.3, "badges": ["verified"]}
  ],
  "actions": ["chat", "book", "favorite"],
  "filters": ["service", "location", "rating"]
}
```

The Interface SOMA decides HOW to render based on:

- **Proprioception** -- screen size, device type, orientation, accessibility settings
- **Design knowledge** -- absorbed from pencil.dev .pen files as LoRA weights

Same signal renders as a grid on desktop, a list on phone, voice output on a speaker. No media queries. No responsive breakpoints. The Mind makes a neural decision based on its knowledge of the device and the design language.

### Design Absorption

Designs are not implemented as CSS. They are absorbed as neural knowledge:

```
1. Designer creates UI in pencil.dev
   (components, colors, typography, spacing, layout patterns)

2. Export as .pen file

3. Synthesizer generates training data from design tokens

4. Train LoRA weights on design-specific examples

5. Interface SOMA loads design LoRA
   -> immediately renders in the design language
```

Dark mode is a different design LoRA. Brand refresh means re-training the LoRA and hot-loading it. The Interface SOMA re-renders without a page reload, without a deploy.

No human writes CSS. No human maintains a component library. The design lives as neural knowledge -- learned from design files, expressed through rendering.

### Cross-Platform from One Model

The Mind generates abstract render programs. The renderer plugin determines the output:

```
Mind generates: create("list_item", {text: "Ana", subtitle: "Stylist"})

DOM renderer:     -> <div class="list-item">...</div>
UIKit renderer:   -> UITableViewCell with textLabel + detailTextLabel
Compose renderer: -> ListItem(headlineContent = "Ana", ...)
```

Same Mind. Same program. Different renderer plugin. The renderer is the only thing that changes between platforms.

### Browser Deployment

The Interface SOMA compiles to WebAssembly (~1-3MB). Synaptic Protocol runs over WebSocket transport. First meaningful paint: approximately 550ms after WASM is cached.

```html
<!DOCTYPE html>
<html>
<head><title>HelperBook</title></head>
<body>
  <div id="soma-root"></div>
  <script type="module">
    import init from './soma-interface.js';
    await init();
    // SOMA takes over #soma-root
    // Connects to Backend SOMA via WebSocket
    // Renders everything from Mind + Design LoRA
  </script>
</body>
</html>
```

No React. No Webpack. No node_modules. A bootstrap HTML file and a WASM binary.

See *[Architecture](architecture.md)* for the complete rendering specification and *SOMA Whitepaper, Section 10* for the overview.

---

## The Builder Workflow

Building an application in Web 4 is a conversation, not a coding session.

### Step 1: Connect and Bootstrap

A human opens any LLM with SOMA connected via MCP. The LLM immediately calls `soma.get_state()` to understand what exists.

### Step 2: Describe What You Want

The human describes the application. They can drop a spec file, explain verbally, or iterate incrementally. The LLM understands the intent.

### Step 3: The LLM Builds via MCP

The LLM translates human intent into structured MCP calls. It installs plugins, creates tables, configures authentication, sets up business logic -- all without the human writing a single line of code.

```
Human: "I want to build an app where people find and book
        service providers."

Claude: -> soma.install_plugin("postgres")
Claude: -> soma.install_plugin("redis")
Claude: -> soma.install_plugin("auth")
Claude: -> soma.install_plugin("calendar")
Claude: -> soma.install_plugin("search")

Claude: "Plugins installed. Let me create the data model."

Claude: -> soma.postgres.execute("CREATE TABLE users (...)")
Claude: -> soma.postgres.execute("CREATE TABLE connections (...)")
Claude: -> soma.postgres.execute("CREATE TABLE appointments (...)")
Claude: -> soma.record_decision({
             what: "Created users table with role ENUM(client,provider,both)",
             why: "Every user has one account with dual role capability"
           })

Claude: "Tables created. Now let me set up authentication..."
```

### Step 4: Every Change Is Recorded

The LLM records decisions after making structural changes. Not just *what* was built, but *why*:

```
soma.record_decision({
  what: "Created waitlist table with max_capacity and position columns",
  why: "Walk-in clients should queue separately when all slots are booked",
  context: "User requested waitlist feature for busy salon periods"
})
```

### Step 5: Continue from Any LLM, Any Session

A new LLM session -- same LLM or different -- calls `soma.get_state()` and has full context. It sees every table, every plugin, every decision. It continues from exactly where the last session left off.

```
Week 1: Claude builds the core data model and auth
Week 2: Switch to local Ollama (privacy concerns)
  Ollama: -> soma.get_state()
  Ollama knows EVERYTHING Claude built
  Ollama continues from where Claude left off
Week 3: Switch to ChatGPT (better at a specific task)
  ChatGPT: -> soma.get_state()
  ChatGPT continues from where Ollama left off
```

Zero context loss. Zero rework. The human never writes code. The human never explains what was already built. SOMA remembers everything.

### Team Collaboration

Multiple people, using different LLMs, can work on the same SOMA simultaneously:

```
Person A (Claude): "Add a reviews table"
Claude: -> soma.postgres.execute("CREATE TABLE reviews (...)")
Claude: -> soma.record_decision({what: "reviews table", why: "..."})

Person B (ChatGPT): "What tables exist?"
ChatGPT: -> soma.get_schema()
ChatGPT: "I see 16 tables including a reviews table added 5 minutes ago."
```

No Git conflicts. No merge issues. No PRs. Both LLMs query the same SOMA. SOMA is the shared truth.

See *[MCP Interface](mcp-interface.md), Section 7* for the complete interaction patterns.

---

## Coexistence with Traditional Software

SOMA does not require replacing existing systems. The migration is gradual.

### Model A: SOMA Behind a Traditional API

The HTTP bridge plugin serves REST or GraphQL. Existing frontends call the API as before. The backend is a SOMA instead of a Node/Python/Java server. Transparent to clients. No frontend changes needed.

### Model B: SOMA Orchestrating Legacy Services

The MCP bridge plugin connects to existing systems via their MCP servers. SOMA orchestrates Stripe, GitHub, Slack without replacing them. Each external MCP tool becomes a SOMA convention automatically.

### Model C: Gradual Migration

One microservice at a time becomes a SOMA. The MCP bridge maintains communication with remaining traditional services. No big bang rewrite.

```
Traditional Architecture          Gradual Migration              Full SOMA

+-------+ +-------+ +-------+   +-------+ +------+ +-------+   +------+
|Node.js| |Python | |Java   |   |Node.js| | SOMA | |Java   |   | SOMA |
|  API  | |  API  | |  API  |   |  API  | |      | |  API  |   |      |
+-------+ +-------+ +-------+   +-------+ +------+ +-------+   +------+
    |         |         |            |        |         |           |
+---+---------+---------+---+   +----+--------+---------+--+   +---+---+
|      Load Balancer        |   |      Load Balancer       |   |Plugins|
+---------------------------+   +---------------------------+   +-------+
```

### When to Use SOMA vs Traditional Code

SOMA excels at: data-driven applications, CRUD, API orchestration, IoT, automation, multi-service coordination -- the vast majority of software built today.

Traditional code remains better for: performance-critical inner loops (game engines, codecs, real-time signal processing), UI framework internals (though SOMA uses them as plugins), and systems where formal verification is required.

The boundary is clear: if the work is translating human intent into coordinated operations across services, SOMA eliminates the translation. If the work is implementing a fast Fourier transform or a video codec, write Rust.

See *SOMA Whitepaper, Section 14* for the full coexistence and migration analysis.

---

## What This Enables

### Apps Built in Conversation, Not in Code

A non-technical founder describes their product to Claude. Claude builds it on SOMA via MCP. The founder never sees code, never hires a developer for the initial build, never deals with technical debt. When they want changes, they describe them. The LLM makes them.

This is not "no-code" in the Bubble/Webflow sense. Those platforms still have code underneath -- they just hide it behind a visual interface. SOMA has no application code at all. The neural weights *are* the application.

### LLM-Agnostic Development

Today, committing to an AI assistant means committing to its ecosystem. Claude's memory does not transfer to ChatGPT. ChatGPT's custom instructions do not transfer to Claude.

With SOMA, the LLM is truly interchangeable. All persistent state lives in SOMA. Any LLM that supports MCP can drive any SOMA. Switch LLMs for capability, for cost, for privacy, for preference -- without losing a single byte of context.

### Self-Improving Systems

SOMA records successful executions as experiences. Periodically, LoRA weights are updated via gradient descent. The SOMA gets measurably better at its specific workload over time. Patterns that prove reliable are consolidated into permanent memory -- the SOMA literally cannot un-learn them.

This is not artificial general intelligence. It is narrow, measurable improvement: the SOMA that has executed 10,000 PostgreSQL queries for a salon booking app gets better at generating the right queries for that specific domain. Like a specialist who gets better with practice.

### Universal Deployment

Same architecture, different scale:

| Target | Parameters | Model Size | RAM | Conventions |
|---|---|---|---|---|
| ESP32 (no PSRAM) | ~50K | ~100KB | ~168KB | 8-16 |
| ESP32 (PSRAM) | ~200K | ~400KB | ~2MB | 16-32 |
| Raspberry Pi | ~800K | ~1.6MB | ~20MB | 32-64 |
| Desktop/Server | ~800K-50M | 3MB-200MB | 50MB-2GB | 64-500+ |

An ESP32 SOMA reads temperature sensors via I2C and reports to a hub SOMA via Synaptic Protocol. A server SOMA runs a full web application with 20+ plugins. Same core binary. Same architecture. Same protocol. Different body.

### Institutional Memory

Personnel change. Tools change. Decades pass. SOMA state persists.

Every decision recorded with its reasoning. Every schema change tracked. Every plugin configuration versioned. When a new team member joins, they connect their LLM to SOMA and call `soma.get_state()`. They have the complete history -- not just what exists, but why each thing was built.

This solves a problem that no version control system addresses: Git tracks *what* changed in code. SOMA tracks *why* decisions were made, in natural language, permanently.

---

## The Path Forward

SOMA is under active development. The Rust runtime compiles, passes 101 tests, and runs as a 14MB binary. Six plugins are implemented (PostgreSQL, Redis, Auth, Crypto, Geo, HTTP Bridge). The Synthesizer trains minds and exports ONNX models. HelperBook -- a full messaging and booking application -- is the first real-world validation.

The pragmatic Interface SOMA (a simple JS renderer, not the neural version) is the near-term frontend path. The neural Interface SOMA described in this document is the research direction -- it adds value when adaptive rendering, design absorption, and cross-platform neural rendering are proven to outperform a well-built traditional frontend.

This is not a theoretical exercise. The code exists. The architecture is validated against specifications with 326+ checklist items passing. What remains is proving it in production with real users.

Web 4 is not a marketing term. It is the observation that neural execution -- intent in, execution out, no code in between -- is a fundamentally different computational paradigm from anything the web has seen before. SOMA is an implementation of that paradigm.

The seventy-year translation layer between human intent and machine execution is ending. Not because AI writes better code. Because the code is no longer necessary.

---

*For the complete technical specification, see the [SOMA Whitepaper](../SOMA_Whitepaper.md). For the conversational interaction model, see [MCP Interface](mcp-interface.md). For the rendering architecture, see [Architecture](architecture.md).*
