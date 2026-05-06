# soma-graft — Architectural Invariants

Grafting a frontier-class SDM (extracted from a 70B/120B model) onto a
small chat model (Qwen-3B). The small model handles fluency and speed;
the big SDM provides knowledge breadth.

## What this project IS

```
input → Qwen-3B (frozen, fast, fluent, ~2GB Q4 in VRAM)
            │
   layer N  │  hidden state h (Qwen's dim)
            ▼
      Bridge Adapter (small, trainable, ~20-50M params)
            │ project Qwen-dim → 70B-dim
            ▼
      Sparse top-k query into 70B SDM (frozen, on SSD/RAM, ~112GB)
            │ retrieve neural primitives (gate, up, down entries)
            ▼
      Bridge Adapter: project 70B-dim → Qwen-dim
            │
   layer N  │  h + retrieval (added to Qwen's residual stream)
            ▼
input → Qwen continues → output logits
```

Components:
- **Qwen-3B**: frozen, the actual chat model. Provides fluency, vocab,
  sequence dynamics. Fast inference via llama.cpp / HF.
- **70B SDM**: extracted MLP weights (gate, up, down) from a frontier
  teacher. Frozen, lives on SSD, mmap'd, queried sparsely.
- **Bridge adapter**: small trainable module that connects Qwen's
  hidden states to the bigger SDM's space. The ONLY thing trained.

## Hardware target (consumer machine)

- Qwen-3B Q4: ~2GB VRAM
- Bridge: ~100MB VRAM
- KV cache + activations: ~1GB VRAM
- 70B SDM: ~112GB on SSD, hot 1-5GB cached in RAM
- Total VRAM: ~3GB on a 24GB card → huge headroom for bigger Qwen
- Total RAM: 5-15GB working set

## Speed target

- Qwen-3B Q4 baseline: ~50 tok/s (llama.cpp)
- SDM lookup overhead: 1-15ms per token depending on cache
- Steady-state with warm cache: **25-40 tok/s**
- Cold / SDM thrashing: 5-15 tok/s

## What this project IS NOT

- Not training a chat model from scratch
- Not distilling Qwen into anything (frozen as-is)
- Not retraining Qwen
- Not a transformer replacement — Qwen IS the transformer, this just
  augments it with extra knowledge
- Not pure RAG — retrieval happens INSIDE the forward pass, at neural
  primitives level (not text-prepend)

## What is honestly RAG-adjacent and what's novel

**Functionally similar to RAG**: small fluent model + external knowledge.

**Genuinely different from RAG**:
- Retrieves neural primitives (k,v MLP entries), not text chunks
- Retrieval blends into hidden states mid-forward-pass, not at input
- Knowledge editable at primitive level (overwrite single (k,v) to
  change a fact — RAG can't do this on a vector DB)
- Multi-source: stack SDMs from many teachers, query unified
- No re-encoding of retrievals (pre-computed MLP outputs)

Honest framing: "RAG with surgical knowledge editing and multi-teacher
neural-primitive fusion."

## Use cases

| Use case | Value |
|----------|-------|
| Local coding assistant | Qwen-3B speed + 70B-coder knowledge + your codebase in episodic SDM |
| Editable expert system | Surgical fact edits — instant updates without retrain |
| Multi-domain assistant | SDMs from coder + medical + legal teachers combined |
| Privacy-preserving expert | All on-device, 70B-class knowledge on a 3090 |
| Continuously-updating knowledge | Episodic SDM grows from interactions |

## Phases

| Phase | What | Status |
|-------|------|--------|
| 1 | Extract teacher SDM (Llama-3-70B or Qwen2.5-72B) | not started |
| 2 | Bridge module (dim projection + sparse SDM query + blend) | not started |
| 3 | Train bridge on a small corpus where teacher knowledge helps | not started |
| 4 | Inference path: Qwen-3B + bridge + SDM, measured tok/s | not started |
| 5 | Editable knowledge demo + multi-source SDM + episodic growth | not started |

## Working rules

- Read this file before architectural changes.
- The bridge is the ONLY trainable component. Qwen frozen, SDM frozen.
- If a change creeps toward "retrain Qwen" or "build new chat model"
  — stop, that's drift.
- Speed and editability are the wins. Not "matches GPT-4".
