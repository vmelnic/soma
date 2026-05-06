# soma-research/qwen-knn — Architectural Invariants

The smallest test of "knowledge in RAM, not in weights" on a real LLM. A frozen base model paired with a non-parametric kNN-LM index over an external corpus. No gradient updates anywhere.

See `docs/qwen-knn.md` in the repo root for the full thesis.

## What this project IS

```
input → Base LM (frozen, fp16, on 3090)
            │
   final layer  hidden h
            ▼
      FAISS top-k query over (h_corpus, next_token) index
            │
      P_kNN(token) ∝ Σ exp(-distance) · 1[token = stored_token]
            │
   blend with P_LM via fixed scalar λ
            ▼
      P_final = λ·P_kNN + (1-λ)·P_LM → sample → emit
```

Components:
- **Base LM**: frozen. Provides the language model. Default Qwen2.5-Coder-7B. Optionally Gemma 3 (4B/12B), Qwen3, or any HF causal LM exposing hidden states.
- **Corpus**: text/code files chosen per experiment. Index is rebuilt per corpus.
- **FAISS index**: `(hidden, next_token)` pairs. Built once, mmap'd, queried per generated token.

## Hardware target

- RTX 3090, 24GB VRAM
- 64GB system RAM
- Windows host accessed via `ssh win` (configured in user's `~/.ssh/config`)

## What is not trained

Nothing. Two scalar hyperparameters tuned by grid search:
- `λ` — kNN/LM mix weight, range 0.1–0.5
- `k` — neighbours per query, range 8–32

If "training" creeps in, the project has drifted. That belongs in `qwen-sdm` (adapter-trained) or `transformer-ltc-sdm` (full architectural rewrite), not here.

## Model choice

Model-agnostic. The index and inference path only require: HF causal LM, fp16-loadable on 24GB, exposes final-layer hidden states via `output_hidden_states=True`.

Default: `Qwen/Qwen2.5-Coder-7B` for code experiments.
Alternatives: `google/gemma-3-4b-it`, `google/gemma-3-12b-it`, `Qwen/Qwen3-4B`, `ibm-granite/granite-4.0-h-1b`.

The model choice changes the base capability. The kNN-LM mechanism is unchanged.

## Phases

| Phase | What | Status |
|-------|------|--------|
| 1 | Corpus + tokeniser + chunking | not started |
| 2 | Index construction (one forward pass over corpus) | not started |
| 3 | kNN-LM inference loop with logit blending | not started |
| 4 | Eval: perplexity + completion vs vanilla base | not started |
| 5 | Domain-swap test: build index over unseen library, measure recall | not started |

## Working rules

- Do not modify `soma-next`. This is a separate research repo.
- Do not introduce trainable parameters.
- Index files are large — they live on the Windows box only, never sync back.
- Code-only sync: `.py`, `.sh`, `.md`, `.txt`, `requirements.txt`.

## Scripts

Mirror `soma-graft/scripts` pattern:

- `scripts/sync.sh` — push code to the Windows box
- `scripts/run.sh <script.py> [args]` — sync, then run on Windows, stream output
- `scripts/fetch.sh <relative-path>` — pull a result file back

The `win` SSH alias is assumed to be configured on the local machine.
