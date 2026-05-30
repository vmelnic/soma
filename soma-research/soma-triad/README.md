# soma-triad — LTC + HDC + SDM

Non-transformer reasoning architecture. No weights in knowledge or composition — only in a tiny controller that sequences operations.

## The thesis

Transformers conflate three functions into one weight matrix: knowledge storage, compositional reasoning, and control flow. Separate them:

| Component | Function | Parameters | Mechanism |
|---|---|---|---|
| **SDM** | Knowledge store | 0 | Content-addressable vectors in RAM. Write episodes, read by cosine similarity. Generalization via address overlap. |
| **HDC** | Compositional algebra | 0 | Bind/unbind/bundle/permute over 10,000-dim bipolar vectors. Role-filler structures, sequence encoding, analogy. |
| **LTC** | Controller | ~2.6M | ODE-based cell that decides which operation to perform next. The only trainable piece. |

## What's proven (29/29 tests pass)

### HDC algebra (10 tests)
- Random vectors are quasi-orthogonal (blessing of dimensionality)
- Bind is invertible and dissimilar to inputs
- Bundle preserves similarity to all inputs
- Permute encodes order (reversible, dissimilar)
- Record encoding: bind(role, filler) pairs bundled → queryable by role
- Analogy: unbind a bundled relationship → retrieves correct filler
- Codebook is deterministic from seed

### SDM store (6 tests)
- Write/read roundtrip by cosine similarity
- Similarity ordering (nearest neighbors first)
- Reinforcement via running average (same label strengthens)
- Blended read interpolates between nearby entries
- Decay removes weak entries (natural selection)
- Empty read returns nothing (no hallucination)

### Triad orchestration (6 tests)
- Store and retrieve episodes across domains
- Controller emits operation signals (untrained — random but valid)
- Controller is tiny (~2.6M params for dim=10000, hidden=128)
- Multi-domain discrimination (code vs kitchen episodes)
- HDC has zero learnable parameters (confirmed)
- SDM has zero learnable parameters (confirmed)

### Axis construct selection (7 tests)
- CRUD pattern generalizes to unseen entities ("article", "location")
- Auth patterns retrieved correctly (not confused with CRUD)
- Real-time/stream patterns retrieved correctly
- Service integration patterns retrieved correctly
- Policy patterns retrieved correctly
- **Full backend composition**: 5-intent booking system spec maps each intent to correct pattern type with >0.3 similarity
- **Novel fields generalize**: completely new field compositions still match the right structural pattern

## Architecture

```
Structured spec ─┐
                 ├─→ HDC.encode_record() ─→ SDM.read() ─→ matched pattern
Natural lang ────┘                              ↑
                                                │
                            Controller decides: re-query? decompose? emit?
```

For structured input (intent already decomposed), SDM+HDC alone work — zero training needed.

For unstructured input ("handle user avatar upload"), the controller sequences: encode → query → low similarity → unbind into parts → re-query each part → bundle results → emit.

## Open issues

### 1. Controller training signal (unsolved)

The LTC controller needs to learn decomposition: "when SDM returns low similarity, try unbinding the query into sub-queries." Training data = decomposition traces (novel intent → sub-intents → retrievals → assembly). Options:

- Hand-author 50-100 traces (like DNA — bootstrap)
- Use an LLM once to generate traces, store in SDM, controller replays the pattern
- Record from structured-input usage, then learn to handle unstructured

**Decision needed:** which bootstrapping path?

### 2. Natural language input (not addressed)

Current proof uses structured dicts: `{"intent": "crud", "entity": "user", ...}`. Real users type English. Something must parse "handle user avatar upload" into structured intents. Options:

- Small fine-tuned model (1B) just for intent extraction (not reasoning)
- Rule-based parser for constrained command syntax
- Accept structured input only (the Axis `--constrain` grammar machine guides what's valid)

**Decision needed:** is NL input in scope, or is structured input sufficient?

### 3. Output generation (not addressed)

The triad retrieves WHICH constructs to use, but doesn't generate the actual Axis code (field names, types, paths). For that:

- Template assembly: each pattern has a template, fill slots from the query fields
- HDC sequence encoding: encode the construct sequence, decode back to tokens via constrained grammar
- Direct SDM retrieval: store full Axis code snippets as SDM data vectors, retrieve and adapt

**Decision needed:** template vs generative vs retrieval?

### 4. HDC capacity wall

Bundling saturates at ~O(sqrt(dim)) items. With dim=10000, reliable recovery of ~100 bundled items. A complex backend spec with 50+ constructs is within range, but a marketplace like helperbook (250 constructs) may hit the limit.

**Mitigation:** hierarchical encoding (bundle groups of constructs, not individual ones), or increase dim to 100000.

### 5. Integration with SOMA runtime

The triad could replace the `BrainFallback` in soma-next's session controller. The bridge:

- SOMA world state snapshot → HDC.encode_record() → SDM query
- SDM returns similar past episodes → skill selection
- Replaces the current HashEmbedder-based SdmBrainFallback with proper HDC encoding

**Not yet wired.** The soma-next SDM uses a simpler FNV-1a hash embedder. The triad's HDC encoding is richer (role-filler structure, not flat hashing). Bridging them is a future step.

### 6. Comparison with soma-brain's LTC+SDM

soma-research/soma-brain attempted LTC+SDM but the LTC tried to do everything — store knowledge, reason, and generate. It required massive training data and produced noise. The triad explicitly removes knowledge storage and composition from the LTC, leaving only control flow. This should be trainable on orders of magnitude less data. **Unvalidated** — no training run has been attempted yet.

## Files

```
soma_triad/
  hdc.py         HDC algebra (0 params): bind/unbind/bundle/permute/encode/analogy
  sdm.py         SDM store (0 params): write/read/blend/decay/reinforce
  controller.py  LTC controller (~2.6M params): ODE cell + operation head + arg head
  triad.py       Orchestrator: wires controller decisions over SDM + HDC
tests/
  test_hdc.py         10 tests — algebra properties
  test_sdm.py         6 tests — memory operations
  test_triad.py       6 tests — orchestration
  test_axis_proof.py  7 tests — Axis construct selection from stored patterns
```

## Next steps (in order)

1. **Template assembly** — for each pattern type (CRUD, auth, stream, service, policy), store an Axis code template in SDM. Retrieve template + fill slots from query fields. Proves end-to-end: structured spec → Axis code, no LLM.
2. **Decomposition traces** — hand-author 20-30 traces of novel requirement → sub-intents. Train the LTC controller on these. Proves: controller learns general decomposition strategy.
3. **SOMA integration** — wire HDC encoding into soma-next's SDM brain fallback. Replace FNV-1a with proper role-filler encoding. Proves: triad works as SOMA's native brain.
4. **Axis grammar constraint** — use `axis --constrain` output as a state machine that validates each emitted token. The triad proposes, the grammar accepts/rejects. Guarantees valid output.
