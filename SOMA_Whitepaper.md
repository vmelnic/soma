# SOMA: Toward a Universal Self-Adaptive Neural Architecture for Direct Intent-to-Execution Computing

**Version 0.1 — Draft for Discussion**
**April 2026**

---

## Abstract

The current paradigm of software development — whether manual or AI-assisted — relies on generating human-readable source code as an intermediate representation between intent and execution. This paper proposes **SOMA** (from Greek *σῶμα*, "body"), a fundamentally new computational paradigm in which a neural architecture is synthesized directly onto a hardware target (or host environment), forming an embodied computational organism that maps human intent to hardware execution without programming languages, compilers, or traditional software as intermediaries. A SOMA instance knows its own body — registers, ports, interrupts, memory, APIs — and orchestrates it directly in response to natural intent. We present the theoretical foundations, architectural principles, synthesis model, and a research roadmap toward realizing this vision.

---

## 1. Introduction

### 1.1 The Problem

For seven decades, software development has evolved by stacking abstraction layers: machine code, assembly, high-level languages, frameworks, and now AI-assisted code generation. Each layer exists because humans cannot efficiently think at the layer below. The latest iteration — using large language models to generate source code — does not break this pattern. It merely automates the production of an intermediate artifact (code) that must still be compiled, debugged, deployed, and maintained.

This approach suffers from fundamental inefficiencies:

- **Translation loss.** A system that understands human intent produces an artifact (source code) that does not understand intent, only to feed it into a dumb executor (compiler/interpreter). Information is lost at every boundary.
- **Fragility.** AI-generated code frequently fails because the model's understanding of the desired behavior does not survive the translation into syntax and semantics of a programming language.
- **Redundancy.** If a system can understand what a human wants, requiring it to express that understanding as Java or Python before anything happens is an unnecessary detour.
- **Human bottleneck.** Developers remain in the loop not because their judgment is needed at every step, but because the artifacts (code) require human-legible form for debugging and maintenance.

### 1.2 The Thesis

We propose eliminating the intermediate artifact entirely. In the SOMA paradigm:

1. A **base neural architecture** is **synthesized** (not trained in the LLM sense, but compiled/fitted) onto a specific hardware target or host environment.
2. The resulting **SOMA instance** possesses intimate knowledge of its own computational body — every capability, constraint, I/O channel, and resource.
3. At runtime, a human provides **intent** via any natural interface (text, voice, or future neural interfaces).
4. The SOMA instance **directly orchestrates** its hardware/environment to produce the desired output, using internal representations that are emergent from the synthesis process — not human-designed programming languages.

No code is written. No code is generated. No code exists. The SOMA **is** the program.

### 1.3 Terminology

| Term | Definition |
|---|---|
| **SOMA** | A neural computational organism synthesized onto a specific hardware or environment target, capable of directly executing human intent. |
| **Synthesis** | The process of compiling/fitting a base neural architecture to a specific target, producing a SOMA instance. Analogous to compiler backend code generation, but producing a neural execution structure rather than machine instructions. |
| **Body** | The complete hardware and/or environment a SOMA inhabits — registers, memory, I/O ports, OS APIs, network interfaces, peripherals. |
| **Proprioception** | A SOMA's knowledge of its own body, capabilities, and constraints. |
| **Neuronal Language** | The internal representational structure a SOMA uses to orchestrate execution. Emergent from synthesis, not human-designed. Not intended to be human-readable. |
| **Intent Interface** | The boundary where human intent enters the SOMA — text, voice, gesture, neural signal. |
| **Soma Network** | Multiple SOMA instances communicating and composing to accomplish distributed tasks. |
| **Synthesizer** | The system that performs synthesis — compiling the base architecture onto a target body. |

---

## 2. Foundational Principles

### 2.1 The Model Is the Program

In traditional computing, a program is a static sequence of instructions that a processor executes. In SOMA, the neural structure **is** the execution logic. There is no separation between "the software" and "the thing that runs the software." The SOMA does not interpret instructions — it **is** the instruction.

This mirrors biological systems. A human brain does not execute a "walking program." The neural structure that produces walking **is** the walking. Motor patterns, sensory feedback, balance correction — all are embedded in the architecture itself, not in an external script the brain reads.

### 2.2 Embodiment

Every SOMA is synthesized for a specific body. A SOMA on an ESP32 with 520KB of SRAM and GPIO pins is a fundamentally different organism than a SOMA on an M4 MacBook with 32GB of unified memory and macOS system calls. They share a common base architecture, but synthesis produces a body-aware instance.

This is analogous to how a single genome produces radically different organisms depending on environmental signals during development — or, in the compiler analogy, how the same C program produces different machine code for ARM versus x86.

A SOMA knows:
- What computational resources it has (memory, processing units, clock speeds).
- What I/O is available (GPIO, USB, network, display, audio, file system).
- What environment it operates in (bare metal, RTOS, Linux, macOS, Android).
- What it **cannot** do — resource boundaries are part of proprioception.

### 2.3 Synthesis, Not Training

Current neural networks are trained on datasets to minimize a loss function. SOMA synthesis is a different process, closer to compilation:

- **Input:** Base neural architecture + target body specification (hardware manifest, OS API surface, peripheral map, resource constraints).
- **Process:** The base architecture is specialized, pruned, and fitted to the target. Internal pathways are established that map operational primitives to the target's actual capabilities. This is deterministic and repeatable for a given base + target pair.
- **Output:** A SOMA instance — a neural execution structure ready to inhabit its target and receive intent.

The synthesis process may itself use learning-based techniques internally (architecture search, weight optimization against hardware simulation), but from the outside it behaves like compilation: same input, same output, every time.

### 2.4 Intent as Input, Execution as Output

A SOMA's interface to the human is **intent**. Not code. Not configuration. Not parameters (unless the human chooses to be that specific). The SOMA's responsibility is to:

1. Receive intent through the intent interface.
2. Disambiguate. If intent is ambiguous, the SOMA asks — like a competent colleague would.
3. Plan execution using its knowledge of its own body.
4. Execute directly on its hardware/environment.
5. Return results or effects.

This loop has no intermediate code generation step. The SOMA's internal neuronal language orchestrates hardware directly.

---

## 3. Architecture

### 3.1 Layered Internal Structure

Although a SOMA has no human-readable code, it is not an opaque black box. Internally, it has a layered structure, each layer with a distinct role:

**Layer 1 — Intent Reception and Parsing.**
Receives raw human input (text, audio, neural signal). Produces a structured internal representation of the desired outcome. Handles ambiguity resolution, clarification dialogue, and intent validation.

**Layer 2 — Planning and Decomposition.**
Takes parsed intent and decomposes it into operational sub-goals. This layer understands causality, sequencing, dependencies, and resource requirements. It is aware of the body's capabilities and constraints.

**Layer 3 — Neuronal Execution Core.**
The deepest layer. Maps operational sub-goals to direct hardware/environment actions. This is where the neuronal language lives — an emergent internal representation that encodes operations in terms native to the target body. On bare metal, this means register manipulation, interrupt handling, signal timing. On an OS-hosted body, this means system calls, API invocations, process management.

**Layer 4 — Feedback and Correction.**
Monitors execution outcomes. Compares results against the original intent. If divergence is detected, this layer can retry with different strategies, adjust parameters, or escalate to the human via the intent interface. This is the SOMA's equivalent of error handling — but adaptive rather than predefined.

**Layer 5 — Proprioception and Self-Model.**
A continuous background layer that maintains the SOMA's knowledge of its own state — memory usage, thermal conditions, I/O status, peripheral availability, network connectivity. Informs all other layers.

### 3.2 Deterministic Mode

Certain operations require absolute precision: arithmetic, cryptographic functions, transaction processing, timing-critical control loops. For these, the neuronal execution core enters a **deterministic mode** where:

- Execution follows verified, invariant pathways synthesized during the compilation phase.
- These pathways are functionally equivalent to traditional compiled machine code but are embedded within the neural structure as fixed circuits.
- Deterministic pathways are formally verifiable — synthesis can generate proofs of correctness for critical operations, similar to how compilers can guarantee certain optimizations preserve semantics.
- The SOMA knows which operations require deterministic mode and activates it automatically based on the nature of the task.

This is not a contradiction of the SOMA paradigm. The human brain also has "deterministic" circuits — reflexes, autonomic functions — alongside flexible cognition. A SOMA similarly has hardened pathways for operations that demand exactness.

### 3.3 The Neuronal Language

A SOMA's internal language is not designed by humans. It emerges from the synthesis process as the most efficient way to represent operations on the specific target body. However, it shares structural properties with both neural activation patterns and traditional instruction sets:

- It has **opcodes** — atomic operations the target can perform.
- It has **composition rules** — how atomic operations combine.
- It has **state representations** — how memory, registers, and I/O are modeled internally.
- It has **optimization patterns** — learned or synthesized shortcuts for common operation sequences.

This language is unique per synthesis target. Two SOMA instances on different hardware will have different neuronal languages, just as ARM and x86 have different machine codes. But both can fulfill the same human intent.

---

## 4. Structured Intent Formalism and the Planning Layer

### 4.1 The Two Classes of Intent

Not all human intents are equal in complexity. A critical architectural distinction exists between two classes:

**Class 1 — Operational Intent.** Direct, concrete, single-operation commands. "List files in /tmp." "Send a confirmation email to Maria." "Read the temperature sensor." These map to a single body operation with clear parameters. The neuronal execution core handles these directly — one intent, one opcode, one execution. A small neural network (sub-1M parameters) can classify and extract parameters reliably.

**Class 2 — Creative/Architectural Intent.** Complex, multi-step, design-level commands. "Add a waitlist feature." "Make the booking page show a calendar view." "Add support for recurring appointments." These require understanding existing application state, reasoning about design, decomposing into a sequence of operations, and executing them in order. No single opcode suffices. This is what human developers do — and it is the hard problem.

The neuronal execution core (Layer 3) is designed for Class 1. Attempting to handle Class 2 with the same architecture would require the entire execution core to possess general reasoning capabilities — effectively requiring an AGI-scale model on every device. This is neither practical nor necessary.

### 4.2 The Small Model as Planning Layer

Layer 2 (Planning and Decomposition) for complex intents can be implemented as a small language model (1B–3B parameters) — models like Phi-3, Llama 3.2 1B, or Qwen 2.5 3B. These run locally, on-device, with minimal resources.

The key insight: decomposing complex intent into a sequence of known operations is a **constrained task**, not an open-ended generation task. The small model does not need general intelligence. It needs to understand:
- The SOMA's current capabilities (body manifest).
- The current application state (existing routes, database structures, business rules).
- How to break a high-level request into a sequence of operations from the known vocabulary.

This is a fine-tuning problem, not a scaling problem. A 1B model fine-tuned on decomposition examples for a specific domain (web applications, IoT, industrial control) can reliably produce structured operation sequences.

```
Human: "Add a waitlist feature for when time slots are full"
                    │
        [Small Model — Layer 2]
                    │
                    ▼
Decomposed plan:
  1. CREATE_TABLE waitlist (slot_id, client_name, email, position)
  2. ADD_ROUTE POST /waitlist/join (slot_id, name, email)
  3. ADD_ROUTE GET /waitlist/{slot_id}
  4. ADD_TRIGGER on_slot_cancel → notify_first_waitlist
  5. MODIFY_ROUTE GET /booking/{slot_id} → show waitlist when full
                    │
        [Neuronal Execution Core — Layer 3]
                    │
                    ▼
        [Body: SQL + HTTP + SMTP]
```

The planning model is **part of the SOMA**, not an external dependency. During synthesis, the planning model is fitted to the target body's capabilities — it can only produce operation sequences that the body can execute. This prevents the planning layer from generating plans the execution core cannot fulfill.

### 4.3 Gherkin as Structured Intent Language

Open Question #4 in this paper asks: "Is natural language sufficient as an intent interface, or will high-performance applications require a more structured intent language?" The answer is: for Class 2 intents, a structured formalism dramatically improves reliability. We propose adopting **Gherkin** — the specification language from behavior-driven development (BDD) — as SOMA's structured intent formalism for complex operations.

Gherkin is a human-readable, machine-parseable language for describing system behavior:

```gherkin
Feature: Waitlist
  Scenario: Client joins waitlist when slot is full
    Given a time slot at 2pm with Ana is fully booked
    When a client submits name "Maria" and email "maria@mail.com"
    Then the client is added to the waitlist at position 1
    And the client receives a confirmation email

  Scenario: Client gets notified when slot opens
    Given "Maria" is position 1 on the waitlist for 2pm with Ana
    When the booked client cancels
    Then "Maria" receives a notification email
    And the slot shows as available for "Maria" for 10 minutes
```

Gherkin is not code. It is a specification of behavior in structured natural language. It occupies the exact boundary between human intent and machine-executable specification.

### 4.4 The Three Roles of Gherkin in SOMA

**Role 1 — Intent formalism.**
The small planning model (Layer 2) decomposes natural language into Gherkin scenarios. The human says "add a waitlist feature." The planning model produces the Gherkin specification above. The human can read it, validate it, and modify it before the SOMA executes. This is the SOMA's disambiguation mechanism (Section 2.4) made concrete — the SOMA shows its plan in a format the human can verify.

**Role 2 — Self-verification.**
The same Gherkin that defines a feature also verifies it works. After the SOMA executes the decomposed operations, it runs the Gherkin scenarios against itself as behavioral tests. If the scenarios pass, the feature is confirmed working. If they fail, the feedback layer (Layer 4) retries or escalates to the human. This solves the verification problem from Section 8 — the specification IS the test suite.

**Role 3 — Versioning.**
Section 9 raises the problem of versioning without source code. Gherkin specifications are versionable text documents. They can be diffed, branched, rolled back, and stored in version control. The complete behavior of a SOMA-powered application is defined by its Gherkin specification library plus its synthesis configuration. Both are versioned. The application's history is the history of its specifications, not the history of its code — because there is no code.

### 4.5 The Complete Intent Pipeline

The full pipeline from human intent to execution, handling both intent classes:

```
Human natural language
  "Add a waitlist feature"
           │
  [Layer 1: Intent Reception — classify complexity]
           │
           ├── Class 1 (simple) ──────────────────────┐
           │   "list files in /tmp"                    │
           │                                           ▼
           │                              [Layer 3: Direct execution]
           │                                           │
           ├── Class 2 (complex) ─────────┐            │
           │                              ▼            │
           │                   [Layer 2: Small model   │
           │                    decomposes to Gherkin]  │
           │                              │            │
           │                              ▼            │
           │                   [Human reviews/approves  │
           │                    Gherkin specification]  │
           │                              │            │
           │                              ▼            │
           │                   [Gherkin parsed into     │
           │                    operation sequence]     │
           │                              │            │
           │                              ▼            │
           │                   [Layer 3: Execute each   │
           │                    operation sequentially] │
           │                              │            │
           │                              ▼            │
           │                   [Run Gherkin scenarios   │
           │                    as self-verification]   │
           │                              │            │
           ▼                              ▼            ▼
                        [Body: hardware/OS/DB/network]
```

### 4.6 Why This Is Not "AI-Assisted Development"

A critical distinction: this pipeline does not generate code. The small model generates Gherkin (a behavior specification), not Python or JavaScript. The Gherkin is parsed into operation sequences (opcodes), not source code. The operations are executed by the neuronal execution core, not by a compiler or interpreter. At no point does a programming language appear.

This is also not "low-code" or "no-code" in the current industry sense. Low-code platforms still produce code behind a visual interface. SOMA produces no code at any layer. The Gherkin is a communication format between the planning layer and the execution layer — analogous to how SQL is a communication format between the SOMA and a database. It is a protocol, not a program.

---

## 5. Synthesis Process

### 4.1 Overview

Synthesis transforms a **base architecture** into a **target-specific SOMA instance**. The process is analogous to compilation but operates on neural structures rather than source code.

```
┌─────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│  Base SOMA       │     │  Target Body     │     │  SOMA Instance   │
│  Architecture    │ ──▶ │  Specification   │ ──▶ │  (ready to run)  │
│  (universal)     │     │  (HW + ENV)      │     │  (target-native) │
└─────────────────┘     └──────────────────┘     └──────────────────┘
                              │
                    ┌─────────┴──────────┐
                    │ - CPU architecture  │
                    │ - Memory map        │
                    │ - Peripheral manifest│
                    │ - OS API surface    │
                    │ - Resource limits   │
                    │ - I/O channels      │
                    └────────────────────┘
```

### 4.2 Phases of Synthesis

**Phase 1 — Body Discovery.**
The synthesizer ingests the target body specification. This can be provided as a formal hardware description (device tree, datasheet-derived manifest), an OS API surface definition, or discovered through probing a live system.

**Phase 2 — Architecture Specialization.**
The base neural architecture is pruned and adapted for the target. A SOMA on a microcontroller with 520KB RAM gets a radically smaller, more focused architecture than a SOMA on a server with 256GB. Layers are scaled, pathways are allocated, and the proprioception layer is populated with the body model.

**Phase 3 — Neuronal Language Generation.**
The synthesizer generates the target's neuronal language — the internal operational vocabulary. This is derived from the intersection of "what operations the base architecture can express" and "what operations the target body can physically perform." For bare metal, this grounds out in register operations and timing. For OS-hosted targets, it grounds out in system calls and API patterns.

**Phase 4 — Deterministic Pathway Hardening.**
Critical operations (arithmetic, crypto, timing-critical loops) are identified and synthesized as fixed, verifiable circuits within the neural structure. These pathways are tested and proven correct during synthesis.

**Phase 5 — Integration and Validation.**
The complete SOMA instance is assembled, loaded onto the target (or its simulator), and validated against a suite of intent-execution pairs. This is the SOMA equivalent of compiler test suites.

### 4.3 Synthesis Properties

- **Deterministic.** Same base + same target = same SOMA instance, every time.
- **Incremental.** If the target body changes (new peripheral added, OS updated), re-synthesis can be partial.
- **Portable.** The base architecture is universal. Only synthesis is target-specific.

---

## 6. The Bootstrap Problem

### 5.1 The Paradox

SOMA proposes to replace programming. But the first synthesizer — the system that produces SOMA instances — must itself be built using traditional programming. This is not a flaw; it is a necessary starting condition, identical to the bootstrap problem in compiler history.

The first C compiler was written in assembly. The first assembly was written in machine code. Every new paradigm must be born inside the paradigm it seeks to replace.

### 5.2 The Bootstrap Path

**Stage 0 — Traditional Implementation.**
The first SOMA synthesizer is written in a conventional language (likely Rust, C++, or a combination) using traditional tools. It is a compiler — it takes the base architecture and a target specification and produces a SOMA instance. This synthesizer is itself conventional software: version-controlled, tested, debugged in the traditional way.

**Stage 1 — Self-Hosting.**
A critical milestone: the synthesizer produces a SOMA instance capable of performing synthesis. At this point, the SOMA paradigm can produce itself. This is equivalent to a compiler compiling itself — the moment the new paradigm becomes self-sustaining.

**Stage 2 — Divergence.**
Once self-hosting is achieved, the SOMA-synthesized synthesizer can evolve independently of its traditional-code ancestor. Improvements to the synthesizer are made by instructing the synthesizer-SOMA, not by editing source code.

### 5.3 The Significance of Self-Hosting

Self-hosting is not just a technical milestone — it is the philosophical proof point. If a SOMA can synthesize other SOMAs, the paradigm is complete. It no longer depends on traditional programming for its continued existence. Every computing paradigm that achieved self-hosting (compilers, operating systems, virtual machines) became permanent. This is the threshold SOMA must cross.

---

## 7. The Synthesizer

### 6.1 What Is the Synthesizer?

The synthesizer is the most critical component of the SOMA ecosystem. It is the bridge between the universal base architecture and every possible target body. It occupies the same position in SOMA that the compiler occupies in traditional computing — but its output is a neural execution structure rather than machine code.

### 6.2 Synthesizer Architecture

The synthesizer itself has several subsystems:

**Body Analyzer.**
Ingests the target specification. For bare metal targets, this means parsing hardware datasheets, device trees, and register maps. For OS-hosted targets, this means cataloging available system calls, APIs, libraries, drivers, and resource limits. For live systems, this may involve active probing — testing what the target can actually do.

**Architecture Mapper.**
Maps the base neural architecture onto the target's capabilities. This is the core intellectual challenge: how to transform a universal, abstract neural structure into one that can directly orchestrate specific hardware. The mapper must decide which layers to scale, which pathways to create, and how to ground abstract operations into physical ones.

**Pathway Compiler.**
Generates deterministic pathways for critical operations. This subsystem is closest to a traditional compiler — it produces verified, fixed execution circuits for arithmetic, cryptography, and other exactness-requiring operations.

**Validator.**
Runs the synthesized SOMA instance against a test suite of intent-execution pairs in a simulated or sandboxed environment before releasing it for deployment.

### 6.3 The Synthesizer as a SOMA

After self-hosting (Section 6), the synthesizer becomes a SOMA instance whose body is the synthesis environment — its I/O is "receive base architecture + target spec" and its output is "produce SOMA instance." It knows its own computational resources, can optimize its own synthesis strategies, and can adapt to new target types through runtime adaptation. The synthesizer-SOMA's intent interface accepts requests like: "Synthesize a SOMA for this ESP32 board" or "Re-synthesize the living room sensor SOMA with updated firmware support."

---

## 8. Verification and Trust

### 7.1 The Problem of Opaque Execution

Without source code, traditional code review is impossible. A SOMA's internal neuronal language is not intended to be human-readable. This raises the question: how do you trust it?

### 7.2 Test-Driven Verification

The primary verification mechanism is **behavioral testing** — the same approach used to verify any system whose internals are opaque (hardware chips, biological systems, black-box certified systems).

- The user or a verification framework provides **intent-output pairs**: "given this intent, this output (or behavior) must result."
- The SOMA is tested against these pairs after synthesis and periodically during operation.
- Deterministic pathways are additionally subject to **formal verification** during synthesis.

This is not fundamentally different from how compiled binaries are trusted today — nobody reads the machine code; they test the behavior.

### 7.3 Introspection

A SOMA maintains a **self-model** (proprioception layer) that can be queried. A human can ask:
- "What are you doing right now?"
- "Why did you produce that result?"
- "What resources are you using?"

The SOMA can explain its actions in natural language through the intent interface, providing transparency without requiring the human to read internal representations.

---

## 9. Versioning and Rollback

### 8.1 The Problem

Traditional software uses version control (git) to track changes in source code. No source code means no diffs, no branches, no pull requests. How do you manage the lifecycle of a SOMA?

### 8.2 Synthesis-Based Versioning

A SOMA instance is fully determined by two inputs: the **base architecture version** and the **target body specification**. Therefore, versioning operates on these inputs, not the output:

- **Base architecture** is version-controlled traditionally (it is, at least initially, produced by conventional tools).
- **Target body specifications** are version-controlled documents: hardware manifests, API surface definitions, resource constraints.
- Any SOMA instance can be **exactly reproduced** by re-running synthesis with the same base + target pair (synthesis is deterministic).

This is analogous to how Docker images are versioned by their Dockerfile, not by the binary contents of the image.

### 8.3 Rollback

If a new synthesis produces a SOMA that behaves incorrectly:

1. Revert to the previous base architecture version and/or target specification.
2. Re-synthesize. The previous SOMA instance is exactly reproduced.
3. Deploy the rolled-back instance.

Because synthesis is deterministic, rollback is guaranteed to produce the exact previous SOMA.

### 8.4 Runtime Adaptation Snapshots

Runtime adaptation (Section 11.3) modifies a SOMA's behavior within bounded limits. To preserve rollback capability, the SOMA periodically creates **adaptation snapshots** — serialized states of its adapted pathways. These snapshots can be restored, shared with other SOMA instances, or discarded during rollback.

---

## 10. Multi-SOMA Communication — The Soma Network

### 9.1 The Need for Composition

Real-world systems involve multiple devices: a sensor on an ESP32, a gateway on a Raspberry Pi, a backend on a cloud server, a UI on a phone. Each hosts its own SOMA instance. They must communicate.

### 9.2 Synaptic Protocol

We propose **Synaptic Protocol** — a communication model inspired by biological neural signaling and proven distributed systems principles.

In biological systems, neurons communicate via synapses: a presynaptic neuron releases a signal, the synapse transmits it (with potential modulation), and the postsynaptic neuron receives and integrates it. This is simple, robust, and scales to billions of connections.

The Synaptic Protocol applies this model to SOMA networks:

- **Signal.** The fundamental unit of inter-SOMA communication. A signal carries intent, data, or feedback — encoded in a compact, self-describing format. Analogous to a neurotransmitter packet.
- **Synapse.** A connection between two SOMA instances. Synapses can be direct (wired, Bluetooth, UART) or routed (TCP/IP, mesh network). The physical medium is abstracted — a SOMA knows it has a synapse to another SOMA, not the transport details.
- **Transmission.** Signals are asynchronous by default, like biological synapses. Synchronous (request-response) mode is available when a SOMA needs to wait for a result.
- **Modulation.** Synapses can modulate signals — compress, filter, prioritize, encrypt. This is configured during synthesis based on the network topology and security requirements.
- **Discovery.** SOMA instances discover each other through a **chemical gradient** model — broadcasting presence signals that propagate through the network with diminishing strength, allowing nearby SOMAs to find each other organically. This is inspired by how biological cells find their neighbors through chemical signaling, and is technically similar to mDNS/service discovery but with adaptive, priority-aware propagation.

### 9.3 Composition Patterns

- **Delegation.** A SOMA receives intent it cannot fully fulfill (insufficient resources, missing I/O). It delegates sub-tasks to other SOMAs in its network via synaptic signals.
- **Hierarchy.** A more capable SOMA (e.g., cloud-hosted) can orchestrate multiple smaller SOMAs (e.g., embedded sensors), forming a nervous-system-like hierarchy.
- **Collective.** Peer SOMAs can form a collective to jointly handle tasks that exceed any individual's capacity — similar to a neural ensemble or a distributed computing cluster, but with synaptic coordination rather than programmatic orchestration.

---

## 11. Runtime Behavior

### 10.1 The Execution Loop

A running SOMA operates in a continuous loop:

```
┌──────────────┐
│  IDLE /       │◀──────────────────────────────┐
│  PROPRIOCEIVE │                               │
└──────┬───────┘                               │
       │ intent received                        │
       ▼                                        │
┌──────────────┐                               │
│  PARSE       │                               │
│  INTENT      │                               │
└──────┬───────┘                               │
       │                                        │
       ▼                                        │
┌──────────────┐    ambiguous    ┌────────────┐│
│  DISAMBIGUATE │───────────────▶│ ASK HUMAN  ││
│              │                └─────┬──────┘│
└──────┬───────┘                      │        │
       │ clear                        │ answer │
       ▼                              ▼        │
┌──────────────┐                               │
│  PLAN        │                               │
│              │                               │
└──────┬───────┘                               │
       │                                        │
       ▼                                        │
┌──────────────┐                               │
│  EXECUTE     │                               │
│  (neuronal)  │                               │
└──────┬───────┘                               │
       │                                        │
       ▼                                        │
┌──────────────┐    mismatch    ┌────────────┐│
│  VERIFY      │───────────────▶│ RETRY /    ││
│  RESULT      │                │ ADAPT      │┘
└──────┬───────┘                └────────────┘
       │ match
       ▼
┌──────────────┐
│  RETURN      │
│  RESULT      │
└──────┬───────┘
       │
       ▼
       ▲ (back to idle)
```

### 10.2 Adaptive Error Handling

Traditional software crashes or throws exceptions. A SOMA handles failure differently:

- **Retry with variation.** If an operation fails, the SOMA tries alternative execution strategies — different pathways through its neuronal language to achieve the same goal.
- **Degrade gracefully.** If a resource becomes unavailable, the SOMA adapts its plan to work within reduced capabilities and informs the human.
- **Learn from failure.** Runtime failures feed back into the SOMA's feedback layer, allowing it to improve its strategies over time without re-synthesis. This is bounded, local adaptation — not open-ended learning.
- **Escalate.** If the SOMA cannot resolve a failure, it reports to the human through the intent interface with an explanation of what went wrong and what it tried.

### 10.3 Evolution and Adaptation

A SOMA has two modes of change:

- **Synthesis-time (base).** The core structure, deterministic pathways, and body model are established during synthesis. This is the immutable foundation.
- **Runtime (adaptation).** Within defined boundaries, the SOMA can optimize execution strategies, cache frequently-used patterns, and adapt to changing conditions (new devices on the network, varying resource availability, updated OS APIs). This is analogous to neuroplasticity — the structure remains, but connections strengthen or weaken with use.

Re-synthesis is required for fundamental changes: new hardware, major OS updates, expanded capabilities.

---

## 12. Real-Time Guarantees

### 11.1 The Challenge

Embedded and industrial applications demand **hard real-time** guarantees: a motor control loop must execute within 10 microseconds, every time. A cardiac pacemaker signal must fire at precisely the right moment. "Adaptive retry" is unacceptable in these contexts. The result must be correct and on time, or people die.

### 11.2 Real-Time Execution in SOMA

Real-time guarantees are provided through the **deterministic mode** (Section 3.2) with additional timing constraints:

**Timing-Bound Pathways.**
During synthesis, the synthesizer identifies operations that require real-time guarantees (from the target body specification, which includes timing requirements). For these operations, it generates **timing-bound deterministic pathways** — fixed execution circuits that are not only functionally correct but have verified worst-case execution times (WCET).

This is analogous to how real-time operating systems (RTOS) provide timing guarantees for critical tasks — but the guarantee is embedded in the neural structure itself, not enforced by a scheduler.

**Priority Layers.**
The neuronal execution core has priority levels. Timing-critical operations preempt all other execution, including intent parsing and adaptation. This is equivalent to hardware interrupt priority but implemented neurally.

**Isolation.**
Real-time pathways are isolated from adaptive pathways. No amount of runtime adaptation can modify or interfere with a timing-bound pathway. These circuits are, in effect, hardwired after synthesis — as immutable as a hardware timer peripheral.

### 11.3 Verification of Real-Time Properties

Timing-bound pathways are verified during synthesis using **static timing analysis** — the same family of techniques used to verify real-time properties in safety-critical embedded software (DO-178C in avionics, IEC 62304 in medical devices). The synthesizer proves that WCET is within the required bound for the specific target hardware before the SOMA instance is released.

---

## 13. Energy and Power

### 12.1 The Challenge

A SOMA on an ESP32 powered by a coin cell battery has a radically different energy budget than a SOMA on a plugged-in server. Neural execution is computationally expensive compared to simple compiled instruction sequences. If a SOMA consumes 10x the energy of compiled C for the same task, it is not viable for embedded/IoT applications.

### 12.2 Energy-Aware Synthesis

The target body specification includes **power constraints**: battery capacity, maximum sustained power draw, thermal limits. The synthesizer uses these constraints to shape the SOMA instance:

- **Architecture scaling.** A power-constrained target gets a smaller, sparser neural architecture — fewer pathways, simpler planning, more reliance on deterministic pathways (which are energy-cheap, equivalent to compiled code).
- **Activation efficiency.** The neuronal execution core is synthesized to minimize active pathways per operation. For simple, repetitive tasks (sensor reading, LED control), only a tiny fraction of the neural structure activates — approaching the energy profile of traditional compiled code.
- **Sleep integration.** Proprioception includes power state management. A SOMA on a battery target knows how to put its body into low-power modes, wake on relevant interrupts, and minimize active time — not because it was programmed to, but because power management is part of its body knowledge.

### 12.3 The Efficiency Spectrum

Not every task needs the full neural execution stack. A SOMA intelligently allocates resources:

- **Simple, repetitive tasks** (toggle GPIO, read sensor) → deterministic pathways, near-zero neural overhead, energy cost approaches compiled code.
- **Moderate tasks** (format data, communicate, schedule) → partial neural execution, moderate energy cost.
- **Complex, novel tasks** (interpret ambiguous intent, plan multi-step operations, adapt to failure) → full neural execution, higher energy cost.

The SOMA itself decides where each task falls. This is proprioception applied to energy: the SOMA knows what it costs to think and chooses how hard to think.

---

## 14. Performance Expectations

### 13.1 Honest Assessment

Will a SOMA be faster than compiled C? Sometimes. Will it be slower? Sometimes. The honest answer is that SOMA trades a different performance profile, not a universally better one.

### 13.2 Where SOMA Is Likely Slower

- **Tight loops and raw computation.** A `for` loop adding numbers will always be faster as compiled machine code than as a neural execution pathway. Deterministic mode narrows this gap but cannot eliminate it entirely due to the overhead of the neural substrate.
- **Latency-sensitive single operations.** The intent-parsing and planning layers add latency before execution begins. For a single, simple operation, this overhead dominates.

### 13.3 Where SOMA Is Likely Faster

- **End-to-end task completion.** Traditional development: write code → debug → compile → deploy → discover bug → fix → redeploy. SOMA: state intent → done. The total time from human intent to working result is potentially orders of magnitude faster.
- **Adaptive workloads.** Tasks that require runtime decision-making (failover, load balancing, protocol negotiation) are handled natively by the neural execution core, without the overhead of programmed conditional logic.
- **Multi-device coordination.** Synaptic Protocol eliminates the need for explicit API design, serialization, protocol implementation. Coordination that takes weeks to program traditionally happens organically.

### 13.4 The Right Comparison

SOMA should not be benchmarked against compiled code for raw instruction throughput. It should be benchmarked against the **full lifecycle**: human intent → working system. By that measure, the performance gap favors SOMA overwhelmingly.

---

## 15. Security Model

### 14.1 Threat Landscape

A SOMA directly controls hardware. A compromised SOMA is as dangerous as a compromised compiler or firmware — potentially catastrophic. This is not a new class of risk; it is the same risk that exists in any system where software has hardware access.

### 14.2 Security Architecture

**Synthesis-Time Security.**
- The synthesizer is the root of trust. A compromised synthesizer produces compromised SOMAs, exactly as a compromised compiler produces compromised binaries (cf. Ken Thompson's "Reflections on Trusting Trust").
- Synthesis is deterministic: a given base + target must always produce the same SOMA instance. This allows third-party verification.
- Deterministic pathways for security-critical operations (crypto, authentication) are formally verified during synthesis.

**Runtime Security.**
- **Capability boundaries.** A SOMA knows what it is allowed to do, not just what it can do. Permissions are part of the body specification and are enforced at the neuronal execution core level.
- **Failure containment.** If the feedback layer detects anomalous behavior (unexpected hardware access patterns, resource usage outside normal bounds), it can halt execution and alert the human — similar to a hardware watchdog timer but neurally implemented.
- **Network trust.** Synaptic Protocol connections between SOMAs use mutual authentication. A SOMA will not accept signals from unverified peers.

### 14.3 The Compiler Analogy

Users trust compilers today without reading the machine code they produce. The same trust model applies to SOMA:
- The synthesizer is open, auditable, and deterministically reproducible.
- SOMA instances are validated behaviorally.
- Security-critical pathways are formally verified.
- Runtime monitoring catches anomalies.

---

## 16. Neuromorphic Hardware Affinity

### 15.1 The Natural Substrate

SOMA's architecture — a neural execution structure directly orchestrating hardware — is a natural fit for **neuromorphic processors**: chips designed to execute neural computations natively, rather than simulating them on von Neumann architectures.

### 15.2 Existing Neuromorphic Platforms

**Intel Loihi (1 & 2).**
A many-core neuromorphic research chip with on-chip learning. Loihi natively implements spiking neural networks — networks where neurons communicate through discrete spikes (events) rather than continuous values. A SOMA synthesized onto Loihi could use spiking patterns as its neuronal language, with hardware neurons directly implementing execution pathways. Loihi's on-chip learning capabilities map naturally to SOMA's runtime adaptation.

**IBM TrueNorth.**
A million-neuron, 256-million-synapse chip designed for ultra-low-power neural computation. TrueNorth's fixed architecture and extreme energy efficiency make it an ideal target for embedded SOMA instances — particularly for sensor processing and pattern recognition tasks where the SOMA needs to operate on minimal power.

**BrainChip Akida.**
A commercial neuromorphic processor targeting edge AI. Akida supports on-chip learning and event-driven processing. A SOMA on Akida could handle real-time sensor fusion and intent processing directly in neuromorphic silicon.

**SpiNNaker (University of Manchester).**
A massively parallel architecture designed to simulate large-scale spiking neural networks in real time. SpiNNaker's communication infrastructure — where cores communicate through small packets routed across a mesh network — is structurally similar to the Synaptic Protocol, making it a natural platform for multi-core SOMA instances.

### 15.3 Why Neuromorphic Matters for SOMA

On conventional hardware (CPU/GPU), a SOMA's neural execution must be simulated — the processor fetches instructions that simulate neural operations. This adds overhead. On neuromorphic hardware, neural execution is **native** — the silicon itself performs neural computation directly, the same way a GPU natively performs matrix operations.

The implication: SOMA on neuromorphic hardware approaches the performance and energy efficiency of traditional compiled code on conventional hardware, while retaining the flexibility and adaptivity of neural execution. This is the long-term hardware trajectory that makes SOMA not just viable but potentially superior.

### 15.4 Hybrid Targets

In the near term, most SOMA instances will inhabit conventional hardware (x86, ARM, RISC-V) with neuromorphic accelerators where available. Synthesis must handle **hybrid targets**: conventional cores for deterministic pathways, neuromorphic accelerators for adaptive neural execution. This is analogous to how modern software uses CPUs for logic and GPUs for parallel computation — but with a SOMA orchestrating the split internally rather than a programmer making explicit decisions.

---

## 17. Concrete Use Cases

### 16.1 Use Case: Smart Agriculture Sensor Network

**Traditional approach:**
A developer writes C firmware for soil moisture sensors (ESP32), a Python backend for a Raspberry Pi gateway, a REST API in Node.js for the cloud server, and a React Native app for the farmer's phone. Four codebases, four languages, API contracts between them, deployment pipelines, OTA update mechanisms. Months of work. Any change in sensor hardware requires firmware rewrites.

**SOMA approach:**
Each device gets a synthesized SOMA instance. The farmer says to the phone SOMA: "Alert me when any field drops below 30% moisture." The phone SOMA signals the cloud SOMA, which signals the gateway SOMA, which configures the sensor SOMAs. The sensor SOMAs know their GPIO (moisture probe pin), their power constraints (solar + battery), and their synapse to the gateway. They read, transmit, sleep. If a sensor is replaced with a different model (different ADC, different pin), re-synthesis produces a new SOMA for that body in minutes. No code rewritten.

### 16.2 Use Case: Personal Home Automation

**Traditional approach:**
Buy a smart home hub. Install apps. Configure automations through clunky UIs. Write YAML for Home Assistant. Debug Z-Wave/Zigbee pairing. Script complex automations in Python or Node-RED. Each new device type requires integration effort.

**SOMA approach:**
Each device in the home has a SOMA. They discover each other through chemical gradient signaling. The homeowner says: "When I leave for work, turn off the lights, lock the doors, and lower the heat." The home SOMAs coordinate through Synaptic Protocol. Adding a new device means synthesizing a SOMA for it; it joins the network and introduces its capabilities. No configuration. No integration code. No app.

### 16.3 Use Case: Industrial Motor Controller

**Traditional approach:**
Embedded engineer writes a PID control loop in C, tunes parameters through extensive testing, implements safety shutoffs, handles edge cases for overtemperature, overcurrent, stall conditions. Each motor variant requires parameter re-tuning or code changes. Certification requires extensive documentation of every code path.

**SOMA approach:**
The SOMA is synthesized onto the motor controller board. It knows its PWM outputs, current sense ADC, temperature sensor, encoder input. Its deterministic pathways handle the real-time control loop with verified WCET. Its adaptive layer handles tuning — it adjusts control parameters based on actual motor behavior, like a human operator who learns the feel of a specific motor. The human says: "Run this motor at 1500 RPM, don't exceed 80°C." The SOMA does it, adapts to load changes, and reports anomalies. Replacing the motor with a different model? The SOMA adapts at runtime or gets re-synthesized. Certification tests behavior, not code.

### 16.4 Use Case: Rapid Prototyping

**Traditional approach:**
Startup wants to prototype an IoT product. Hire embedded developer, backend developer, mobile developer. Three months minimum to a working demo. Any pivot requires significant rework.

**SOMA approach:**
Synthesize SOMAs onto prototype hardware. Describe the desired product behavior in natural language. Iterate by talking to the SOMAs: "Actually, make it send alerts every hour instead of on-change." "Add a battery level indicator to the phone." Changes take minutes, not sprints. The prototype is the product.

---

## 18. Comparison with Existing Paradigms

| Aspect | Traditional Development | AI-Assisted Development | SOMA |
|---|---|---|---|
| **Intermediate artifact** | Source code | AI-generated source code | None |
| **Human reads/writes code** | Yes | Yes (reviews, fixes) | No |
| **Hardware awareness** | Via compiler/OS abstraction | None (model unaware) | Intimate (proprioception) |
| **Error handling** | Predefined (try/catch) | Predefined | Adaptive (retry, vary, degrade) |
| **Determinism** | Full | Partial (AI generation is stochastic) | Hybrid (deterministic mode for critical paths) |
| **Multi-device** | Protocols, APIs, middleware | Same | Synaptic Protocol (organic) |
| **Developer role** | Write code | Write prompts, review code | Define intent; improve synthesizer |
| **Deployment** | Compile, package, deploy | Same | Synthesize to target |
| **Real-time capable** | Yes (with RTOS) | No | Yes (timing-bound pathways) |
| **Energy profile** | Optimal for fixed tasks | Same as traditional | Adaptive (scales with task complexity) |
| **Multi-device setup** | Weeks/months | Weeks | Minutes (Synaptic discovery) |
| **Hardware change** | Rewrite/recompile | Regenerate/recompile | Re-synthesize |

---

## 19. Coexistence and Migration Path

### 18.1 Reality Check

SOMA will not replace all software overnight. Billions of lines of existing code run the world. The transition must be gradual, and SOMA must coexist with traditional software during that transition.

### 18.2 Coexistence Models

**Model A — SOMA as Peripheral.**
A SOMA instance runs alongside traditional software on the same host. The traditional application handles its core logic; the SOMA handles specific tasks that benefit from adaptive, intent-driven execution (user interaction, device management, error recovery). They communicate through standard OS mechanisms (IPC, shared memory, sockets).

**Model B — SOMA as Orchestrator.**
A SOMA instance manages and coordinates existing software systems. Rather than replacing a legacy backend, the SOMA learns to invoke it — treating the legacy system as part of its body. The SOMA's proprioception includes "I have a PostgreSQL database accessible on port 5432" and "I have a REST API at this endpoint." Intent from the human is translated into orchestration of these existing components.

**Model C — Incremental Replacement.**
Legacy systems are progressively replaced, component by component. A microservice is removed and its function is absorbed into the SOMA. The external interface remains the same; the internal implementation shifts from code to neural execution. Other components don't know the difference.

### 18.3 The Migration Incentive

Adoption will be driven by economics:
- Reducing development time from months to minutes for new features.
- Eliminating entire categories of bugs (integration errors, API contract violations, deployment failures).
- Enabling hardware changes without software rewrites.
- Reducing the need for specialized developers for each layer of the stack.

Organizations won't adopt SOMA for ideology. They'll adopt it because it's cheaper and faster. The migration path must make the first step easy and the benefits immediate.

---

## 20. Research Roadmap

### Phase 1 — Theoretical Foundation (Months 0–6)
- Formalize the base neural architecture specification.
- Define the synthesis process mathematically.
- Define the neuronal language emergence model.
- Publish this whitepaper and solicit peer feedback.
- Survey existing neuromorphic hardware for synthesis target suitability.

### Phase 2 — Minimal Proof of Concept (Months 6–18)
- Build a SOMA synthesizer for a constrained target (ESP32 or equivalent microcontroller, simulated).
- Demonstrate: human says "blink LED every 2 seconds" → SOMA directly drives GPIO, no code generated.
- Demonstrate deterministic mode for basic arithmetic operations.
- Demonstrate proprioception: SOMA reports its own resource usage and capabilities.
- Benchmark energy consumption against equivalent compiled C implementation.

### Phase 3 — OS-Hosted SOMA (Months 18–30)
- Synthesize a SOMA onto a macOS/Linux environment.
- Demonstrate: SOMA uses OS APIs (file I/O, networking, process management) to fulfill intent.
- Demonstrate adaptive error handling: SOMA retries with alternative strategies on failure.
- Demonstrate coexistence Model A: SOMA running alongside a traditional application.
- Demonstrate coexistence Model B: SOMA orchestrating a legacy REST API.

### Phase 4 — Soma Network (Months 30–42)
- Implement Synaptic Protocol.
- Demonstrate multi-SOMA coordination: ESP32 sensor SOMA + cloud processing SOMA + phone UI SOMA working together.
- Demonstrate delegation, hierarchy, and collective composition.
- Implement at least one concrete use case end-to-end (e.g., smart agriculture).

### Phase 5 — Self-Hosting (Months 42–54)
- Achieve bootstrap Stage 1: a SOMA that can synthesize other SOMAs.
- Validate self-hosted synthesis produces identical output to traditional synthesizer.
- Begin synthesizer development through intent rather than code.

### Phase 6 — Neuromorphic Targets (Months 48–60)
- Synthesize SOMA onto Intel Loihi or equivalent neuromorphic hardware.
- Benchmark neural-native execution against simulated execution on conventional hardware.
- Demonstrate hybrid targets (conventional + neuromorphic).

### Phase 7 — Open Research (Months 60+)
- Formal verification tooling for deterministic and timing-bound pathways.
- Runtime adaptation boundaries and safety proofs.
- Intent interface expansion (voice, neural).
- Community-contributed target body specifications.
- Real-time certification pathway (DO-178C, IEC 62304 compatibility).
- Performance benchmarking against traditional compiled software at scale.

---

## 21. Ethical and Societal Impact

### 20.1 Developer Displacement

SOMA, if successful, renders traditional software development obsolete. This affects millions of professionals worldwide. The ethical responsibility of this project includes:

- **Honest communication.** Not claiming SOMA will "augment" developers when the long-term trajectory is replacement. The paradigm eliminates the need for humans to write, review, debug, and maintain code.
- **Transition timeline.** Full displacement, if it happens, is decades away. During the transition, developers evolve into SOMA architects (designing base architectures), synthesis engineers (improving the synthesizer), verification specialists (testing SOMA behavior), and intent designers (crafting effective human-SOMA interaction patterns).
- **Economic preparation.** The broader economic impact of eliminating an entire professional class must be addressed at a policy level, not just a technical one. This project should engage with economists and policymakers early.

### 20.2 Concentration of Power

If a single entity controls the synthesizer and base architecture, they control all SOMA instances. This is an unacceptable concentration of power — worse than any current platform monopoly because SOMA would control hardware directly.

Mitigation:
- The synthesizer must be open source from day one.
- The base architecture specification must be an open standard.
- Multiple independent synthesizer implementations must be encouraged.
- Deterministic synthesis enables independent verification: anyone can check that a synthesizer produces the expected SOMA for a given input.

### 20.3 Autonomy and Control

A SOMA directly controls hardware and adapts at runtime. This raises questions:

- **Who is responsible** when a SOMA's adaptive behavior causes harm? The synthesizer creators? The intent provider? The SOMA itself?
- **Can a SOMA refuse intent?** Should it? If a human instructs a SOMA to perform a harmful action, the capability boundary system (Section 15.2) provides a mechanism for refusal — but who defines the boundaries?
- **Adaptation drift.** If runtime adaptation changes a SOMA's behavior beyond what synthesis intended, at what point has the SOMA become something no one authorized?

These are not solved problems. They are active ethical questions that must be addressed as the technology develops, not after deployment.

### 20.4 Access and Equity

If SOMA delivers on its promise — anyone can create functional software by stating intent — it democratizes computing power in an unprecedented way. A farmer in a rural area, without coding skills, could configure their own automation system. A small business owner could build custom tools by describing what they need. This potential for equalization is one of SOMA's strongest ethical arguments.

However, if synthesis requires expensive infrastructure or proprietary base architectures, this democratization fails. Open access to the synthesizer and base architecture is not just an ideological preference — it is an ethical requirement.

---

## 22. Open Questions

The following remain unsolved and are presented as challenges for the research community:

1. **Synthesis convergence.** Can synthesis always produce a valid SOMA for any base + target pair, or are there target bodies too constrained for the base architecture?
2. **Neuronal language interpretability.** Should the neuronal language be introspectable by researchers (for debugging synthesis), even if not human-readable in the programming sense?
3. **Adaptation boundaries.** How far can runtime adaptation go before it constitutes unsafe drift from the synthesized base? Where is the line?
4. **Intent formalism.** Is natural language sufficient as an intent interface, or will high-performance/safety-critical applications require a more structured intent language?
5. **Synthesis performance.** Can synthesis be fast enough to be practical? Minutes? Hours? Days?
6. **Economic model.** Who builds and maintains the base architecture? The synthesizer? Target body specifications? What is the open-source model for SOMA?
7. **Liability framework.** When a SOMA causes harm through adaptive behavior, who is legally responsible?
8. **Neuromorphic co-design.** Should future neuromorphic hardware be designed specifically as SOMA substrates, co-evolving silicon and architecture?
9. **Formal equivalence.** Can it be proven that a SOMA instance is functionally equivalent to a traditional compiled program for a given specification? Is this even the right question?
10. **The consciousness question.** As SOMAs become more complex, adaptive, and self-aware (proprioception), at what point — if ever — do ethical obligations toward the SOMA itself arise?

---

## 23. Conclusion

The SOMA paradigm proposes a fundamental shift: instead of writing programs that run on computers, we synthesize computational organisms that **are** the computation. The model is the program. The hardware is the body. Human intent is the only input. This is not an incremental improvement to software development — it is a replacement of the paradigm itself.

The path from here to realization is long and requires contributions from neuromorphic computing, compiler theory, hardware design, neural architecture research, distributed systems, ethics, and public policy. But the direction is clear: the era of humans writing instructions for machines is ending. The era of machines understanding intent and acting directly is beginning.

SOMA is a first step toward formalizing that future.

---

## References

- Karpathy, A. (2017). "Software 2.0." Medium.
- Thompson, K. (1984). "Reflections on Trusting Trust." Communications of the ACM.
- Mead, C. (1990). "Neuromorphic Electronic Systems." Proceedings of the IEEE.
- Davies, M. et al. (2018). "Loihi: A Neuromorphic Manycore Processor with On-Chip Learning." IEEE Micro.
- Esser, S. et al. (2016). "Convolutional Networks for Fast, Energy-Efficient Neuromorphic Computing." PNAS.
- Hennessy, J. & Patterson, D. (2019). "A New Golden Age for Computer Architecture." Communications of the ACM.
- Furber, S. et al. (2014). "The SpiNNaker Project." Proceedings of the IEEE.
- Merolla, P. et al. (2014). "A Million Spiking-Neuron Integrated Circuit with a Scalable Communication Network and Interface." Science.
- Lee, E.A. (2008). "Cyber Physical Systems: Design Challenges." ISORC.
- Amodei, D. et al. (2016). "Concrete Problems in AI Safety." arXiv.

---

*This document is a living draft. Contributions, challenges, and criticism are invited.*