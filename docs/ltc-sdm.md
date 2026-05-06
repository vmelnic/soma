# Transformer Decompilation: SDM + LTC

Surgically decompose a pretrained transformer into two complementary
parts:
- **Static Sparse Distributed Memory (SDM)** holding all the model's
  knowledge as explicit (key, value) pairs extracted from MLP layers.
- **Liquid Time-Constant (LTC) core** running continuous-time dynamics
  derived from the transformer's attention mechanism.

The original transformer is discarded after extraction and a brief
trajectory-distillation phase. The resulting (SDM, LTC) system
reproduces the transformer's behavior with adaptive compute, in-RAM
knowledge, surgically editable memory, test-time learning, and no
GPU dependency at inference.

## The core insight

Bricken et al. (NeurIPS 2022) proved that **transformer attention
approximates Sparse Distributed Memory**. Every transformer is already
an SDM. A 7B model is an SDM with billions of learned patterns. A
1T model is an SDM with hundreds of billions.

The implication, sitting unused in the literature for four years:
**we don't need to build SDM separately. We can use any pretrained
transformer AS the SDM.**

What's been missing: a reasoning agent that knows how to query this
existing SDM efficiently. That's the LTC core.

## What this is NOT

**Not RAG.** RAG retrieves stored text and feeds it back as input
to a transformer that then generates. Here, the transformer never
generates and the LTC never sees text — it queries the transformer's
internal hidden states and reasons over them directly.

**Not retrieval-augmented training.** The transformer is fully frozen.
We don't fine-tune it, don't add adapters to it, don't change its
weights. The only learning happens in the LTC.

**Not knowledge distillation.** We're not compressing the transformer
into a small model. The transformer stays at full size. We're building
an agent that uses it as a tool.

**Not a hybrid.** "Hybrid" suggests two systems doing the same job
differently. Here the roles are completely separate: transformer
stores, LTC reasons. Like a hippocampus and a textbook — different
organs, complementary functions.

## The architecture

```
   ┌────────────────────────────────────────────────────────┐
   │                                                        │
   │   ┌─────────────────────────────────────────────────┐  │
   │   │ FROZEN TRANSFORMER (knowledge substrate)        │  │
   │   │                                                 │  │
   │   │   L0 ── L8 ── L16 ── L24 ── L32 ── L48 ── L80   │  │
   │   │    ↑     ↑     ↑      ↑      ↑      ↑     ↑    │  │
   │   │    │     │     │      │      │      │     │    │  │
   │   │  read/write taps (learned projections)          │  │
   │   │    │     │     │      │      │      │     │    │  │
   │   └────┼─────┼─────┼──────┼──────┼──────┼─────┼────┘  │
   │        │     │     │      │      │      │     │       │
   │   ┌────▼─────▼─────▼──────▼──────▼──────▼─────▼────┐  │
   │   │ LIQUID CORE (LTC, 100M-1B trainable)           │  │
   │   │                                                │  │
   │   │  ODE dynamics dx/dt = f(x, query, retrieval)   │  │
   │   │  TTT: weights update during inference          │  │
   │   │  Free energy minimization across queries       │  │
   │   │                                                │  │
   │   │  Decides per timestep:                         │  │
   │   │   - which layer to query                       │  │
   │   │   - what query vector to send                  │  │
   │   │   - when state has converged (stop)            │  │
   │   │   - what to commit to working memory           │  │
   │   └────────────────────────────────────────────────┘  │
   │                                                        │
   └────────────────────────────────────────────────────────┘
```

### Components

**1. Frozen substrate** — any pretrained transformer. The LTC sees
its layers as a stack of content-addressable memories at different
abstraction levels:

- Early layers (0-8): syntax, lexical patterns, surface features
- Middle layers (16-24): factual associations, entity relationships
- Late layers (32+): task-relevant abstractions, semantic compositions

The LTC learns which depth to query for which kind of question.

**2. Read/write taps** — small learned projection layers at chosen
depths. Each tap has:
- Query projection: maps LTC state → transformer-input-space
- Read projection: maps transformer-hidden-state → LTC state
- Write projection (optional): maps LTC state → transformer-hidden-state
  for steered forward passes

Roughly 5M params per tap. With 5-7 taps: ~30-40M total.

**3. Liquid core** — ODE-based reasoning engine. State evolves
continuously. At each integration step, the core can issue a query
to the substrate and integrate the response into its dynamics.

Adaptive compute: the ODE solver takes more steps when the state
hasn't converged. Easy questions converge fast (one query). Hard
questions iterate (query, integrate, query again, integrate, …).

**4. TTT layer** — gradient updates within the LTC during inference.
The substrate stays frozen, but the LTC's weights update from each
query/response interaction. Within a session, the LTC gets better at
querying for the current task.

**5. Output heads** — task-specific projections from final LTC state:
span boundaries, classification logits, scalar values, structured
fields. No autoregressive generation through the transformer.

## How a query works

```
1. Input embedding x₀ enters LTC
2. LTC initializes state h₀
3. LTC ODE step: dh/dt = f(h, retrieved_context)
4. LTC produces query q via query_projection(h)
5. Query feeds into transformer at chosen layer L
6. Transformer runs forward from L onward (or just L's activations)
7. Hidden state at chosen depth → read_projection → LTC retrieval
8. LTC integrates retrieval into h
9. If |dh/dt| < threshold: converged, stop
   Else: GOTO 3, possibly query different layer
10. Final h → output head → answer
```

The transformer is never asked to generate tokens. It's asked
"given this query, what's at layer L?" The LTC composes the answer
from many such layer-reads.

## Substrate scaling: 7B to 1T

The substrate is frozen. We don't train it. We don't store gradients
for it. We only need inference. This changes the hardware math
completely.

| Substrate | fp16 | int8 | int4 | Hardware |
|-----------|------|------|------|----------|
| Qwen-7B | 14GB | 7GB | 3.5GB | RTX 3090 (24GB) easily |
| Llama-70B | 140GB | 70GB | 35GB | RTX 3090 with offload, or 2x A6000 |
| Mistral-123B | 246GB | 123GB | 62GB | A100 80GB + offload |
| Qwen-110B | 220GB | 110GB | 55GB | A100 80GB + offload |
| Llama-405B | 810GB | 405GB | 200GB | 4-8x H100, or 1x H100 + heavy offload |
| DeepSeek-V3 (671B MoE) | 1.3TB | 670GB | 336GB | 8x H100, or aggressive offload |
| Hypothetical 1T dense | 2TB | 1TB | 500GB | Multi-node only at fp16 |
| Hypothetical 1T MoE (50B active) | 100GB active | 50GB | 25GB | Single H100 if expert-routed |

### Why MoE substrates are ideal

DeepSeek-V3 has 671B parameters but only 37B activate per token.
The model is already organized as **content-addressable expert
modules** — exactly the SDM property we want.

For an LTC navigating MoE:
- Only the experts the LTC routes to need to be in VRAM
- Other experts can stay on CPU/disk
- LTC learns expert selection as part of its query policy
- Inference cost scales with **expert activation count**, not total
  parameter count

A 1T-parameter MoE substrate with 100B active parameters per query
is more practical than a 70B dense model. The total knowledge stored
is 14x larger but the active compute is similar.

### Aggressive quantization

The substrate is read-only at inference. Quantization error is bounded
and predictable. Recent work (BitNet, AQLM, QuIP#) shows int4 and even
int2 quantization preserve most knowledge for retrieval-style use:

- The LTC reads representations, not exact logits
- The LTC's TTT can compensate for substrate quantization noise
- We don't need the substrate to generate fluently — only to encode

A 1T MoE at int4 with selective expert loading: ~50-100GB active
working set. Runs on a single H100 with CPU offload for cold experts.
Affordable to rent for training runs ($2-4/hour on Lambda/RunPod).

### Hierarchical substrates

The LTC can query multiple substrates of different sizes:

```
LTC
 ├── always: Qwen-7B (local, fast, broad coverage)
 ├── on hard queries: Llama-405B (cloud, slow, deep knowledge)
 └── on specialized queries: domain-specific 70B (cloud, niche)
```

The LTC learns the **cost/benefit** of each substrate. Cheap query
first, escalate only when needed. Free energy minimization gives a
principled criterion: keep querying until information gain per unit
cost drops below threshold.

This is how a brain works. Cortex routes queries to specialized
regions based on need. The LTC plays the role of routing/integration.

## Transformer decompilation

The LTC is not trained from scratch. It is **surgically derived** from
the original transformer using four published equivalences nobody has
combined:

| Equivalence | Source | Role |
|-------------|--------|------|
| Transformer = discretized Neural ODE | Chen et al. (2018) | Layers ARE samples of an ODE trajectory |
| Modern Hopfield = Attention | Ramsauer et al. (2021) | Attention IS associative memory retrieval |
| MLP = Key-Value Memory | Geva et al. (2021) | MLPs ARE explicit (k,v) stores |
| Attention ≈ SDM | Bricken et al. (2022) | Attention IS sparse distributed memory |

A transformer is therefore: **(static SDM) + (continuous-time dynamics
that retrieves from it)**. The dynamics happen to be encoded as
discrete layers + softmax attention, but those are implementation
choices. The same computation can be re-expressed as an ODE-driven
LTC navigating an explicit SDM.

This is decompilation: convert the compiled binary form (matrix
multiplications baked into a forward pass) back into its source form
(LTC ODE + SDM lookup tables) where the source is editable and
adaptive.

### Surgery operation 1: Extract SDM from MLPs

A SwiGLU MLP block is `down · (silu(gate · x) * (up · x))`. Each
intermediate dimension is one (k_gate, k_up, value) triplet. Extracting
all 36 MLP layers from Qwen-3B yields ~400K explicit entries — the
static knowledge of the model in addressable form. Nothing about this
step requires training.

### Surgery operation 2: Derive LTC dynamics from attention

A multi-head attention layer computes `softmax(QK^T)V`. Modern Hopfield
networks proved this is energy-minimization retrieval over stored
patterns {K, V}. An LTC integrating `dh/dt = kernel(h, K) · V - h`
with the right kernel produces the same fixed-point behavior in
continuous time.

The LTC's recurrent dynamics are therefore not learned from scratch —
they are the continuous-time form of attention. Initialize:
- LTC's K_proj ← averaged W_q matrices across transformer heads
- LTC's V_proj ← averaged W_v matrices
- Kernel function ← softmax (start) → learnable (refine)
- Time constant τ ← 1 layer per ODE step (one Euler step matches one
  transformer layer)

The LTC starts already partially competent. Training fine-tunes, not
builds from zero.

### Surgery operation 3: Trajectory distillation (not output distillation)

A transformer running on input x produces a sequence of hidden states
[h_0, h_1, ..., h_L] across its layers. Standard distillation matches
only the final h_L. Trajectory distillation matches **every** hidden
state — the LTC learns the full internal computation, not just the
conclusion.

Loss for one input:
```
L_traj(x) = Σ_l || LTC(x, t=l/L) - h_l(x) ||²
```

Where the LTC is integrated from t=0 to t=1 with checkpoints at l/L
that should match h_l. This is solving an ODE-fitting problem on a
known trajectory — well-studied (Neural ODE distillation), with
guaranteed convergence under standard conditions.

### Surgery operation 4: TTT runs in continuous time

The LTC's weights are small (~100M-500M). TTT updates apply during
inference as gradient steps on the LTC's vector field f, conditioned
on the current input and free energy minimization. The substrate SDM
stays frozen. Only the dynamics adapt.

### What this changes vs naive distillation

| | Output distillation | Surgery + trajectory distillation |
|---|---------------------|----------------------------------|
| LTC init | random | derived from transformer weights |
| Training signal | one target per input (final hidden state) | L targets per input (every layer's hidden state) |
| Attention mechanism | learned from scratch | structurally derived from softmax attention |
| Theoretical basis | empirical | Neural ODE + Hopfield + Geva + Bricken |
| Convergence time | days | hours |
| Lossy reconstruction | likely | bounded by Neural ODE fitting error |

### Why the four-paper synthesis matters

Each cited result on its own is interesting but limited:
- Neural ODE (2018): cool, but fitting one to a transformer was never
  attempted at scale because nobody saw the point.
- Hopfield = Attention (2021): published, then sat unused. Implication
  ignored.
- MLP = KV Memory (2021): used for interpretability, not for replacement.
- Attention = SDM (2022): used to argue transformers are "doing SDM",
  not to extract them as such.

The synthesis: **all four equivalences hold simultaneously**, so a
transformer is fully decomposable into (SDM substrate, ODE dynamics).
We're not proposing new math — we're combining four known equivalences
into a complete decompilation pipeline.

## Training pipeline (post-surgery)

Three phases. The original transformer is needed only during phase 1
(extraction) and phase 3 (trajectory distillation, as oracle). After
phase 3, the transformer is discarded.

### Phase 1: Surgical extraction (hours)

- Extract MLPs → SDM (deterministic, no training)
- Initialize LTC weights from averaged transformer block weights
- Initialize LTC kernel from softmax attention form
- No gradient updates yet

### Phase 2: Trajectory distillation (hours-days)

- Run text corpus through original transformer once, cache full
  hidden-state trajectories
- Train LTC to interpolate trajectories continuously
- Loss: per-layer MSE on hidden states + free energy regularizer
- Trainable: ~100M-500M params (LTC vector field + read/write taps)
- Substrate (extracted SDM) stays static
- Single GPU sufficient

### Phase 3: Task fine-tuning (hours)

- Replace per-layer MSE loss with task-specific loss (span CE, etc.)
- Add output heads for the target task
- LTC adapts dynamics to optimize task performance, not just imitation
- The student can become better than the teacher on specialized tasks

### Phase 4: TTT calibration (hours)

- Train TTT layer on multi-session clusters
- LTC adapts within sessions via online gradient on its own vector field
- Substrate stays frozen forever — only LTC dynamics evolve

## Why this works

### The substrate is doing the hardest part for free

A 1T model encodes language fluency, world knowledge, reasoning
patterns, code syntax, mathematical structure, multi-modal alignment
— everything pretraining produces. Someone spent $100M training it.
We get all of that as a frozen oracle.

The LTC only needs to learn **how to ask the right questions**. That's
a much smaller problem than learning everything from scratch.

### ODE dynamics give adaptive compute

A standard transformer spends identical compute on "what is 2+2" and
"prove the Riemann hypothesis." The LTC spends compute proportional
to problem difficulty. Trivial queries: one substrate read, one ODE
step, done. Hard queries: many reads at varying depths, many ODE
steps until state converges.

### TTT enables continual learning without modifying the substrate

The LTC's TTT updates apply only to the LTC's own ~100M parameters.
The 7B/70B/1T substrate stays frozen forever. Yet the system as a
whole learns from each session, because the LTC's ability to query
the substrate improves over time.

This is impossible in a pure transformer. Any learning would require
modifying the transformer's weights, breaking the frozen substrate
property and risking catastrophic forgetting.

### Free energy is the right objective

Cross-entropy on tokens trains for fluency. Free energy on
representations trains for **understanding**: minimize prediction
error in a continuous latent space, encode only surprises, pay
attention only to information.

The LTC's free energy loss naturally implements:
- Active inference: prefer queries that maximally reduce uncertainty
- Bayesian model reduction: consolidate redundant patterns into TTT
- Predictive coding: represent the world as deviations from expectation

These are the principles biological brains run on. Transformers don't
have access to them because their architecture doesn't support
continuous latent dynamics.

## Why nobody has done this

1. **Three communities don't talk.** Liquid networks (Hasani, MIT),
   memory architectures (DeepMind, Meta), active inference (Friston,
   computational neuroscience). The combination requires fluency in
   all three.

2. **The Bricken result was filed under "interesting curiosity."**
   Showing attention=SDM in 2022 should have prompted "great, now
   build the LTC that uses it." Nobody did. The implication was
   missed.

3. **Training a frozen oracle is unfamiliar.** Standard ML trains the
   model that produces outputs. Here we train an agent that USES a
   model. Different optimization landscape, different debugging
   intuitions, no off-the-shelf tooling.

4. **Doesn't fit the scaling narrative.** The story "scale = win" gets
   funded. The story "small reasoner queries big oracle" doesn't fit
   the slide deck. Investors don't know how to value it.

5. **The hardest open models only got big enough recently.** A 1T
   open MoE didn't exist in 2022. Now DeepSeek-V3 makes the substrate
   approach viable on rentable hardware.

6. **TTT is barely two years old.** Sun et al. (2024) introduced TTT
   for transformers. Combining it with ODE cores and frozen substrates
   needed all the pieces to mature first.

## Practical first build

Start small to prove the idea, then scale substrate.

### Step 1: LTC + Qwen-7B substrate

- Substrate: Qwen2.5-7B (already on RTX 3090)
- LTC: 200M params, 5 layer taps at depths 8, 14, 20, 26, 32
- Task: SQuAD 2.0 extractive QA + yes/no head
- Training: single GPU, days
- Goal: prove LTC + frozen transformer outperforms LTC + nomic
  embeddings (current soma-brain F1=0.291)

### Step 2: Scale substrate to 70B

- Substrate: Llama-3-70B at int4 (~35GB)
- Same LTC, retrained
- Task: same QA + add code generation evaluation
- Hardware: 2x RTX 3090 or 1x A6000 48GB, or rented H100
- Goal: demonstrate substrate scaling improves results without LTC
  retraining

### Step 3: Scale substrate to 671B MoE

- Substrate: DeepSeek-V3 671B at int4 (~336GB)
- LTC adds expert-selection head
- Task: full reasoning suite (MMLU, GSM8K, HumanEval)
- Hardware: rented 8x H100 cluster, training in days
- Goal: match or exceed DeepSeek-V3's standalone scores at fraction
  of inference cost (LTC routes to fewer experts than autoregressive
  generation)

### Step 4: Multi-substrate routing

- Local 7B + cloud 70B + cloud 405B
- LTC learns cost-aware substrate routing
- Tasks vary in difficulty; LTC scales compute appropriately
- Hardware: hybrid local/cloud
- Goal: production-grade system, consumer GPU primary, cloud burst
  for hard queries

### Step 5: Transformer surgery — extract substrate to explicit SDM

The endgame. Stop running transformers entirely. Convert their weights
into pure addressable memory.

**The mathematical basis:** Geva et al. (2021) showed transformer MLP
layers are literally key-value memories. Bricken et al. (2022) proved
attention approximates SDM. Therefore the entire transformer is
*already* a content-addressable memory — we just need to unroll it
into explicit form.

**MLP extraction (clean):**

Each MLP layer `y = W_down · σ(W_up · x + b_up) + b_down` becomes
a SDM where:
- Keys = rows of `W_up` (with their biases)
- Values = rows of `W_down`
- Activation σ provides natural sparsity (only top-k keys fire)

For each MLP: `4 × hidden_size` (k,v) entries. For a 1T MoE with
all experts unrolled: hundreds of billions of explicit entries.
Lossless (mod quantization).

**Attention extraction (relational):**

Each attention head's `W_q · W_k^T` defines a learned bilinear form
over token pairs. Extract as a relational SDM where each entry is
a (query_template, key_template, value_pattern) triple. The LTC
queries by composing entity embeddings into both query and key
positions.

Lossy but covers most attention knowledge. The residual-stream
interaction effects are lost — those would need to be relearned
inside the LTC during phase-3 training.

**Layer-interaction effects (recovered via TTT):**

Whatever capability emerges from cross-layer integration in the
running transformer must be reconstructed by the LTC's own dynamics.
TTT enables this: the LTC sees inputs, queries the extracted SDM,
observes prediction errors against ground truth, adjusts its own
weights to compose retrievals correctly. The dynamics that the
transformer's residual stream provided implicitly become the LTC's
ODE trajectory explicitly.

**Storage and access:**

| Source model | Extracted SDM (int4) | Hot working set | Storage |
|--------------|---------------------|-----------------|---------|
| Qwen-7B | ~3.5GB | ~50MB | RAM |
| Llama-70B | ~35GB | ~200MB | RAM |
| Llama-405B | ~200GB | ~500MB | NVMe + RAM cache |
| DeepSeek-V3 671B MoE | ~336GB | ~1GB | NVMe + RAM cache |
| Hypothetical 1T dense | ~500GB | ~2GB | NVMe array |
| Hypothetical 1T MoE | ~500GB | ~500MB | NVMe + RAM cache |

mmap the SDM into virtual memory, page in entries on demand. Per
query, only ~0.1-0.5% of entries are accessed. The OS handles
caching naturally — frequently queried regions stay hot in RAM.

**What this enables:**

- **No GPU at runtime.** SDM lookup is memory-bound, not compute-bound.
  A consumer machine with 32GB RAM and a 1TB SSD runs a system that
  required 8x H100 to run as a transformer.
- **Surgical edits.** Wrong fact at SDM[42, 1337]? Overwrite that
  entry. No retraining, no fine-tuning. Direct memory edit.
- **SDM merging.** Extract two specialized 70B models, combine their
  SDMs into one. The LTC queries the merged store and uses both
  knowledge bases.
- **Differential extraction.** Extract domain-specific subsets — only
  MLP entries that fire on code inputs. Get a "code SDM" that's 10x
  smaller than the full extraction but covers the relevant capability.
- **Unbounded context.** Extracted SDM has no sequence length. The LTC
  can query the full knowledge base for every reasoning step regardless
  of how much "context" has accumulated.

**Why nobody has done this:**

1. The Geva and Bricken results are interpretability research — they
   were used to *understand* transformers, not to *replace* them.
2. Building extraction tooling requires careful linear algebra and
   handling of layer norms, RMS norms, attention bias, RoPE, etc.
   Complex but bounded engineering work.
3. The community optimizes transformers for inference (vLLM, TGI,
   exLlama) rather than considering "what if we just don't run the
   transformer." The assumption that the transformer must run at
   inference is so deep nobody questions it.
4. The Bricken paper's implication — that you can replace the attention
   mechanism with explicit SDM lookup — has been sitting unused for
   four years.

**Concrete first extraction:**

- Pick Qwen-7B (small enough to validate, large enough to be useful)
- Extract all 32 MLP layers → 32 × 16384 = ~524K (k,v) pairs
- Extract attention heads as relational templates
- Build SDM lookup with hierarchical index (layer → MLP slot → top-k)
- Verify: feed inputs through the extracted SDM (without running Qwen)
  and through original Qwen, compare hidden states at each layer
- Acceptance: <5% RMSE between extracted and original at all layers
  on a probe set
- Result: Qwen-7B as a 3.5GB on-disk SDM, queryable without GPU

**Then train LTC on top:**

The phase-2 LTC training repeats with the extracted SDM as substrate
instead of the running transformer. If the extraction is faithful, the
LTC's downstream task performance should match the frozen-substrate
version. If not, identify the gap and refine the extraction.

**Then scale:**

- 70B → 35GB SDM, runs on a workstation
- 405B → 200GB SDM, NVMe-resident
- 671B MoE → 336GB SDM, single-node consumer hardware feasible
- 1T → 500GB SDM, still fits on a high-end consumer NVMe

The frontier model becomes a one-time download. Everyone with a
beefy SSD and 32GB RAM owns the equivalent of frontier inference,
without GPU, without API calls, without per-query cost.

## What success looks like

1. **88M LTC + 7B frozen substrate** matches or exceeds **70B
   standalone** on knowledge tasks. Validates the hypothesis that
   transformer layers ARE usable as SDM.

2. **200M LTC + 671B MoE substrate** approaches **frontier model
   capabilities** on reasoning tasks while running on rentable
   hardware. Validates the scaling story.

3. **TTT delta is positive** on multi-question sessions. LTC actually
   learns from interaction, not just from offline training.

4. **Inference cost scales with problem difficulty** measurably.
   Easy questions cost <100ms (one ODE step, one substrate read).
   Hard questions cost <2s (many ODE steps, multiple reads at
   different depths). Standard transformers cost the same regardless.

5. **Extraction validates losslessly.** Qwen-7B → 3.5GB extracted SDM
   reproduces original layer activations within <5% RMSE on probe
   inputs. No GPU required to query the extracted SDM.

6. **Extracted-substrate LTC matches frozen-substrate LTC.** Phase-2
   LTC trained on extracted SDM achieves equivalent task performance
   to phase-2 LTC trained on running transformer. Proves extraction
   is information-preserving for the LTC's purposes.

7. **GPU-free frontier inference.** A 671B MoE extracted to ~336GB
   SDM runs on a consumer workstation (32GB RAM + 1TB NVMe) with
   per-query latency under 5 seconds. The frontier model becomes a
   one-time download instead of a $100K cluster dependency.

## Connection to soma-brain

soma-brain (current) is the same architecture with two restrictions
that this design removes:

| Property | soma-brain (current) | Transformer-as-substrate |
|----------|---------------------|--------------------------|
| SDM contents | nomic-embed surface vectors (768d) | Frozen transformer layer activations (4096d) |
| Knowledge depth | What we ingest | Whatever the transformer learned |
| Language fluency | None — we'd need to add a decoder | Inherited from substrate |
| Substrate cost | Nothing (we built it) | Frozen pretrained model (free if open) |
| Trainable params | 88M | 100M-1B (LTC + taps + heads) |
| F1 on SQuAD | 0.291 | Expected significantly higher (richer features) |

This is the same liquid-core philosophy applied to a far richer
memory substrate. The soma-brain SDM contains 20K passage embeddings.
A 1T frozen transformer contains hundreds of billions of learned
patterns spanning all of human-recorded knowledge.

The math doesn't change. The substrate gets vastly richer.

## Open research questions

1. **Optimal tap depths**: where to read? Probably learnable per
   query — the LTC chooses depth as part of its policy.

2. **Quantization sensitivity**: how much int4/int2 noise can the
   LTC tolerate before TTT can't compensate?

3. **MoE expert routing**: can the LTC learn to bypass MoE's gating
   network and route directly to experts? Would save the gating
   compute entirely.

4. **Substrate switching**: can a single LTC trained on Qwen-7B
   generalize to Llama-70B with only tap recalibration? If yes, the
   LTC is genuinely substrate-agnostic.

5. **Free energy at scale**: does free energy minimization remain
   stable when querying a 1T substrate? Or does the high-dimensional
   readout space cause numerical issues?

6. **Generation without autoregression**: can the LTC produce fluent
   long-form text via iterative substrate queries, never running
   autoregressive decoding? If yes, this is a fundamentally new
   generation paradigm.

## References

Bricken, T. et al. (2022). Attention Approximates Sparse Distributed
Memory. *NeurIPS 2022*.

Hasani, R. et al. (2021). Liquid Time-constant Networks. *AAAI 2021*.

Sun, Y. et al. (2024). Learning to (Learn at Test Time): RNNs with
Expressive Hidden States. *arXiv:2407.04620*.

Ali, M. et al. (2024). Titans: Learning to Memorize at Test Time.
*arXiv:2501.00663*.

Friston, K. (2010). The free-energy principle: a unified brain theory?
*Nature Reviews Neuroscience*.

DeepSeek-AI (2024). DeepSeek-V3 Technical Report.

Meng, K. et al. (2022). Locating and Editing Factual Associations in
GPT. *NeurIPS 2022*.

Borgeaud, S. et al. (2022). Improving Language Models by Retrieving
from Trillions of Tokens (RETRO). *ICML 2022*.

Tay, Y. et al. (2024). Mixture-of-Experts Meets Instruction Tuning
(Switch / OLMoE / DeepSeek MoE foundations).
