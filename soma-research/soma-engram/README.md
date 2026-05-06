# soma-engram

Decompile a pretrained transformer into:
- **Static knowledge** — all MLP weights as explicit (key, value)
  associative memory entries (after Geva 2021).
- **Liquid Time-Constant (LTC) core** — a continuous-time integrator
  whose vector field reproduces the original transformer's depth-wise
  computation.

After decompilation the original transformer is no longer needed at
inference. The (SDM, LTC) system reproduces its outputs.

This README documents what's empirically proven, what's measured, and
what is *not yet solved* — particularly around running large models on
consumer hardware.

## The four-paper synthesis (the basis of this project)

| Equivalence | Source | What it proves |
|-------------|--------|---------------|
| Transformer = discretized Neural ODE in depth | Chen et al. (2018) | Layer-by-layer evolution can be expressed as ODE integration |
| Modern Hopfield = Attention | Ramsauer et al. (2021) | Attention is associative memory retrieval |
| MLP = Key-Value Memory | Geva et al. (2021) | MLPs are explicit (k,v) stores |
| Attention ≈ SDM | Bricken et al. (2022) | Attention is sparse distributed memory |

## What's empirically proven (Qwen-3B)

| Claim | Method | Result |
|-------|--------|--------|
| MLP layers ARE associative memories | `extract.py` + `verify_mlp_streaming` | 0% RMSE on all sampled layers (byte-exact reconstruction) |
| Native forward without HuggingFace works | `forward_native.py` | Produces `'Paris'` matching HF, logit RMSE 0.25% (fp16 noise) |
| LTC surgery from transformer weights is exact | `ltc/surgery.py` + `test_ltc.py` | LTC.integrate(L=36) matches HF Qwen exactly |
| Trajectory distillation pipeline works | `distill.py` | Initial loss 9.97e-6 (surgery is faithful), gradients flow, fits in 19/24GB VRAM |
| (SDM, LTC) system can chat | `chat.py` | Coherent responses (haiku, math, etc.) using only extracted weights |

## What's measured

### Speed (Qwen-3B fp16 on RTX 3090)

| Implementation | tok/s | Notes |
|----------------|-------|-------|
| HF Qwen-3B with KV cache | 17.7 | HF native generate() with all optimizations |
| Our LTC (no KV cache, no compile) | **24.0** | Naive Python, recompute full sequence per token |
| torch.compile | unavailable | Triton not on Windows |

**Surprise**: our naive LTC is *faster* than HF's KV-cached generate. HF's
overhead in `.generate()` (sampling, masking, hooks) costs more than KV
cache saves at this size. The hardware (RTX 3090 + 3B fp16) is the
bottleneck, not the implementation.

### Memory

| Operation | Peak VRAM | Comment |
|-----------|-----------|---------|
| Extraction | ~7GB | Loaded on CPU; only verification touches GPU per layer |
| Native forward | ~7GB | Full LTC on GPU |
| Trajectory distillation (all params, batch=1) | 19GB | Backward through 3B with SGD |

## What is **NOT** solved

### Big models on consumer hardware: the honest analysis

The original goal "1T on a 3090 at chat speed" requires touching ~50GB+
of weights per token. Even with int4 quantization and streaming, this
is fundamentally bandwidth-bound (PCIe 4.0 x16 ~12 GB/s realistic):

| Model | Per-token streaming cost | Achievable speed |
|-------|--------------------------|------------------|
| 3B fp16 (fits) | none | 24 tok/s |
| 7B int4 (fits) | none | ~30-40 tok/s estimated |
| 70B int4 (fits w/ swap) | partial | ~5 tok/s |
| 120B int4 (60GB) | full streaming | ~0.2 tok/s — unusable |
| 1T int4 (250GB) | full streaming | ~0.05 tok/s — unusable |

**Streaming inference does not scale to chat speed for 100B+ models.**
The bottleneck is moving 60GB+ of weights per token over PCIe.

### What would actually be needed

Three architectural changes, each researched separately, none currently
implemented in this project:

1. **Sparse MLP via top-k gate**
   - Only compute the top-k intermediate dims where `silu(gate · x)` is large
   - Implemented (see `LayerSlice.mlp(top_k=...)`) but **not validated** —
     accuracy under this approximation needs measurement.
   - Expected: 10x speedup with ~5% accuracy loss based on similar work
     (Mixture-of-Experts, Deja Vu, Power-LLM)

2. **Sequence-wise state-space (SSM) replacement of attention**
   - Replace softmax attention with Mamba-style selective state space:
     `s_new = A(x) * s + B(x) * x`, `out = C * s + D * x`
   - O(state_dim) per token regardless of sequence length
   - Removes KV cache memory growth, removes per-token re-encoding
   - Surgery from transformer attention to SSM weights is **non-trivial**
     — the math equivalence (Hopfield = Attention) doesn't directly
     give SSM parameters.
   - Requires: training data, distillation loss against teacher

3. **Adaptive depth (the original "compression" goal)**
   - LTC learns to skip layers per token
   - Tried in `compress.py`: 36→18 steps with naive layer-skipping init
     hits 0% top-1 match, slow recovery (5% after 200 SGD steps).
   - Smarter init (averaging skipped slices) is the next experiment.

### What "liquid networks" actually solve (vs what they don't)

| Problem | Solved by LTC? |
|---------|----------------|
| Adaptive integration depth | ✅ Yes — ODE solver chooses steps |
| Continuous-time dynamics | ✅ Yes — state evolves smoothly |
| **Sparse weight access per token** | ❌ No — orthogonal concept |
| **Sequence-wise recurrence** | ❌ No — LTC depth-only, attention still global |
| **Running 100B+ models on 24GB** | ❌ No — fundamentally bandwidth-bound |

Hasani's actual LTCs are tiny (200-1000 neurons) for control tasks. They
achieve efficiency by being **small**, not by being **sparse**. Liquid AI's
LFM2 (1.2B) does full forward passes per token — same memory cost as a
transformer of its size. Liquid networks are not a magic solution to the
big-model-on-small-GPU problem.

## Files

```
soma-engram/
├── extract.py        ✅ Phase 1: extract Qwen weights to flat .pt file
├── forward_native.py ✅ Pure-tensor forward (no HF) + --stream flag
├── debug_forward.py  ✅ Layer-by-layer divergence debugger
├── debug_traj.py     ✅ Trajectory comparison debugger
├── capture.py        ✅ Cache Qwen hidden trajectories
├── distill.py        ✅ Trajectory distillation training loop
├── compress.py       🟡 Adaptive-depth compression (works, slow convergence)
├── chat.py           ✅ Interactive chat with the decompiled model
├── bench_speed.py    ✅ LTC vs HF speed comparison
│
├── ltc/
│   ├── core.py       ✅ LiquidCore with sparse MLP top-k support
│   └── surgery.py    ✅ initialize_from_qwen
│
├── checkpoints/      qwen3b_full.pt (6.17GB), ltc_distilled.pt
├── data/             trajectories.pt (200 captures, 390MB)
└── scripts/          sync.sh, run.sh, fetch.sh
```

### Files NOT yet written

- `ltc/ttt.py`, `ltc/heads.py`, `ltc/sdm_query.py`
- Task fine-tuning (SQuAD QA, code, etc.)
- Benchmark suite vs original Qwen / RAG / soma-brain
- CPU-only inference path (architecturally easy, untested)
- SSM (state-space) replacement of attention — research direction

## Workflow

All compute on Windows RTX 3090. Code locally:
```
./scripts/sync.sh                   # push code to Windows
./scripts/run.sh extract.py         # run on RTX 3090, stream output
./scripts/fetch.sh checkpoints/     # pull artifacts back
```

## Realistic next steps (revised honestly)

| Direction | Effort | Value |
|-----------|--------|-------|
| Validate sparse MLP top-k accuracy on real prompts | Hour | Confirms #1 in "what would actually be needed" |
| CPU inference test (port forward_native, no GPU) | Hour | Proves the "no GPU at runtime" claim for small models |
| Editable knowledge demo (overwrite SDM entry, see effect) | Hour | Demonstrates editable memory advantage |
| Add task heads (SpanExtractor on SQuAD) | Days | Fair comparison vs soma-brain on real benchmark |
| SSM attention replacement | Weeks | Required for sequence-O(N) inference, big models |
| Compression with smarter init | Weeks | True adaptive compute |

## Connection to soma-brain

soma-brain proved LTC + SDM at small scale (F1=0.291 SQuAD, 88M
params, 20K SDM entries from nomic-embed). soma-engram applies the
same architecture to richer features extracted from a 3B transformer.
Same philosophy. Different substrate. Different research questions.

## What this project actually demonstrates

A 3B transformer can be losslessly decomposed into (extracted weights,
ODE integration). The pieces of the four-paper synthesis hold
empirically — the math works, the chat works, the surgery is exact.

What this project does *not* yet demonstrate is **value over running
the original transformer**. The decompiled system reproduces the
transformer's output but doesn't yet outperform it on speed, memory,
or editability. The path to such advantages (sparse MLP, SSM, adaptive
depth) is mapped out above but not yet built.

This is honest research scaffolding, not a finished product.
