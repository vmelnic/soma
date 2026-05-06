# soma-engram — Architectural Invariants (Read First)

These are non-negotiable. Violations have happened before — re-read this
when in doubt instead of designing freely.

## What this project IS

Decompile a pretrained transformer into:
1. **Static SDM** = MLP weights as explicit (gate, up, down) entries.
   Lives in RAM/disk. Frozen. Editable.
2. **Liquid core** = small ODE-based continuous-time reasoning network.
   Trainable. Queries the SDM sparsely.

Distillation: small liquid core learns to navigate the big SDM by
matching teacher (Qwen) outputs on diverse text.

## What this project IS NOT

| Wrong drift | Why it's wrong |
|-------------|----------------|
| LTC = full transformer with SDM-MLP swap | Same compute as source. No win. |
| LTC uses standard Q/K/V softmax attention | Transformer attention is global O(N²) lookup. Not liquid. |
| LTC has surgery-copied weights from Qwen | Same parameter budget as source. Not "small". |
| Train on QA / SQuAD specifically | Distillation needs general distribution, not narrow tasks. |
| Knowledge in LTC weights | Knowledge belongs in SDM. LTC is the librarian, not the library. |
| Streaming the full transformer for big models | LTC stays small. SDM gets bigger. Streaming is a wrong-headed concept. |
| KV cache | Transformer-paradigm contamination. Real liquid is recurrent over sequence. |

## The four-paper synthesis (the basis)

| Equivalence | Source | Role |
|-------------|--------|------|
| Transformer = discretized Neural ODE in depth | Chen 2018 | LTC integrates the depth-time ODE continuously |
| Modern Hopfield = Attention | Ramsauer 2021 | Attention IS associative memory retrieval |
| MLP = Key-Value Memory | Geva 2021 | What we extract into SDM |
| Attention ≈ SDM | Bricken 2022 | The store is naturally sparse-distributed |

## What a real liquid core looks like

- **CfC** (Closed-Form Continuous-time, Hasani et al.) or **LTC cells** —
  state evolves via ODE-like update with learned time constants
- **Recurrent in sequence** — single state carries past, no token-pair
  attention, no KV cache
- **Recurrent in depth too** — same parameters reused across integration
  steps (or a small fixed set of slices), not 36 separate transformer
  blocks copied from the source
- **Adaptive compute** — solver decides number of integration steps per
  input, hard inputs get more

## SDM extension is the killer feature

The LTC stays small (~100-500M trainable). The SDM grows by:
1. Concatenating extractions from more transformers (Qwen-7B, Llama-70B, …)
2. Surgical edits (ROME/MEMIT-style direct (k,v) overwrite)
3. Ingesting documents into a parallel embedding-keyed store
4. TTT writes during inference

Adding knowledge is O(disk space). LTC training cost stays constant.
Impossible with a transformer (would require retraining all weights).

## Honest scaling math

24 tok/s on Qwen-3B fp16 on RTX 3090 is **GPU-memory-bandwidth-bound**.
Naive Python LTC (24 tok/s) ≈ HF native (17.7 tok/s). Hardware ceiling.

The advantage of this architecture is **NOT raw speed on small models**.
It's:
- Knowledge edit without retrain
- Small LTC + huge SDM (10T-equivalent on disk, fits on consumer hardware)
- Adaptive compute via ODE
- TTT during inference (real online learning)
- Domain-specific knowledge ingestion

If a discussion drifts toward "make Qwen-3B chat faster", redirect — that
is not the win.

## Files (what they ARE vs what they SHOULD BE)

| File | Currently | Should be |
|------|-----------|-----------|
| `ltc/core.py` | LiquidCore = surgery-copied 3B Qwen with `integrate()` | Reference reimplementation only — kept for verification, NOT the production architecture |
| `ltc/engram.py` | EngramStep = standard attention + SDM query | EngramStep = CfC/LTC cell + sparse SDM query, recurrent in sequence and depth |
| `ltc/surgery.py` | Copies Qwen weights → 3B LTC | Borrow only embeddings + final norm; DO NOT copy attention/MLP weights into the LTC |
| `chat.py` | Uses LiquidCore (surgery) — chats but is just Qwen | Should use EngramLTC after distillation |

## Working rules for this project

- **Reread this file when planning architecture changes.** It exists
  because drift happened.
- **Knowledge → SDM. Reasoning → LTC.** When deciding where a parameter
  goes, ask: is this a fact or a pattern? Facts go to SDM (or are
  retrieved from it). Patterns go to LTC.
- **No transformer attention in the LTC.** Use CfC, LTC cells, or
  state-space recurrence (Mamba-style). Not Q/K/V softmax.
- **Diverse text for distillation.** Generic corpus. Not task-specific
  data unless the task is the actual goal.
- **The LTC must be small.** Target: ~100-500M trainable. If parameter
  count creeps toward source-transformer size, the design is wrong.
