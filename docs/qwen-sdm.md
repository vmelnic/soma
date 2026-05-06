# Qwen + SDM Hybrid

A frozen small Qwen-Coder model coupled with Sparse Distributed Memory replaces parametric knowledge with content-addressable RAM. The transformer keeps the grammar and reasoning patterns it already learned; SDM holds the facts and is swappable per project.

## Thesis

Knowledge baked into transformer weights is the expensive, immobile part of an LLM. The grammar — syntax, type-system instincts, multi-step reasoning chains — is the cheap and reusable part. Separate them. Ride a public 1.5B-class code model for the grammar. Put project-specific and library-specific knowledge in an SDM file you mount per repo.

This is the smallest plausible falsification of the SOMA invariant "knowledge in RAM, never in weights" on a real language model.

## Distinction from sibling docs

- `docs/ltc-sdm.md` — research bet replacing the transformer entirely with an LTC/CfC controller plus SDM. Higher upside, longer horizon.
- `docs/native-brain.md` — soma-brain as an independent project building the LTC+SDM core from scratch.
- This doc — pragmatic single-3090 path: keep the transformer, replace the knowledge.

## Architecture

```
input tokens
    |
Qwen embedding (frozen)
    |
[Qwen layers 0..k]   (frozen)
    |
hidden state h_k  ----> SDM query head (trained, ~10M params)
                              |
                         FAISS top-k read
                              |
                         gating MLP (trained, ~5M)
    |                         |
[Qwen layers k..N] <---- injected as KV-prefix or cross-attn
    |
output logits
```

### Three injection variants

1. **Logit fusion (kNN-LM style).** SDM produces a token distribution; blend with Qwen logits via a learned mixing weight. Cheapest. Trains in hours. Use to validate that SDM signal is real before investing further.
2. **KV-prefix injection.** SDM read is projected and prepended as fake KV-cache entries at one chosen layer. Qwen attention treats it as earlier context. No surgery on the model graph.
3. **Cross-attention adapter.** Insert a small cross-attention block at one layer. Q from Qwen hidden, K/V from SDM read. Most capacity, most training cost.

Recommended order: ship 1, then 2 if the signal holds.

## Trainable surface

| Component | Status | Approximate size |
|---|---|---|
| Qwen2.5-Coder-1.5B base | frozen | 1.5B (no grad) |
| SDM store (FAISS-IVF-PQ) | non-parametric | 0 |
| Query head (h_k -> SDM key) | trained | ~5–20M |
| Gating / fusion MLP | trained | ~5M |
| Optional LoRA on Qwen | trained | ~10–50M |

Total trainable parameters: 10–80M. Fits comfortably on a 24GB 3090 at batch 16, sequence 2048.

## SDM layout

Multiple SDMs mounted concurrently; the query head routes implicitly via learned key projections.

- **Project SDM.** Repo functions, types, modules. Tree-sitter chunked. Key derived from local context embedding; value is the full chunk plus docstring and signature.
- **Crate SDM.** Top public crates, rustdoc + signatures.
- **Stdlib SDM.** Language standard library plus core async runtime.

Each SDM is a file. Switching project means mounting a different `.sdm`. No retraining per repo — that is the whole point.

Capacity: 64GB RAM holds roughly 10^7 raw entries or 10^8 with product quantization.

## Training stages

### Stage 0 — Ingest (no gradients, ~1 day)

Tree-sitter chunk source at function and item boundaries. Embed each chunk with Qwen's own frozen embedder. Write key/value pairs into FAISS-IVF-PQ. Capacity-bound by RAM; no compute pressure.

### Stage 1 — Query head pretrain (2–4 days)

Self-supervised retrieval objective. Mask a function body, train the query head so the SDM read at the masked position returns the held-out function. Loss is contrastive (InfoNCE). No language-model loss yet — isolating retrieval signal.

### Stage 2 — Fusion training (3–5 days)

Plug SDM read into the forward pass. Loss is next-token cross-entropy on held-out code. Train gating MLP and, optionally, a LoRA on the upper Qwen layers. Compare perplexity against vanilla Qwen with the same context window.

### Stage 3 — Task reinforcement (optional, ~1 week)

Run inside SOMA. Model proposes patches; `cargo check` and `cargo test` outcomes drive a policy gradient on the trainable parameters only. Episodes feed the routine compiler.

## Why one 3090 is enough

No base-model pretraining. Qwen's 18T-token training run is reused as-is. SDM ingest is CPU/IO bound. Only the small adapter stack needs gradient updates, and 80M parameters at batch 16 is a fraction of 3090 capacity. The whole experiment runs end-to-end in roughly two weeks of wall time.

## Falsification — what beats Qwen + RAG

RAG retrieves into the context window. The model pays the full attention cost on retrieved tokens at every step, and recall depends on whatever attention happens to do with them. Qwen+SDM bypasses the context window: retrieval fires inside a chosen layer, gated, with no token-budget cost.

The bet is real if Qwen+SDM beats Qwen+RAG (matched retrieval corpus, matched base model) by a meaningful margin on:

- Held-out function completion on the project's own repo, post-training-cutoff commits.
- Held-out crate API usage where the crate was added to SDM but never seen during Stage 1/2.
- `cargo check` and `cargo test` pass rate on model-proposed patches over a held-out clippy backlog.

Tie or worse on these, the architecture is not winning over RAG — keep RAG, document the negative result, move on.

## What this does not test

This does not test the LTC/CfC dynamics claim. The transformer still does the reasoning. A win here proves only that SDM can replace parametric knowledge inside an existing transformer. The native-brain bet — that liquid dynamics can replace the transformer itself — remains separate, and remains unproven.

## First concrete step

Stage 0 is a single Python script: tree-sitter walk over `soma-next/src`, embed chunks with a frozen Qwen tokenizer + embedder, write FAISS-IVF-PQ. No training. Output is a `.sdm` artifact and a recall benchmark over hand-labeled queries against the repo. If recall is poor at this stage, no later stage will save it.
