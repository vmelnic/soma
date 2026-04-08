# SOMA Synthesizer

The Synthesizer is the SOMA equivalent of a compiler. It takes a base neural architecture + a target body specification (plugins + hardware) and produces a trained Mind that can operate that body.

**The Synthesizer is the ONLY component that uses Python and PyTorch.** Everything it produces is consumed by the Rust SOMA Core at runtime. It is a build tool — used once during synthesis, not during operation.

```
[Python + PyTorch]              [Rust + ONNX Runtime]
   Synthesizer            →      SOMA Binary
   (build time)                   (runtime)

   Trains model                   Loads .onnx model
   Exports to ONNX                Runs inference
   Produces artifacts             Handles plugins
                                  Speaks Synaptic Protocol
                                  Manages memory/LoRA
```

## Installation

```bash
cd soma-synthesizer
pip install -e .

# Or with dev tools
pip install -e ".[dev]"
```

Requires Python 3.10+ and PyTorch 2.0+.

## Quick Start

```bash
# Train a Mind from plugin training data
soma-synthesize train \
  --plugins ./plugins \
  --output ./models

# Train with domain-specific data
soma-synthesize train \
  --plugins ./plugins \
  --domain ./helperbook/training.json \
  --config synthesis_config.toml \
  --output ./models

# Validate training data before training
soma-synthesize validate --plugins ./plugins

# Train LoRA for a specific plugin
soma-synthesize train-lora \
  --plugin postgres \
  --base-model ./models \
  --output ./lora

# Export to different targets
soma-synthesize export \
  --model ./models/checkpoint.pt \
  --output ./models \
  --target both

# Export for ESP32
soma-synthesize export \
  --model ./models/checkpoint.pt \
  --target embedded \
  --embedded-ram 256000 \
  --embedded-flash 4000000 \
  --output ./models/esp32

# Test model on held-out intents
soma-synthesize test \
  --model ./models/checkpoint.pt \
  --plugins ./plugins

# Benchmark inference speed
soma-synthesize benchmark \
  --model ./models/checkpoint.pt \
  --iterations 100

# Export experience from SOMA checkpoint for re-synthesis
soma-synthesize export-experience \
  --checkpoint ./checkpoints/soma-latest.ckpt \
  --output ./experience.json
```

## Architecture

### Neural Model: BiLSTM + GRU

```
Intent text ("list files in /tmp")
       |
  [Tokenizer — vocab lookup]
       |
  [Encoder — BiLSTM, 2 layers, bidirectional]
       |
  [Decoder — GRU, autoregressive]
       |  step 0: START → predict opcode, arg types, spans, refs, literals
       |  step 1: prev_op → predict opcode, arg types, spans, refs, literals
       |  ...
       |  step N: STOP predicted → exit
       |
  [Program — list of convention calls with resolved arguments]
```

**Output heads per decoder step:**
- Opcode logits (which convention to call)
- Arg0/Arg1 type logits (none / span / ref / literal)
- Span position logits (start/end pointers into the intent)
- Ref logits (pointer to a previous step's result)
- Literal value logits (decoded from vocabulary)

### Transformer Variant (Future)

For SOMAs with 100+ conventions. Uses Transformer encoder/decoder with cross-attention. Requires ~10-50M parameters vs ~1M for BiLSTM+GRU. Architecture is designed but not yet implemented — the `TransformerMind` class is a documented stub.

## Synthesis Pipeline

```
Inputs:
  ├── Plugin training data (training/examples.json per plugin)
  ├── Architecture config (synthesis_config.toml)
  ├── Domain-specific training data (optional)
  └── Target specification (server/embedded)

Pipeline:
  1. Collect training data from all plugins
  2. Validate (convention checks, conflict detection, coverage balance)
  3. Build unified convention catalog (merge all plugins, add EMIT+STOP)
  4. Build tokenizer vocabulary from all intents
  5. Expand training pairs (intents x param pools)
  6. Augment (synonym replacement, word dropout, typo injection)
  7. Train base Mind model
  8. Evaluate on held-out test set
  9. Train per-plugin LoRA weights (optional)
  10. Export:
      - encoder.onnx + decoder.onnx (server)
      - model.soma-model (embedded, int8 quantized)
      - tokenizer.json, catalog.json, meta.json
      - Per-plugin .lora files

Outputs:
  models/
    encoder.onnx          # Intent encoding
    decoder.onnx          # Program generation (single step)
    tokenizer.json        # Vocabulary
    catalog.json          # Convention catalog (separate file)
    meta.json             # Metadata with model_hash (SHA-256)
  embedded/
    model.soma-model      # Int8 quantized binary format
  lora/
    postgres.lora         # Per-plugin LoRA weights
    postgres.lora.json    # LoRA metadata
```

## Configuration

Create `synthesis_config.toml`:

```toml
[architecture]
type = "bilstm_gru"        # or "transformer" (future)
embed_dim = 64
hidden_dim = 128
decoder_dim = 256
num_encoder_layers = 2
dropout = 0.3
max_program_steps = 16
opcode_embed_dim = 32

[training]
epochs = 200
batch_size = 32
learning_rate = 1e-3
weight_decay = 1e-2
patience = 30
scheduler_patience = 10
scheduler_factor = 0.5
gradient_clip = 1.0
train_split = 0.8
val_split = 0.1
test_split = 0.1

[augmentation]
enabled = true
synonym_replace_rate = 0.3
word_dropout_rate = 0.2
word_shuffle_rate = 0.1
typo_rate = 0.05
augmentation_factor = 3

[lora]
rank = 8
alpha = 2.0
epochs = 40
learning_rate = 2e-3
target_modules = ["op_head", "gru", "a0t_head", "a1t_head"]
```

## Plugin Training Data Format

Each plugin provides `training/examples.json`:

```json
{
  "schema_version": "1.0",
  "plugin": "postgres",
  "plugin_version": "0.1.0",
  "examples": [
    {
      "id": "pg_001",
      "intents": [
        "find all contacts near downtown",
        "search for contacts in the downtown area",
        "show contacts close to downtown"
      ],
      "program": [
        {
          "convention": "postgres.query",
          "args": [
            { "name": "sql", "type": "literal", "value": "SELECT * FROM contacts WHERE area = $1" },
            { "name": "params", "type": "span", "extract": "location" }
          ]
        },
        { "convention": "EMIT", "args": [{ "name": "data", "type": "ref", "step": 0 }] },
        { "convention": "STOP" }
      ],
      "params": {
        "location": {
          "pool": ["downtown", "city center", "north side", "the park"],
          "type": "string"
        }
      },
      "tags": ["geospatial", "contacts"]
    }
  ]
}
```

## Training Data Validation

```bash
soma-synthesize validate --plugins ./plugins

Validation Report:
  ✓ 412 examples loaded from 8 plugins
  ✓ All conventions referenced exist in plugin manifests
  ✗ CONFLICT: examples pg_003 and pg_007 have identical intent but different programs
  ⚠ IMBALANCE: postgres has 120 examples, smtp has 8 examples (15:1 ratio)
  ⚠ LOW COVERAGE: redis.subscribe has 0 training examples
  ✓ No circular refs in program steps
  ✓ All ref indices point to valid previous steps
```

Checks performed:
- Convention exists (error if referenced convention not in any plugin)
- Conflict detection (error if same intent maps to different programs)
- Ref index validation (error if ref points to future step)
- Coverage balance (warning if plugin ratio exceeds 5:1)
- Duplicate detection (warning for identical examples)
- Program length (warning if > max_steps)
- Intent length (warning if > 100 tokens)

## Data Augmentation

The augmentor generates diverse training data from templates:

| Technique | Rate | Example |
|-----------|------|---------|
| Synonym replacement | 30% | "list files" → "show files" |
| Word dropout | 20% | "please list all the files" → "list files" |
| Word shuffle | 10% | "find contacts nearby" → "contacts find nearby" |
| Typo injection | 5% | "search for contacts" → "serach for contacts" |

## Evaluation Metrics

| Metric | Description | Target |
|--------|-------------|--------|
| Op accuracy | Per-step opcode prediction | >95% |
| Program exact match | Entire program correct | >90% |
| Span accuracy | Span positions correct | >90% |
| Ref accuracy | Ref pointers correct | >95% |
| Literal accuracy | Literal values match | >90% |
| End-to-end | All correct per step | >85% |
| Novel intents | Held-out phrasings | >75% |

## LoRA Training

Per-plugin LoRA gives the Mind immediate expertise with a plugin's conventions:

```bash
soma-synthesize train-lora \
  --plugin postgres \
  --base-model ./models \
  --output ./lora
```

Produces:
- `lora/postgres.lora` — LoRA A/B weight matrices
- `lora/postgres.lora.json` — Metadata (architecture, dims, rank, alpha, training stats)

LoRA weights are loaded by the Rust SOMA Core at runtime via `MindEngine::attach_lora()`.

## Export Formats

### ONNX (Server/Desktop)

Standard ONNX models consumed by `tract-onnx` in the Rust runtime:
- `encoder.onnx` — BiLSTM encoder
- `decoder.onnx` — Single GRU decoder step (autoregressive)

### .soma-model (Embedded/ESP32)

Custom binary format with int8 quantization:

```
Header:
  magic: "SOMA" (4 bytes)
  version: u8
  quantization: u8 (0=f32, 1=f16, 2=int8)
  architecture: u8 (0=bilstm_gru)
  vocab_size: u32 BE
  embed_dim, hidden_dim, decoder_dim: u16 BE each
  num_layers: u8
  num_conventions: u16 BE
  max_steps: u8

Sections:
  section_count: u16 BE
  [name_len(u8) + name + shape(ndim u8 + dims u16 BE) + data + scale(f32) + zero_point(i8)]
```

Quantization uses post-training calibration with activation range observation.

## Tokenizer

Two tokenizer strategies:

| Strategy | Use Case |
|----------|----------|
| **Word-level** (default) | Simple, fast, sufficient for fixed domains |
| **BPE** | Handles OOV, technical content (SQL, URLs), multilingual |

Both support special tokens (PAD=0, UNK=1, NULL=2) and span extraction via `find_span()`.

For embedded targets, `max_vocab_size` constrains the vocabulary to fit in flash.

## Re-Synthesis

When plugins change, the Mind needs updating:

| Change | Action |
|--------|--------|
| New plugin added | Train plugin LoRA only (incremental) |
| Convention added | Train plugin LoRA, or full retrain |
| Convention removed | Full re-synthesis (model must unlearn) |
| Architecture change | Full re-synthesis, all LoRAs invalidated |

### Continuous Synthesis from Experience

```bash
# Export successful experiences from a running SOMA
soma-synthesize export-experience \
  --checkpoint ./checkpoints/soma-latest.ckpt \
  --output ./experience.json

# Add to training data and re-synthesize
soma-synthesize train \
  --plugins ./plugins \
  --domain ./experience.json \
  --output ./models
```

This closes the loop: runtime experience improves the next synthesis.

## Module Structure

```
soma_synthesizer/
  __init__.py          # Package metadata
  cli.py               # CLI entry point (soma-synthesize command)
  config.py            # SynthesisConfig from TOML
  tokenizer.py         # Word-level + BPE tokenizers
  model.py             # SomaMind (BiLSTM+GRU) + TransformerMind (stub)
  data.py              # ConventionCatalog, training data collection, Dataset
  trainer.py           # SomaTrainer with combined loss + all eval metrics
  augmentor.py         # Synonym, dropout, shuffle, typo augmentation
  validator.py         # Training data validation (errors + warnings)
  exporter.py          # ONNX export, .soma-model, quantization, metadata
  lora.py              # LoRA training, save/load, merge
```

**4,368 lines of Python. Self-contained. Zero dependencies on poc/ or pow/.**

## Spec Compliance

Validated against the [synthesizer specification](../docs/synthesizer.md) (14 sections, 89 items):
- 87 PASS
- 2 documented-future (Transformer variant, paraphrase generation via external LLM)
