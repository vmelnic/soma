# Hoffman Architecture Audit

Date: 2026-10-19

> **Historical baseline.** This document audits the implementation that existed
> before the conscious-agent replacement. The named rejected modules have since
> been removed. The replacement is described in [ARCHITECTURE.md](ARCHITECTURE.md)
> and is accepted only when every item in
> [IMPLEMENTATION-CHECKLIST.md](IMPLEMENTATION-CHECKLIST.md) and the final
> external runs are complete. The analysis below is retained to preserve the
> exact reasons for the redesign.
>
> **Replacement status:** the checklist is complete. The rejected modules and
> fixed matrix induction path are absent from `src/`; the current implementation
> uses relational frames, a joint program posterior, causal `Q = DAP` action,
> exact trace semantics, variable-space RTL, and feedback into the same
> posterior. Current measurements are recorded in [RESULTS.md](RESULTS.md).

## Verdict

The current operational path is not a generic conscious-agent headset. It is a
matrix-specific structural solver surrounded by reusable Markov components.
The ARC refactor moved specialization out of the codec and VM, but placed it in
`world_induction.rs`: rectangular masks, equal extents, aligned square windows,
the dihedral group D4, block isomorphism, coordinate orbits, and rectangular
matrix output are built into induction and execution.

The deeper architectural failure is causal. The structural solver commits the
action and the sealed evaluator scores it before the experience kernel, PDA
presentation, reflective dynamics, and observer trace are constructed. Those
objects describe a completed interaction; they do not produce perception,
choice, or action. The project therefore does not yet execute one
conscious-agent or recursive-trace dynamics across the external interaction.

Several modules remain worth keeping: exact rational Markov kernels,
first-passage/first-return calculations, sealed evaluation, lossless relational
transport, open transducer composition, provenance, persistent memory, and the
explicit separation between formal results and physical conjectures. They need
to be connected through a different core.

## Sources and standard of comparison

This audit compares code behavior, rather than names, against these sources:

- Hoffman, Prakash, and Chattopadhyay, [*Traces of Consciousness*](https://traceinstitute.org/papers/foundational/2025-hoffman-et-al-traces-of-consciousness.pdf), version dated 5 September 2025.
- Trace Institute, [*A Science of Reality: Observers, Interfaces, and Traces*](https://traceinstitute.org/papers/Trace-Whitepaper.pdf), 2026 research whitepaper.
- Hoffman, Prakash, and Prentner, [*Fusions of Consciousness*](https://www.mdpi.com/1099-4300/25/1/129), 2023.
- Prakash, [*On Invention of Structure in the World*](https://traceinstitute.org/papers/foundational/2019-prakash-invention-of-structure.pdf), 2019.
- Hoffman and Prakash, [*Objects of Consciousness*](https://sites.socsci.uci.edu/~ddhoff/Objects_of_Consciousness.pdf), 2014.

The 2026 whitepaper is a research program. Its physics statements are eight
conjectures, not completed derivations. It is used here to identify the proposed
RTL architecture, not as evidence that the conjectures are true.

## What the source theory actually requires

### 1. A conscious agent is a typed closed interaction

In the 2025 formulation a conscious agent is

```text
C = ((X, Sigma_X), (G, Sigma_G), (W, Sigma_W), P, D, A, N)
```

where `X` is the measurable experience space, `G` is the measurable action
space, `W` is the state space of the complete conscious-agent network including
the agent, `P: W -> X` is perception, `D: X -> G` is decision, `A: G -> W` is
action, and `N` sequences experiences. The experience kernel is the composite
`Q = D A P` under the paper's row-kernel convention. The theory assumes qualia,
choice, action, and sequencing; it does not assume a physical substrate.

The important software condition is not that a report contains values called
`perception`, `decision`, and `action`. Those stochastic maps must type-check,
compose, and cause the next step of the same agent/network dynamics.

### 2. An observer is itself a trace process

For finite kernels, observing a process on a support is a first-return trace
that preserves hidden excursions. Agent `A` observes `B` when the experience
kernel of `A` is a trace of the experience kernel of `B`. The trace relation is
a preorder on agents and a partial order on kernels modulo equality. Its local
Boolean structure, trace-of-trace composition, stationary measures, and the
homomorphism to the Lebesgue order of beliefs are part of the proposed theory.

A support subset alone is therefore not an observer. It becomes an observation
only together with its induced trace kernel, its relation to the source kernel,
and, for empirical use, uncertainty due to finite sampling.

### 3. Recursive trace logic changes the state space

The 2026 RTL proposal adds Markov dynamics over restrictions and expansions of
the current experience space. A policy transition may change the size and
content of `X`; a meta-policy is a Markov kernel over policies; the hierarchy can
continue to further levels. Valid expansions and contractions must respect
trace logic. Thus a catalog of fixed kernels sharing one state set is not an RTL
implementation.

### 4. A headset is an adaptive interface

In interface theory, an interface is the agent's perception-action channel. Its
icons and geometry need not copy objective structure. Prakash's construction of
perceived space uses an action group acting on perception fibers. Symmetries
such as D4 may emerge as an action group for a particular interface; D4 is not a
universal primitive of a headset.

For SOMA, a headset must be an executable pair or loop of observation and action
maps learned within the agent dynamics. A posterior over matrix reconstruction
programs can be one hypothesis family used by a headset, but it is not the
headset itself.

### 5. Networks, combinations, and fusions are part of the ontology

The world `W` is modeled as a network of conscious agents, not as a detached
physical payload. Earlier work supplies compatibility conditions for joining
agents; the 2023 work studies products, interacting cross terms, idempotent or
stationary fusions, Markov polytopes, and maps from communicating classes toward
decorated permutations and positive geometry. The final projection to physics
remains conjectural, but agent/network composition is not optional if the code
claims to implement conscious-agent theory.

### 6. Hoffman does not provide a general ARC induction algorithm

The papers give an ontology, stochastic agent dynamics, a theory of
observation, and a program for interfaces and physical projection. They do not
specify a solver that infers arbitrary executable programs from a few ARC
demonstrations. That missing inducer must be invented and evaluated by SOMA.
Calling a proposal kernel “Markov” cannot supply its hypothesis language or make
its search universal.

## Current execution path

The operational path is:

```text
matrix JSON
  -> MatrixCodec relational frame
  -> world_induction decodes the frame back to a matrix
  -> choose a common scalar as a rectangular mask
  -> enumerate equal-extent square tilings and D4 block matches
  -> build coordinate orbits
  -> read matrix values through orbit traces
  -> emit a rectangular matrix action
  -> reveal and score the sealed answer
  -> fabricate receive/perceive/decide/commit/feedback event labels
  -> fit an experience kernel to those labels
  -> factor it as PDA and compile reflective dynamics
  -> compute an observer trace
```

The required causal order is:

```text
current agent-network state W_n
  -> P_n produces an experience in X_n
  -> D_n produces a choice in G_n
  -> A_n changes the agent network W_(n+1)
  -> trace-valid policy dynamics may revise X, G, P, D, or A
  -> the next observation and feedback update the same dynamics
```

An external matrix, sensor stream, text conversation, or experiment can provide
boundary events. The cognitive core should receive only typed relations/events.
It must infer a generative latent process, an interface, and an action program
inside the same causal loop.

## Code-to-theory comparison

| Requirement | Current implementation | Verdict | Missing bridge |
|---|---|---|---|
| Primitive experience | `ExperienceId` is opaque and `FoundationalAxioms` declares experience primitive. | Partial | Runtime interaction states are lifecycle strings and payload digests constructed after action, rather than the states that drove it. |
| Agent tuple | `PdaFactorization::derive` factors any kernel through one witness per nonzero edge and an identity decision map. | Incorrect equivalence | No first-class `ConsciousAgent`, distinct `X/G/W`, network state, counter, or causal PDA loop. The factorization is algebraically valid but non-identifiable and semantically arbitrary. |
| World as agent network | External frames and internal descriptions are called worlds. | Missing | No network whose states are configurations of agents, no agent membership relation, and no agent-to-agent action. |
| Observation by trace | Exact first-return and first-passage calculations preserve hidden excursions, duration, tail, and nonreturn mass. | Strong partial | No first-class trace-order witness between observer and observed kernels, no transitivity certificate, no trace lattice operations, stationary map, or finite-sample uncertainty. |
| Recursive state-space change | `ReflectiveDynamics` forms a joint kernel over description and experience. | Contradicted by implementation | All descriptions are required to share the exact same experience states. Policies choose fixed windows/hypotheses; they cannot restrict, expand, combine, or fine-grain `X`. |
| Policy/meta-policy hierarchy | Two policy enums and batch-size/order meta-policies are serialized into descriptions. | Superficial partial | They manage search resources over a supplied catalog. They are not kernels over trace-valid state-space transformations, and the hierarchy cannot grow recursively. |
| Headset | `ObserverWindow` selects support; `LearnedHeadset` stores a posterior over D4 matrix programs. | Incorrect abstraction | A single learned perception-action channel participating in the CA loop, with geometry derived from learned action groups and utility/evidence. |
| Interface geometry | Intervention-equivalence classes and shortest intervention paths are calculated. | Useful original extension | They depend on externally labeled interventions and are not connected to ARC action. No learned action group, perception fibers, faithful/transitive action, or interface quotient. |
| Universal computation | Open Markov transducers compose sequentially, in parallel, and through guarded feedback; a two-counter language is expressively universal. | Expressivity only | The ARC path does not use it. The inducer cannot invent primitive semantics; it repeats one supplied primitive and compresses repeated expressions. |
| Universal induction | A message-length scorer, finite frontier, and posterior/hypothesis kernel exist for sequence transitions. A second inducer searches matrix structures. | Missing | One generative posterior over latent world, interface, agent law, and program; typed abstraction discovery; reusable learned operators; counterfactual/interventional scoring. |
| Closed learning loop | Sealing and action-before-answer are implemented correctly. | Boundary correct, cognition disconnected | Feedback does not revise the program that acted. The reflective kernel is created only after evaluation. |
| Agent combination/fusion | Parallel transducer product exists. | Not CAT combination | No conscious-agent compatibility, joins, tensor products with agent semantics, interacting cross terms, fusion, or Markov-polytope analysis. |
| Physical projection | Projection code labels physics claims as open conjectures and cannot affect induction. | Honest boundary | No derived map to decorated permutations/positive geometries, quantum instruments, spacetime, particles, or scattering observations. |

## Concrete hard-coded assumptions in the claimed generic headset

The following are cognitive assumptions, regardless of the fact that the
transport adapter itself has zero cognitive operators:

1. [`StructuralPrimitive`](src/world_induction.rs#L23) explicitly includes an
   aligned finite window, D4 action, canonical block isomorphism, and orbit
   closure.
2. [`induce_world_program`](src/world_induction.rs#L284) immediately decodes
   every relational frame through `MatrixCodec`; it cannot consume an arbitrary
   relational world.
3. [`complete_demonstrations`](src/world_induction.rs#L1076) accepts a selector
   only when its coordinates fill a rectangular bounding box and the action is
   that exact rectangle.
4. [`propose_tilings`](src/world_induction.rs#L614) enumerates aligned square
   tilings from matrix height and width.
5. [`build_program`](src/world_induction.rs#L638) requires every completed
   demonstration to have the same finite rank and extent.
6. [`execute_queries`](src/world_induction.rs#L473) finds a rectangular marker
   bound and emits precisely that rectangular matrix.
7. [`structural_grammar`](src/world_induction.rs#L596) records
   `task_specific_operators: 0`, although its primitive vocabulary encodes these
   rank-two matrix assumptions. The field only counts declared operators; it
   does not measure specialization.

Task `0934a4d8` passed because these assumptions fit that task. Task `135a2760`
failed before action because its periodic texture repair does not present the
same rectangular-mask/equal-extent structure. This is evidence that the first
pass measured a matching prior, not a domain-general headset.

## Disconnected modules and misleading certificates

### Two cognitive islands

`runtime.rs` uses `RecursiveInducer`, `ProgramRegistry`, symbolic programs,
transducers, object geometry, and persistent memory on sequence data.
`interaction.rs` uses the independent solver in `world_induction.rs`. The
operational interaction never calls the universal `RecursiveInducer`, symbolic
backend, macro registry, or object geometry to form its action.

The universal language therefore cannot explain either ARC result. Conversely,
the matrix inducer's learned structures cannot become general transducer macros
or revise the persistent cognitive runtime.

### Post-hoc Markov closure

[`run_world_interaction`](src/interaction.rs#L106) performs induction, query
execution, and sealed evaluation at lines 119–132. It constructs the experience
log, experience kernel, PDA factorization, reflective dynamics, and observer
trace only at lines 135–170. Therefore the reported “one reflective Markov law”
was not the law that chose the action.

[`interaction_experience_log`](src/interaction.rs#L300) creates states such as
`receive:<digest>` and `feedback:<digest>` from already completed lifecycle
phases. This provides provenance for an orchestration trace. It is not evidence
that these are primitive experiences or that their kernel generated cognition.

### Checks that certify presence rather than causal use

- `relational_calculus_executed` checks a standalone calculus certificate and a
  nonempty program digest. It does not show that the calculus produced the
  action.
- `headset_learned_and_revisable` checks that a selected program exists and a
  hypothesis kernel validates. No feedback-driven revision is executed.
- `feedback_returned_as_experience` checks the presence of a `feedback:` label
  and digest. That feedback never reaches the solver that acted.
- `one_reflective_markov_law` validates a kernel compiled after evaluation.
- `domain_neutral_runtime` checks the codec identifier and adapter counter. It
  does not inspect the matrix assumptions in `world_induction.rs`.

The completed interaction checklist is consequently a historical component
check, not proof of the claimed architecture.

## Other omissions found in the core

### Trace logic

- No `TraceOrder` object or exact certificate that one kernel is a trace of
  another.
- No executable trace-of-trace composition theorem.
- No meet, join, complement, compatibility, or simultaneous-verifiability
  operations within a parent's local Boolean algebra.
- No initial distribution or stationary-measure object, and no stationary-map
  certificate from trace order to belief order.
- No statistical distinction between an ideal trace and a trace estimated from
  finite observations.

### Recursive trace logic

- `ReflectiveDynamics::compile` rejects descriptions with different experience
  state sets.
- State-space inclusions, restrictions, expansions, combinations, and
  fine-grainings are not represented as typed morphisms.
- Policy transitions are supplied as an update table instead of being generated
  from admissible trace transformations.
- Meta-policy depth is fixed and only adjusts batch size or proposal order.
- Surprise or predictive discrepancy between a larger process and its realized
  trace is not an executable objective.

### Induction

- `RecursiveInducer` receives its primitive from the caller and only constructs
  sequential, parallel, and guarded-feedback repetitions of that primitive.
- Observer windows, policies, and meta-policies come from caller-provided finite
  alternatives.
- Operator “invention” only aliases repeated subexpressions that search already
  generated; it does not create a new typed primitive or semantics.
- `hypothesis_dynamics` is a selection kernel computed after scoring. Markov
  transitions over fixed hypotheses do not by themselves infer a new model.
- The prefix cost is serialized byte length, not a demonstrated prefix-free
  code or a representation-invariant prior.

### Networks and interfaces

- No compatible joins of conscious agents, synchronized experience counters,
  or network-level transition semantics.
- No distinction between agent-relative experience, action, and the state of
  the containing network.
- No learned perception fibers or action groups from which interface geometry
  can arise.
- No payoff/fitness or other declared operational criterion for retaining one
  interface over another.
- No counterfactual intervention loop in the ARC interaction path.

### General boundaries

`WorldProtocol` is typed, and `MatrixCodec` is a proper lossless transport
boundary. However, `load_sealed_interaction` currently loads only the matrix
manifest format. A new modality still requires code changes. The manifest
describes channels and fields but cannot declare general actions,
interventions, costs, or admissible state-space transformations.

## Correct replacement architecture

The replacement should have eight layers with one causal execution path.

### 1. Measurement boundary

Accept immutable, provenance-bearing events and relations. Codecs may express
syntax such as matrix coordinates, graph edges, tokens, timestamps, sensor
channels, and allowed external actions. They may not supply segmentation,
symmetry, object classes, latent causes, or solution rules.

### 2. Conscious-agent core

Implement `ConsciousAgent<X, G, W>` with typed stochastic maps `P`, `D`, `A`,
counter `N`, and exact composition to `Q`. Implement an `AgentNetwork` whose
world state is a composition of agents. Add checked directed/undirected joins,
products, interaction cross terms, and explicit compatibility certificates.

Experience can remain the chosen primitive ontology. To stay comparable with
CAT, the code must distinguish an assumed primitive `Q` from a supplied or
learned `P/D/A` realization, retain non-uniqueness of possible factorizations,
and avoid presenting the edge-witness factorization as the agent's discovered
semantics.

### 3. Trace logic

Represent a Markov chain with state space, kernel, and initial distribution.
Make a trace witness first-class: source, support/inclusion, induced trace,
duration law, and proof digest. Implement the trace preorder, transitive
composition, local meet/join/complement when defined, compatibility, stationary
measures, and the stationary-map certificate. Learned finite traces must carry
sample counts and uncertainty separately from exact algebra.

### 4. Recursive state-space dynamics

Represent each experience space as an object and every admissible restriction,
expansion, combination, or fine-graining as a typed morphism carrying a trace
witness. A policy is a kernel over these morphisms. A meta-policy is a kernel
over policy descriptions. Use a recursive description type so the hierarchy is
not fixed at two levels. Compile the chosen state-space morphism and experience
transition into one executable law.

### 5. Learned headset

Define a headset as the current agent-relative observation/action interface:

```text
Headset = (perception channel, action channel, trace witness,
           learned action algebra, utility/evidence state)
```

Infer perceptual equivalence, latent objects, transformations, and topology
from interaction. Learn candidate action groups from observed composition laws;
derive geometry from their action on perception fibers. D4 may be recovered for
some grid worlds, but it must enter as a learned hypothesis with evidence and
provenance.

### 6. Universal program hypotheses

Retain relational algebra, open transducers, and the symbolic register-machine
backend as a typed executable hypothesis language. Add abstraction operators
that can introduce latent entities, relations, recursive generators, quotient
types, and reusable macros. A learned macro must be callable across worlds by
type, not by matrix coordinates or benchmark identity.

Computational universality establishes only that programs can be represented.
It does not establish that the inducer can find them.

### 7. Joint generative induction

Maintain a posterior over complete executable hypotheses:

```text
h = (agent network, latent dynamics, headset, program, policy hierarchy)
```

Each hypothesis must generate observable traces and action consequences.
Proposals may revise any typed component through local reversible moves,
program synthesis, learned macro reuse, and state-space expansion/contraction.
Score predictive likelihood on held-out observations and interventions, plus a
declared code length. Use active experiment selection when the external
protocol permits interventions. The Markov proposal kernel is the search
dynamics; correctness comes from generative semantics and predictive evidence.

### 8. Causal interaction and evaluation

For each episode, run the selected or posterior-mixed agent dynamics to produce
perception, choice, and an external action. Commit the action before answer
access. Convert feedback into the next boundary event and update the same
posterior, policy hierarchy, memory, and headset. The certificate must trace the
actual causal chain from source event through every executable program to the
committed action and subsequent revision.

ARC then becomes one adapter: the matrix codec supplies scalar incidence and
coordinate-order relations; the action protocol accepts a finite relation that
decodes to a matrix. The core may infer periodic generators, sparse corruption,
objects, symmetries, and repair programs, but no ARC- or grid-specific operator
appears in the headset or inducer.

## What should be retained, replaced, or removed

### Retain and generalize

- `exact.rs` and most of `kernel.rs`.
- First-passage/first-return algebra in `trace.rs`.
- Sealed public/evaluator separation in `world.rs`.
- Lossless relational codecs and provenance.
- Relational algebra in `relational.rs`.
- Open transducer composition in `transducer.rs`.
- Symbolic local rules in `symbolic.rs` as a program language.
- Persistent storage and digest verification in `memory.rs`.
- The no-feedback projection boundary and honest claim statuses.

### Replace

- Replace `ObserverWindow` as the headset abstraction with trace witnesses and a
  learned perception-action interface.
- Replace fixed-state `ReflectiveDynamics` with typed variable-state RTL.
- Replace `RecursiveInducer` and `world_induction` with one joint generative
  inducer over the universal program language and headset/network hypotheses.
- Replace post-hoc lifecycle experience construction with event states emitted
  while the causal agent loop executes.
- Replace checklist booleans with provenance paths proving that each component
  contributed to the committed action and changed after feedback when claimed.

### Remove from the cognitive core

- `LearnedObservationMask`, `AlignedFiniteWindow`, D4, canonical square blocks,
  coordinate orbit closure, rectangular bounds, and equal-extent assumptions.
- `task_specific_operators: 0` as a neutrality certificate.
- Claims that a valid posterior kernel alone makes a headset revisable.
- Claims that post-evaluation reflective compilation caused the interaction.

These structures may return later as ordinary learned hypotheses or as
adapter-level syntax only where the external protocol explicitly supplies them.

## Acceptance criteria before another ARC run

1. The operational command uses one inducer and one program registry; there is
   no separate matrix induction module.
2. No cognitive module imports `MatrixCodec`, ARC identifiers, rank-two
   coordinates, rectangles, tilings, or D4.
3. A first-class conscious-agent/network transition produces every committed
   external action.
4. The same run constructs a trace witness connecting observer and observed
   kernels before the action is chosen.
5. A policy can execute a real change between different experience spaces, and
   the change has a trace-valid morphism and provenance.
6. Feedback from a completed episode changes a future posterior, program,
   headset, or policy through the same dynamics; the report shows the before and
   after executable digests.
7. Learned operators are typed program definitions whose semantics replay from
   primitive operations and whose reuse is measured on a different episode or
   world.
8. The matrix manifest uses only a transport/action protocol. Task 1 and task 2
   are run once each from clean sealed inputs; neither can select a special
   grammar or code path.
9. Architecture certificates follow the causal provenance graph to the action.
   Component presence and standalone mathematical consistency are reported
   separately.
10. Failure to induce a supported program remains a valid result. It must not be
    repaired by inserting the missing transformation into the core.

## Priority of the omissions

### P0: required to restore the claimed architecture

1. One `ConsciousAgent`/`AgentNetwork` execution model with typed `P/D/A`.
2. First-class trace witnesses and trace order.
3. Variable experience spaces and typed RTL transformations.
4. One joint generative inducer used by operational interaction.
5. A headset implemented as a learned perception-action interface.
6. Feedback that revises the same mechanism that acted.

### P1: required for open-ended learning

1. General latent relation/entity invention.
2. Learned action algebras and interface geometry.
3. Cross-world typed macro transfer.
4. Agent composition, compatibility, and fusion semantics.
5. Active intervention selection and uncertainty-aware finite traces.
6. Persistent posterior, policies, autobiographical state, and operator library.

### P2: research projections

1. Communicating-class maps to decorated permutations and positive geometries.
2. Quantum-instrument extensions and observer back-reaction.
3. Controlled limits toward spacetime, quantum amplitudes, fields, particles,
   scattering, gravity, cosmology, and empirical tests.

P2 must remain outside selection until an explicit mathematical map and
held-out empirical prediction exist. ARC can test induction and interface
construction; it cannot validate the consciousness ontology or the physical
projection.

## Final assessment

The refactor achieved a clean external matrix boundary and answer sealing, but
did not achieve a generic headset. It relocated grid knowledge into
`world_induction.rs` and attached the conscious-agent/trace machinery after the
solver had acted. The omitted foundation is not another ARC operator. It is a
single causal model in which an agent network, its trace observer, its changing
experience space, its learned interface, and its executable program are jointly
inferred and jointly responsible for action.
