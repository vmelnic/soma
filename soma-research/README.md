# soma-research

Standalone research projects. **Not part of the SOMA runtime.** None of these is required to build, test, or run `soma-next`. Each project lives in its own subdirectory with its own `CLAUDE.md`, `README.md`, and dependency surface.

## Projects

| Project | Thesis | Status |
|---|---|---|
| `soma-brain/` | Native brain: LTC core + SDM + TTT + predictive coding. No transformer. | Active research. See `docs/native-brain.md`. |
| `soma-engram/` | Episodic memory consolidation experiments. | Active research. |
| `soma-graft/` | Graft a frontier-class SDM (extracted from a 70B teacher) onto a small chat model via a trainable bridge. | Active research. See `soma-graft/CLAUDE.md`. |
| `qwen-knn/` | Frozen base LM + non-parametric kNN-LM. Zero training. Cheapest falsification of "knowledge in RAM, not in weights." | Active research. First smoke test produced a negative result; see `qwen-knn/RESULTS.md`. |

## Why these are separated from the runtime

`soma-next` is the body — domain-agnostic, no LLM dependencies, no Python, no GPUs at runtime. These projects either build *replacement* brains (soma-brain), *augment* existing brains (soma-graft, qwen-knn), or explore memory primitives (soma-engram). A failed research bet here changes nothing about `soma-next`.

The body invariants in the root `CLAUDE.md` apply to `soma-next` and `soma-ports`. They do not apply here. Each research project sets its own invariants in its own `CLAUDE.md`.

## What does not belong here

- Anything imported by `soma-next` or `soma-ports`.
- Anything that ships in a `soma-project-*` proof harness.
- Production code paths.

If a research project graduates, the integration point becomes a `soma-port` (cdylib) that the runtime can load — not a direct dependency.
