# SOMA Conscious-Agent Runtime

SOMA is an executable research architecture built around finite conscious-agent dynamics, exact Markov traces, recursive changes of experience space, learned perception-action interfaces, and generative program induction.

The operational causal path is:

```text
external payload
  -> lossless transport codec
  -> typed relational frame
  -> joint posterior over headset/program/world/goal/policy hypotheses
  -> executable headset perception
  -> conscious-agent P, D, A maps with Q = DAP
  -> posterior-selected program execution
  -> world action and sealed commitment
  -> external feedback
  -> posterior, headset, policy, and persistent-memory revision
```

The cognitive modules do not import the matrix codec or benchmark identifiers. A matrix is one boundary adapter. The inducer receives typed finite relations and searches a compositional calculus of carriers, subobjects, quotients, morphisms, joins, grouping, closure, folds, scans, abstraction, and guarded fixed points. Interactive trace adapters use the same model to learn action-conditioned Markov laws, goal hypotheses, and plans.

The implementation keeps three evidence classes separate:

1. Exact finite algebra and code invariants are formal results.
2. Measurements and task outcomes come only from checksum-pinned external files and sealed evaluators.
3. Consciousness and physical projections remain explicit research hypotheses unless they produce independent empirical predictions.

Run the integrated real-observation validation:

```bash
cargo run --release -- validate manifests/integrated-real-observations.json
```

Run one sealed external interaction:

```bash
cargo run --release -- interact manifests/matrix-boundary-0934a4d8.json \
  --report artifacts/interaction-0934a4d8.json
```

Learn and inspect a plan from measured intervention traces:

```bash
cargo run --release -- plan manifests/observed-intervention-traces.json --horizon 8
```

`integration_complete` means the executable causal, trace, RTL, substrate, predictive-replay, sealing, feedback, and provenance certificates passed. `external_task_solved` means the committed action exactly matched the answer revealed afterward. Neither field is evidence of AGI or consciousness.

The source-driven redesign and its acceptance criteria are recorded in [HOFFMAN-ARCHITECTURE-AUDIT.md](HOFFMAN-ARCHITECTURE-AUDIT.md). Implementation status is tracked in [IMPLEMENTATION-CHECKLIST.md](IMPLEMENTATION-CHECKLIST.md).
