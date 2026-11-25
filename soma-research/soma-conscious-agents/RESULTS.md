# Validation Results

Date: 2026-10-19

## Integrated real-observation suite

Suite `real-observations-2026-10-19` completed successfully. Its canonical
report digest is
`09b0b4007e8009fe7760f244c5a4e9fa28cfe5119b6b5f901c56321a3ad5910d`.
All acceptance criteria were fixed before evaluation.

The checksum-pinned human odor files supplied 5,888 trial rows and 92 rating
rows. Declared filters retained 3,776 trials and 88 ratings from 44 participant
sequences. No synthetic observation or pseudocount was inserted. The finite
observer trace contains 1,884 recorded visible-to-visible transitions; its
empirical kernel and exact 95% Chebyshev squared-error bound remain separate
from the ideal trace law.

The external intervention instrument supplied 415 measured transitions. The
chronological holdout used 334 for fitting and 81 for evaluation. Fourteen
held-out events had previously observed state-action support and 5 had the
correct top-one predicted successor; the other 67 remained explicitly
unscored. This is a measured limitation of the present world model.

The suite certified the following finite formal properties:

- the executable agent has typed, closed `P`, `D`, and `A` maps and computes
  `Q = DAP` exactly;
- compatible agent networks, joint dynamics, communicating classes,
  algebraic rank, and fusion idempotence are derived;
- experience products and trace-certified fine-graining are executable;
- first-return trace, trace order, stationary restriction, and finite trace
  evidence are distinct certified objects;
- the pinned external document round-trips losslessly through the transport
  boundary.

These results certify the declared finite model and recorded measurements.
They do not establish the consciousness ontology or a physical projection.

## Universal interaction regressions

All three pinned ARC-AGI-2 tasks use the same causal path:

```text
external JSON -> transport relations -> candidate headset perception
-> typed relational program posterior -> Q = DAP action
-> sealed commitment -> feedback -> posterior/headset/RTL revision
```

The cognitive source contains no ARC identifier, task identifier, matrix
codec, D4 primitive, or branch selected by a benchmark operation. The matrix
codec and answer comparison remain in `world.rs`, the boundary/evaluator.

### `0934a4d8`

The selected rank-generic term is `IsomorphicCompletion`. It discovers the
unavailable subobject and transports values through the signed-axis
isomorphism with the best relation support. The final audit re-induced the
unavailable symbol independently in every held-out fold.

- resource limit: 16 retained candidates;
- replay: 93/93 exact cells across four demonstrations;
- re-induced leave-one-demonstration-out: 93/93 exact cells;
- committed query: exact 9 x 3 action;
- unresolved and ambiguous query cells: 0;
- integration complete: yes; external task solved: yes;
- report digest:
  `a646635532780acda3561d583915872af3ec3e2fc124401b08a3a32864aa3a57`.

### `135a2760`

The selected term is `QuotientRepair`. It partitions a finite product by
homogeneous hyperplanes, peels supported frames, compares translation
quotients by prefix length plus residual error, and reconstructs each region
from the selected residue classes.

- retained executable hypotheses: 13;
- replay: 527/527 exact cells across two demonstrations;
- leave-one-demonstration-out: 527/527 exact cells;
- committed query: exact 29 x 29 action with 9 changed cells;
- unresolved and ambiguous query cells: 0;
- integration complete: yes; external task solved: yes;
- report digest:
  `91512f88448cd00a13de87dfaa3e4ede34fd239851c1bfd429319ee797011772`.

### `136b0064`

The selected term is `EntityTrace`. It discovers the source/canvas separator,
same-value entities under product adjacency, their ordering, the marker, and
four reusable entity-class trace macros. The output carrier and trace are
constructed by those macros. Each leave-one-out fold re-induces the macros
from the remaining demonstrations before executing the held-out case.

- retained executable hypotheses: 12;
- replay: 231/231 exact cells across three demonstrations;
- re-induced leave-one-demonstration-out: 231/231 exact cells;
- committed query: exact 19 x 7 action;
- unresolved and ambiguous query cells: 0;
- integration complete: yes; external task solved: yes;
- report digest:
  `2baebc05dc1c10c3b0487320ec6a8c8850f1148544d016b3188f219618673d1b`.

These tasks and their expected outputs were inspected during development.
Their success is integration evidence for the implemented mechanisms, not an
unbiased ARC generalization score. Such a claim requires untouched tasks fixed
before further implementation.

## Source and build audit

`cargo fmt --all -- --check`, `cargo check --all-targets`, and
`cargo clippy --all-targets -- -D warnings` pass. All 20 source modules are
connected through `lib.rs`; the rejected duplicate induction/runtime modules
are absent. A static audit finds boundary-only `MatrixCodec` references in
`world.rs` and no benchmark or pinned-task identifier in cognitive modules.

## Claims not established

The runs do not demonstrate AGI or empirical consciousness. They do not derive
the Born rule, special or general relativity, QFT, quantum gravity, cosmology,
the Big Bang, or the mint experience bridge. Those remain open projections
until they yield a mathematical derivation and an independent empirical
prediction.
