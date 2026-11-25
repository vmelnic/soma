# Mathematical Contract

Experience is taken as primitive. For a finite conscious agent

```text
C = ((X, Sigma_X), (G, Sigma_G), (W, Sigma_W), P, D, A, N),
```

`P: W -> X`, `D: X -> G`, and `A: G -> W` are Markov kernels under the row-kernel convention. The experience law is the actual composite

```text
Q = D A P.
```

This equation is a formal model invariant. It does not establish that a software process is conscious.

For a closed Markov chain `P` and visible support `A`, with productive hidden support `B`, observation is the first-return trace

```text
Tr_A(P) = P_AA + P_AB (I - P_BB)^(-1) P_BA.
```

The implementation also retains first-return duration, finite tail, and nonreturn mass. Trace witnesses define the observer preorder. Trace-of-trace composition, local Boolean operations, and normalized stationary restriction are checked exactly when their finite assumptions hold.

Recursive Trace Logic is represented over configurations `(d, x)`, where `d` names an experience space, headset, program, memory state, and a path through a policy hierarchy. If `K(d'|d)` is the configuration proposal, `H_l` are policy kernels, `M_(d,d')` is a trace-valid state-space transport, and `Q_d'` is the destination experience law, the executable transition is proportional to

```text
K(d'|d) * product_l H_l(p'_l|p_l)
* M_(d,d')(y|x) * Q_d'(x'|y),
```

with each configuration row normalized before composition. Thus policy states causally gate state-space changes.

A generative hypothesis is a prefix-coded tuple containing an executable headset, typed relational program, latent Markov law, goal posterior, policy type, and validated abstractions. The calculus constructs carriers and relations with subobjects, quotients, products, morphisms, grouping, closure, fold, scan, composition, and guarded fixed points. It contains no task identifier or grid transformation.

For a finite-function relation `f: C -> V`, the implemented structural models are stated on the carrier `C`, rather than on an external raster encoding. Completion searches partial carrier isomorphisms and minimizes relation disagreement. Repair searches quotient maps `q: C -> C/~` and minimizes prefix length plus residual error. Entity traces infer a partition into product-connected subobjects, an order on those subobjects, and a finite macro map from normalized entity signatures to carrier-relative actions. These are executable finite models; they do not assert that the same priors are sufficient for every world.

For public evidence `E`, selection uses predictive replay, leave-one-trace-out likelihood, and the code prior `2^-L(h)`. Parameters stored by a fitted structural term are re-induced from the remaining folds before the held-out action is evaluated. A symmetric local proposal over one typed component edit, followed by Metropolis acceptance, gives an exact reversible Markov search kernel. External feedback updates the transition/goal model, reweights the posterior, and proposes new typed support around a failed program.

Finite measurements never become exact trace laws by declaration. The empirical trace stores counts and a 95% Chebyshev certificate whose squared error radius is bounded by `min(1, 5/n)` for a row with `n` observed departures.

Products, trace refinements, transducers, and symbolic machines are formal constructions. A mapping from this architecture to quantum amplitudes, spacetime, fields, gravity, cosmology, or a particular quale remains outside the demonstrated results until it yields a derived prediction on independent measurements.
