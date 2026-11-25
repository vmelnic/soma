# Architecture

## 1. Boundary

`instrument.rs` and `world.rs` own external syntax, checksums, codecs, public demonstrations, private answers, action commitments, and feedback. Cognition sees only `RelationalFrame` values. `LearningView` has no answer field; `SealedEvaluation` reveals answers once, after every query has a complete program-bound commitment.

`FiniteRelationalStructure` infers atom carriers and relation signatures without assigning object or task semantics. `IndexedStructure` is a discoverable finite-function hypothesis over that general frame, not a matrix type.

## 2. Conscious agents and networks

`ConsciousAgent` implements the finite tuple `(X, G, W, P, D, A, N)`. The maps are closed stochastic maps with checked domains and codomains. The qualia law is computed by composition:

```text
Q = D A P
```

`AgentNetwork::join` checks a shared world, synchronized counters, complete joint-action domains, and member support before deriving joint experience dynamics. Fusion certificates compute communicating classes, exact rank, and idempotence.

## 3. Trace logic

`MarkovChain` includes an initial distribution. `TraceWitness` records a support inclusion, exact first-return kernel, duration law, tail/nonreturn certificate, and normalized restricted initial law. The implementation provides trace order, trace transitivity, local meet/join/complement, stationary measures, and the stationary restriction map.

`FiniteTraceEvidence` is separate from the exact trace. It records observed counts, unobserved rows, empirical probabilities, and an exact conservative sampling-error certificate.

## 4. Recursive trace dynamics

`ExperienceModel` represents one experience space. `SpaceMorphism` represents identity, restriction, expansion, combination, or trace-certified fine-graining. `ExperienceCombination` constructs exact finite products and factor projections.

A `PolicyHierarchy` has arbitrary finite depth. Level zero controls space morphisms; each higher level controls states at the level below. `RecursiveTraceDynamics::compile` multiplies configuration, policy, transport, and destination-experience probabilities into one closed Markov law. The operational interaction executes an exact restriction/expansion cycle between a full computation space and its observer trace.

## 5. Headset

`HeadsetProgram` is part of every joint hypothesis. It owns observation and action carriers, executable perception and action programs, contextual role hypotheses, and typed correspondence hypotheses. Query execution starts by running this perception program. `LearnedHeadset` then realizes the selected interface as stochastic channels, an exact trace witness, perception fibers, a learned action algebra, and action-derived geometry before the agent chooses an external action.

## 6. Program and induction substrate

The finite structural program is a typed compositional relational calculus. Its carriers include boundaries, constructed carriers, subobjects, partitions, quotients, products, coproducts, images, preimages, pullbacks, and pushouts. Its executable terms include selection, typed images, joins, set operations, grouping, transitive closure, overlays, folds, scans, composition, abstraction, and guarded least fixed points.

Search generates schema-valid relation terms, retains a bounded beam, expands it compositionally, and stops on exact prediction, frontier exhaustion, or adaptive stagnation. The global resource bound is applied after demonstration replay and then prefix length, so short hypotheses from one headset cannot evict a predictive hypothesis from another headset before likelihood is considered. Programs receive an explicit self-delimiting prefix code. Relational predictive error and leave-one-trace-out Markov likelihood supply likelihood. Symmetric local edits define a Metropolis kernel over complete hypotheses. Each state contains the executable headset, program, latent dynamics, goal posterior, policy type, and validated macros. Query disagreement ranks available experiments without answer access.

The action-side program contains only the inferred semantic data relation. Coordinate, extent, and successor relations belong to the external frame schema and are reconstructed by the boundary after cognition emits an action. This prevents a transport encoding from becoming a learned world law.

Three rank-generic structural normal forms currently accelerate induction without introducing task identifiers or boundary concepts. `IsomorphicCompletion` completes an unavailable finite subobject by an induced signed-axis correspondence. `QuotientRepair` selects translation quotients by description length and residual error. `EntityTrace` induces product-connected entity signatures, their order, and executable trace macros on a constructed carrier. Their carrier, axes, values, correspondences, and macro parameters are inferred from relations. Data-dependent completion and trace parameters are re-induced inside every leave-one-demonstration-out fold.

There are no benchmark transformations or matrix/grid constructors in the cognitive source. Connected components, translation quotients, completion, and propagation execute over arbitrary-rank finite-function relations. A repeated subterm becomes a persistent macro only after independent reuse and positive held-out description gain.

The relational calculus handles finite external relations. Open Markov transducers supply sequential, parallel, and guarded-feedback composition. A two-counter symbolic Markov language supplies a countably infinite universal representation with lazy finite observer traces. Computational universality is a property of the language; it does not imply that induction finds every program.

## 7. Causal interaction

Candidate predictions compile the finite interface before answer access. For each query, the selected headset then performs perception, the conscious-agent transition chooses the program action, and the program executes from that perceived database. The resulting action must replay its pre-action prediction exactly before commitment. Only after all commitments exist does evaluation return feedback.

The report contains a validated graph for every episode:

```text
boundary observation -> primitive experience -> selected hypothesis
-> program execution -> world action -> sealed commitment
-> feedback -> posterior revision -> revised headset/RTL state
```

Persistent memory merges prior episodes instead of resetting. It retains episodic and semantic memory, typed programs, posterior weights, failed typed proposals, validated structural macros, active world models, policy descriptions, headset states, and autobiography. Compatible programs and failed revisions re-enter later posterior searches with provenance.
