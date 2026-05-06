# Results

## Run 1 — 2026-05-05 — Qwen2.5-3B + SOMA corpus (smoke test)

**Setup**
- Base model: `Qwen/Qwen2.5-3B` (fp16 on RTX 3090)
- Index corpus: `ingest/soma-codebase` (7 .rs files) + `ingest/soma-design-docs` (10 .md files)
- Held-out: `ingest/holdout` (4 files: `goal_registry.rs`, `bootstrap.rs`, `embodied-program-synthesis.md`, `context-os-proof.md`)
- Index: FAISS IVF-PQ, nlist=4096, nprobe=32, PQ m=64 nbits=8
- 170,244 (key, next_token) pairs collected in 31s
- Bench window: 256 tokens; 21,769 held-out tokens scored

**Result**

| | perplexity | top-1 |
|---|---|---|
| vanilla Qwen2.5-3B | **9.43** | 0.5811 |
| Qwen + kNN-LM (k=16, λ=0.25) | 12.58 | 0.5811 |

kNN-LM perplexity **regresses** by 33%; top-1 accuracy is unchanged. The argmax never shifted at this λ — kNN added probability mass to wrong tokens without changing the prediction.

**Diagnosis**
- Corpus is small relative to vocabulary (170k pairs vs 152k vocab) — most retrieved neighbours are semantically irrelevant.
- λ=0.25 weights kNN too aggressively given that signal-to-noise.
- IVF-PQ may be over-quantising at this index size; an exact `IndexFlatIP` would isolate retrieval quality from compression artefacts.
- Qwen2.5-3B is a smoke-test stand-in; real falsification target is Gemma 4 26B-A4B.

**Status**: not a win. Pipeline is proven; the kNN-LM signal at this configuration is negative-to-noise. Document and tune.

## Next moves

Ordered cheapest to most-expensive. Stop early on a win.

1. **λ / k sweep.** Run `bench.py` over the existing index for λ ∈ {0.05, 0.1, 0.15, 0.2, 0.25, 0.3} × k ∈ {8, 16, 32, 64}. Report best (λ, k). Cost: ~30 min wall time, no rebuild.
2. **Drop PQ.** Rebuild with `IndexFlatIP` (no quantisation). 170k × 2048 fp32 ≈ 1.3 GB — fits comfortably in RAM. Removes compression as a confound. Cost: ~1 min rebuild + sweep.
3. **Distance scaling.** Currently `weights = exp(-D / temperature)`; for IP-similarity FAISS returns cosine-like scores, not distances — sign and scale are likely wrong. Audit `D` distribution, switch to `softmax(D / τ)` with τ tuned to give non-degenerate `P_kNN`. Cost: code change + sweep.
4. **Bigger corpus.** Ingest full `soma-next/src/` tree (~10× current), and the rest of `docs/`. Larger index yields better neighbour quality. Cost: ~5 min rebuild.
5. **Better base model.** Set up `huggingface-cli login` on the Windows box, accept Gemma 4 license, switch to `google/gemma-4-26B-A4B-it`. Better hidden states → better retrieval keys → bigger kNN headroom. Cost: ~30 min download (50 GB) + rebuild.
6. **Larger held-out.** 21k tokens is enough to detect 1–2% perplexity moves; for noise-floor confidence at 0.5%, expand holdout to ~100k tokens.

If items 1–3 do not produce a single configuration where kNN-LM beats vanilla on perplexity *and* on top-1, the line is dead at this corpus scale and we move to item 4 or stop. If items 1–5 still produce no win, the kNN-LM hypothesis is falsified for SOMA-scale corpora on consumer hardware. Record the negative, kill the project, redirect to `qwen-sdm` (adapter-trained) or accept that RAG-into-context is the right baseline.

## Falsification bar (recap)

A win requires Gemma 4 (or Qwen) + kNN-LM to beat the same base model with no retrieval on:
- Held-out perplexity (lower is better) — meaningful margin, e.g. ≥5%.
- Held-out top-1 accuracy (higher is better) — at least matches, ideally beats.
- Hand-labelled QA over indexed SOMA design docs — kNN variant should answer correctly where vanilla hallucinates.

Anything less is not a win. Tie or worse on held-out perplexity is the negative result that ends the line.
