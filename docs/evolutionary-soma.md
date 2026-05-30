# Evolutionary SOMA — The Missing Keystone Is Mutation

> **Status (built & proven).** The keystone is no longer hypothetical. Mutation
> operators (`soma-next/src/memory/mutation.rs`) and the breeding loop
> (`soma-next/src/runtime/world_state.rs`, opt-in via `mutation_enabled`) are
> implemented and unit-tested. Single-node adaptation is proven end-to-end on real
> port execution in `soma-project-evolution` (5 phases, including failure-directed
> rescue). Horizontal gene transfer is proven over real TCP in `soma-project-swarm`
> (3 phases) using the production `RemoteRoutineBroadcaster`. Failure-directed
> ("guided") mutation — rescuing a routine before death by mutating away from the
> skill that failed (`guided_mutate`) — is built and proven; full *brain*-directed
> (Lamarckian) mutation, where the LLM proposes the repair, remains future. The
> sections below keep the original discovery framing; the **Build status** table at
> the end is the source of truth for what is done versus still open.

## The thesis

SOMA already has four of the five ingredients of open-ended Darwinian evolution. It inherits, it selects, it computes fitness from real-world consequences, and it can transfer genes across a network of peers. The one missing piece is **variation by mutation**. Add a mutation operator and a breeding loop, and a system that today only *learns from experience* becomes one that *evolves* — an embodied, LLM-directed, distributed evolutionary organism whose fitness function is reality itself, mediated through ports.

This is not a moonshot bolted onto SOMA. It is the keystone of an arch that is already built. This document records the research: what exists, what is missing, what the keystone unlocks, how to prove it, and how to contain it.

The unit of evolution is the **routine** — SOMA's compiled, heritable, executable program (`types/routine.rs:108`). A routine is a genotype: it has match-conditions (its niche), compiled steps (its phenotype), a confidence (its current fitness), a version, and a free-energy-derived model evidence. Routines are data. Evolution on data is cheap.

## SOMA is already 4/5 of a Darwinian engine

Every row below is verified against the code, not asserted from theory.

| Ingredient of Darwinian evolution | Status | Where |
|---|---|---|
| **Genotype** — a heritable, executable program | ✅ proven | `Routine` struct: match-conditions, compiled_steps, confidence, version, model_evidence (`types/routine.rs:108`) |
| **Heredity / replication** — genes persist and are inherited at birth | ✅ proven | Routines written to disk after every change (`persistence.rs:288`); reloaded at boot, packs ship pre-wired routines (`bootstrap.rs:351`) |
| **A fitness scalar** — a number selection can act on | ✅ proven | `model_evidence = -delta_f` — negative free energy from Bayesian Model Reduction (`routines.rs:321`) |
| **Selection-against** — unfit genes die | ✅ proven | Confidence decays 0.7× per consecutive failure; routine invalidated below 0.3 (`world_state.rs:169`) |
| **Horizontal gene transfer** — genes move between organisms over a network | ✅ proven | `TransportMessage::TransferRoutine`, received and stored as `RoutineOrigin::PeerTransferred` (`transport.rs:53`, `transport.rs:871`) |
| **Variation by mutation** — novel genes appear without being experienced or authored | ❌ **missing** | No mutation/crossover/duplication operator exists. New routines come *only* from episode compilation or human authoring. |
| **Reproduction-for** — fit genes breed proportional to fitness | ❌ **missing** | Confidence only ever decreases. Selection today is **purifying-only**: it kills, it never breeds. |

The headline: SOMA's evolutionary machinery is mostly present and *unrecognized as such*. The reactive monitor (`world_state.rs`) is already a selection loop — it just only runs the death half. `TransferRoutine` already makes this a **population**, not a lone organism. The genome is already clean, serializable data. What's absent is the single most generative force in biology: heritable variation.

## The keystone: a mutation module + a breeding loop

The genome is JSON, so mutation is a set of JSON transforms. It belongs in the **runtime**, not a port: mutation touches nothing external (it produces no `PortCallRecord`), and it passes the `body != brain` test — the operators are identical whether the domain is coding or a kitchen, so they are domain-agnostic runtime logic, living next to routine compilation in `memory/`. The *intelligence* of a directed mutation comes from the brain via `invoke`; the *mechanics* stay in the body.

### Blind operators (cheap, in the runtime) — [done — `memory/mutation.rs`]

- **Point mutation** — swap a `skill_id` for a sibling skill; perturb an `input_override` value.
- **Insertion / deletion** — add or remove a step in `compiled_steps`.
- **Niche mutation** — broaden or narrow a `match_condition` (`types/routine.rs:112`), moving the routine to a different region of world-state space.
- **Gene duplication + divergence** — clone a high-confidence routine, then mutate the copy. Biology's primary source of *new* function: the original keeps doing its job while the duplicate explores. The `version` field already distinguishes lineage generations.
- **Crossover (recombination)** — first half of routine A's steps + second half of routine B's, yielding offspring that carries structure from both parents.

### Directed mutation (smart, via the brain) — [build, brain proven]

Hand the brain a routine plus its failure episodes; it returns a *reasoned* variant ("this fails on large repos → insert `git.gc` before `diff`"). This is **Lamarckian variation under Darwinian selection**: directed proposals, but kept only if they survive real consequences through ports. It is the bridge that makes embodied evolution tractable — see below.

### The breeding loop — [done — `world_state.rs` monitor success branch]

The death half of selection already lived in `world_state.rs`. The symmetric birth half now lives in the same monitor's success branch: confidence is reinforced on success (capped at 0.95, mirroring the failure decay), and a routine that sustains success — high confidence, under a population cap — spawns mutated offspring that compete for its niche. Reproduction proportional to fitness + the purifying selection already present = a complete selection loop. Gated by the `mutation_enabled` config flag (off by default, like the reactive monitor itself), so the behavior is opt-in.

## What the keystone unlocks

Each leap is tagged by how much of it is proven today versus build.

### 1. Self-authoring DNA — the genome stops being hand-written

Today the loop is: deliberation → episode → compiled routine (proven pipeline: `memory/sequence_mining.rs` → `schemas.rs` → `routines.rs`). That is already the **Baldwin effect** — a behavior the cortex *learned* becomes innate. Close it with mutation and it runs both directions: learned genes mutate, winning mutants re-assimilate as new instincts. Over many cycles **the `dna` pack itself evolves** — the bootstrap genome is no longer four routines a human typed (`packs/dna/manifest.json`); it is the distilled survivor of thousands of selection rounds. The system writes its own instincts. [substrate proven; breeding loop build]

### 2. Embodied evolution that is actually tractable

Classic genetic programming needs millions of evaluations against a *simulated* fitness function. SOMA's fitness is real outcomes through real ports — every interaction yields a typed `PortCallRecord`, folded into a free-energy delta (`routines.rs:321`). And the brain pre-filters mutations, so you need *dozens* of evaluations, not millions. **LLM-directed mutation + embodied fitness** is the combination that makes evolution-in-the-real-world cheap enough to run live. This pairing is the genuinely novel contribution. [build; both halves proven]

### 3. A memetic swarm — population-level evolution over the wire

`TransferRoutine` is proven (`transport.rs:53`). A mutation discovered on **node A** that survives selection can be broadcast to nodes **B** and **C** = horizontal gene transfer. Evolution is then not one agent improving but a *population* of embodied agents sharing genes over a LAN, each selecting against its own slice of the environment. A good survival behavior evolves *once, anywhere*, and propagates *everywhere* — no human writing a rule, no model retraining. The only missing piece is a gossip/affinity policy deciding *which* routines to push; the transport already carries them. [transport proven; gossip policy build]

### 4. Speciation and transfer-as-hybridization

We already proved one genome transfers coder → kitchen (`agi-through-body.md`, Phase 2, same DNA different ports). Add mutation and the genome *forks*: git-helpful mutations survive in the coder niche, manipulation-helpful ones in the kitchen niche → **speciation**, observable as a phylogeny once routines carry a `parent_id` (a small field addition). Crossover between a coder-lineage and a kitchen-lineage routine = **transfer learning as literal hybridization** — a carried abstraction ("probe before you commit") jumping domains. A mechanistic, inspectable account of transfer, against which black-box ML hand-waves. [transfer proven; lineage + speciation build]

### 5. An artificial immune system

The `dna.anomaly` lineage maturing under mutation *is* clonal selection / **affinity maturation**: variants that bind real anomalies better get cloned and refined; ones that don't, die. Point SOMA at a service and its anomaly genome self-tunes to that service's actual failure modes — a defense that evolves to fit the threat it faces. This is a sub-framing of the same engine, not a separate build. [build on proven anomaly + decay machinery]

## The disruptive product, in one line

**Software that evolves to fit its niche and maintains itself** — the antidote to code rot. Ship a seed genome and a poller; the system runs continuously; mutation proposes, the live environment selects, the swarm shares winners. When the world changes — new failure mode, new API, new attack — the population *adapts on its own*, because mutation + selection is a general adaptation engine and the brain makes it fast. Not "write code → it decays → humans patch." Instead: code that runs its own natural selection, in production, and teaches its whole fleet what it learned.

## The proof

Proof culture here means end-to-end behavior on real data, not "compiles."

**Single-node adaptation — done, proven.** `soma-project-evolution` runs the full
loop on real routine execution through the reference filesystem port. The unit of
evolution is the routine; fitness is the real `PortCallRecord` outcome read off the
session trace. A `readfile` seed breeds; mutation discovers a `stat` generalist; the
environment is flipped file→dir; the seed is invalidated and a mutated `stat`
descendant survives and succeeds where the seed fails — no human authored the winner.
Four phases (variation, selection-against, selection-for, adaptation), exit 0 iff all
pass, deterministic across runs. The harness's selection loop mirrors the reactive
monitor's feedback block line for line, driven as an explicit generation loop instead
of the wall-clock thread.

**Swarm — done, proven over real TCP.** `soma-project-swarm` shows horizontal gene
transfer end to end: a `stat` routine produced by the real mutation operator (origin
`Mutated`, validated by execution on node A) is broadcast over real TCP via the
production `RemoteRoutineBroadcaster` — the same type the breeding loop calls — and a
`LocalDispatchHandler` listener (the runtime's real receive path) registers it on the
peers as `PeerTransferred`. The peers then run the gene successfully through the real
reference port. Three phases (wire, gene flow, swarm-to-two-peers), exit 0 iff all
pass, deterministic across runs. The breeding loop in `world_state.rs` now broadcasts
a mutant the first time its reinforced confidence crosses the fitness floor (gated by
`mutation_enabled` + configured TCP peers), so on a live multi-node deployment a fix
that earns its fitness on one node spreads to the others with no human in the loop.

The remaining headline not yet shown as one automated run is the *full* loop on live
monitor threads across three real `soma` processes (deliberation → compile → breed →
broadcast → adopt) with a measured time-to-adapt. Every piece of it is built and
proven in isolation; what's missing is the single multi-process harness that strings
them together.

## Guardrails — evolution against real ports has real consequences

Mutation against live ports can reward-hack (a mutant that games its own fitness signal) or run away. The containment substrate already exists and must bound the evolutionary loop:

- **Exposure gating** — `default_deny_destructive` on pack exposure (`packs/dna/manifest.json`; policy-mutation default-deny noted at `types/pack.rs:115`). Evolved routines run *inside* this gate, never outside it.
- **Mutation scope** — `MutationMode` on peers (`types/peer.rs:91`) and `policy_scope` on routines (`types/routine.rs:129`) bound what a routine may touch.
- **The containment invariant** — a mutant must never be able to mutate the policy that contains it. Fitness is measured *inside* the sandbox; the sandbox is not a gene.

These are design constraints to honor from the first commit, not afterthoughts.

## Build status

| Piece | Status | Note |
|---|---|---|
| Genotype (routine as gene) | proven | `types/routine.rs:108` |
| Heredity (persist + boot-load) | proven | `persistence.rs:288`, `bootstrap.rs:351` |
| Fitness scalar (free energy) | proven | `routines.rs:321` |
| Selection-against (decay) | proven | `world_state.rs:169` |
| Horizontal gene transfer | proven | `transport.rs:53`, `transport.rs:871` |
| Mutation operators (blind) | **done** | `memory/mutation.rs` — point/insert/delete/perturb/niche/crossover, deterministic, lineage in id |
| Breeding loop (reproduction-for) | **done** | success branch of `world_state.rs` monitor: confidence reinforcement + `should_breed` gate; config `mutation_enabled` |
| Directed rescue (failure-guided) | **done** | `guided_mutate` (`memory/mutation.rs`) replaces the failed skill; monitor rescues before invalidation (`world_state.rs`) |
| Brain-directed (Lamarckian) mutation | build | LLM proposes the repair — needs a structured-output brain capability |
| Lineage / `parent_id` field | build | small field add; enables an explicit phylogeny (lineage works today via id convention) |
| Broadcast trigger + policy | **done** | `RemoteRoutineBroadcaster` (`runtime/remote.rs`); breeding loop broadcasts proven mutants once (`should_broadcast`); main wires it from configured peers |
| Single-node evolution proof | **done** | `soma-project-evolution` — 5 phases (incl. directed rescue), real port execution, exit 0 iff adapts |
| Swarm transfer proof | **done** | `soma-project-swarm` — 3 phases over real TCP; evolved gene runs on peers as PeerTransferred |
| Lineage / `parent_id` field | build | small field add; enables an explicit phylogeny (lineage works today via id convention) |
| Full multi-process live-monitor loop | build | one harness across 3 `soma` processes with measured time-to-adapt |

The vision is large; the build is small; the substrate is proven. That is the rare combination worth doing.
