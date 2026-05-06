# Qwen + kNN-LM

A frozen Qwen2.5-Coder paired with a non-parametric nearest-neighbour language model over an external corpus. No gradient updates anywhere — the base model is reused as-is, and "knowledge" lives in a FAISS index of `(hidden_state, next_token)` pairs.

## Thesis

The cheapest possible test of "knowledge in RAM, not in weights" against a real language model. If this beats vanilla Qwen on a held-out code corpus, the SDM-as-memory hypothesis has a one-day proof. If it does not, no amount of adapter training is going to rescue the line.

## How it works

At every generated token:

1. Qwen runs its normal forward pass over the prompt, producing hidden state `h` (final layer) and logits `P_LM(next_token)`.
2. FAISS is queried for the top-k stored hidden vectors closest to `h`.
3. Each stored entry carries the token that actually followed it in the corpus. A distribution `P_kNN` is built by softmax-weighting those tokens by negative distance.
4. Final distribution: `P_final = λ · P_kNN + (1 − λ) · P_LM`.
5. Sample or argmax from `P_final`. Emit. Repeat.

The prompt never grows. Retrieved entries are vectors and token ids — they never get tokenised, never enter attention, never spend context budget. This is the fundamental difference from RAG.

## What is not trained

Nothing. There is no optimiser, no learning rate, no checkpoints, no LoRA, no adapter. The only knobs are two scalars:

- `λ` — mix weight between LM and kNN distributions, typical range 0.1–0.5.
- `k` — number of neighbours, typical range 8–32.

Both are tuned by grid search on a validation set. Minutes, not days.

## Index construction

One forward pass over the chosen corpus, no backprop:

- Tokenise corpus.
- Run Qwen across it; at each position record `(final_layer_hidden, actual_next_token_id)`.
- Write pairs to a FAISS-IVF-PQ index. Keys are the hidden vectors, values are the token ids.

A 10M-token corpus produces ~10M entries, fits in a few GB of RAM with product quantisation. A 1B-token corpus needs ~50GB compressed and is still tractable on a 64GB box.

## Hardware footprint

A single RTX 3090 with 64GB system RAM is sufficient for both index construction and inference. Index construction is forward-only, so it is fast — limited by IO and tokenisation rather than compute. Inference adds one FAISS query per generated token, mitigated by caching or by only firing on high-entropy steps.

## Distinction from sibling docs

- `docs/qwen-sdm.md` — adapter-trained variant. Trainable query head + gating MLP, multiple injection points. Larger commitment, longer training horizon.
- `docs/ltc-sdm.md` — replaces the transformer entirely with an LTC/CfC controller plus SDM. The bigger architectural bet.
- This doc — the smallest experiment. Zero training, one corpus, one weekend.

## Falsification

Build the index over Rust Book + rustdoc of selected public crates. Hold out a separate set of code files the model has never seen and the index does not contain. Measure on the held-out set:

- Token-level perplexity, vanilla Qwen vs Qwen+kNN-LM.
- Function-completion exact match and edit distance.
- Hand-labelled API-usage queries on crates that are present in the index.

If perplexity drops meaningfully and completions improve on the in-index domain without regressing on the held-out set, the hypothesis survives. If results are a wash, the line is dead — record the negative and stop.

This is intentionally cheap to disprove. That is the point.

## Constraints

The experiment lives in a separate repository (for example `soma-research/qwen-knn`) and does not touch `soma-next`. It produces a research result and possibly a SOMA port wrapper later if the result is positive. It does not modify the runtime.

## First concrete step

Write the index-construction script: tokenise corpus, run Qwen forward, dump `(hidden, next_token)` pairs to FAISS-IVF-PQ. No model code modified. No training loop. The script either runs end-to-end and produces an index, or it does not — and that is day one.
