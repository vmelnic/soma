# Design Review

The source comparison and the defects that forced the redesign are documented in [HOFFMAN-ARCHITECTURE-AUDIT.md](HOFFMAN-ARCHITECTURE-AUDIT.md).

The replacement makes the following decisions:

- The conscious-agent tuple is first-class, and `Q = DAP` is computed rather than attached as a label.
- Observation is an exact first-return trace with an initial law, durations, and finite-sample evidence kept separately.
- Policies gate real morphisms between distinct experience spaces inside one Markov law.
- A headset is the learned perception-action interface, including perception fibers, action composition, geometry, trace, evidence, and posterior state.
- Matrix and benchmark concepts remain in boundary adapters and evaluators.
- Induction searches a typed compositional grammar with a prefix prior, held-out prediction, local reversible proposals, and active-query disagreement.
- The action is caused by the instantiated agent/headset/program before answer reveal. Feedback updates and persists the same mechanism.
- Universal representability and successful induction are separate claims.

The architecture implements finite formal objects and finite observations. It does not claim empirical consciousness, AGI, or derived physics. Those require independent operational criteria and held-out evidence.
