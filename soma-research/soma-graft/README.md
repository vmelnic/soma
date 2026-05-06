# soma-graft

Graft a large model's knowledge onto a small, fast chat model. The small
model (Qwen-3B) stays frozen and handles fluency. MLP weights extracted
from a larger model (Qwen-32B) serve as a Sparse Distributed Memory,
queried mid-forward-pass via a small trainable bridge adapter.

## Architecture

```
input tokens
    |
Qwen-3B (frozen)  layers 0..11
    |
    +--- Bridge: norm -> proj_in (2048->5120) -> sparse top-k MLP query -> proj_out -> gate
    |
Qwen-3B  layers 13..17
    |
    +--- Bridge injection
    |
Qwen-3B  layers 19..23
    |
    +--- Bridge injection
    |
Qwen-3B  layers 25..35
    |
    +--- Bridge injection
    |
output logits
```

Three components:
- **Qwen-3B** (frozen): fluency, vocabulary, sequence dynamics
- **SDM** (frozen): MLP weights extracted from Qwen-32B (gate/up/down per layer, 64 layers x 27648 intermediate dim, ~53GB on disk)
- **Bridge** (trainable, ~84M params): projects between hidden dimensions, queries SDM sparsely, blends results via learned gates

## Status

| Phase | Status |
|-------|--------|
| Extract Qwen-32B MLPs to SDM | Done (`qwen32b_sdm.pt`, 53.5GB) |
| Bridge module | Done (4 injection points, top-k=128) |
| Train bridge (2000 steps, 1K wiki samples) | Done |
| Eval: vanilla vs bridge LM loss | Done — **bridge hurts (+0.215 CE)** |
| Retrain with fixes | Pending |

### First eval results (2026-05-04)

```
vanilla Qwen-3B:     ce = 2.1938
Qwen-3B + bridge:    ce = 2.4091
delta (b - v):       +0.2153  (bridge HURTS)
per-sample wins:     4/50 samples improved with bridge
```

### Diagnosis

The 2048-to-5120 linear projection is the bottleneck. Qwen-3B's hidden
states don't contain the information that Qwen-32B's MLPs expect. A
linear map can't reconstruct features that were never computed. The SDM
returns noise; the gate opens; loss increases.

### Next steps

1. Extract SDM from Qwen-7B (d=3584, closer to Qwen-3B's d=2048)
2. Retrain bridge with the smaller dimension gap
3. Scale training: 100K+ samples, 50K+ steps

## Usage

Requires a machine with GPU (RTX 3090 24GB tested).

```bash
# 1. Extract MLP weights from teacher model
python extract.py --model Qwen/Qwen2.5-7B --out checkpoints/qwen7b_sdm.pt

# 2. Train bridge (only bridge params are trainable)
python distill.py --sdm checkpoints/qwen7b_sdm.pt \
    --corpus-size 10000 --steps 5000 --max-len 256

# 3. Evaluate: compare LM loss with and without bridge
python eval_bridge.py --sdm checkpoints/qwen7b_sdm.pt
```

## Files

```
extract.py          Extract MLP weights from any HF transformer
distill.py          Train bridge via self-supervised LM loss
eval_bridge.py      Compare vanilla vs grafted LM loss
bridge/
  bridge.py         Bridge adapter + GraftedQwen hook wrapper
  __init__.py
checkpoints/
  bridge.pt         Trained bridge weights (~160MB)
  qwen32b_sdm.pt    Extracted Qwen-32B MLPs (~53.5GB, Windows only)
```

## How it differs from RAG

- Retrieves neural primitives (MLP gate/up/down entries), not text chunks
- Blends into hidden states mid-forward-pass, not prepended to input
- Knowledge editable at the weight level (overwrite one MLP entry to change a fact)
- No re-encoding — MLP outputs are pre-computed
- Multi-source: stack SDMs from multiple teachers

## Hardware

- Qwen-3B fp16: ~6GB VRAM
- Bridge: ~160MB VRAM
- SDM: memory-mapped from SSD, ~900MB per layer copied to GPU on query
- Training fits in 24GB with batch_size=1
