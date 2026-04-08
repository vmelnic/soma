# Synthesizer

## Overview

The Synthesizer is the SOMA equivalent of a compiler. It takes a base neural architecture, a target body specification (plugins and hardware), and plugin training data, then produces a trained Mind that can operate that body.

The Synthesizer is the ONLY component that uses Python and PyTorch. Everything it produces is consumed by the Rust SOMA Core at runtime. It is a build tool -- used once during synthesis, not during operation.

```
[Python + PyTorch]              [Rust + tract-onnx]
   Synthesizer            ->      SOMA Binary
   (build time)                   (runtime)

   Trains model                   Loads .onnx model
   Exports to ONNX                Runs inference
   Produces artifacts             Handles plugins
                                  Speaks Synaptic Protocol
                                  Manages memory/LoRA
```

Input: plugin training data + architecture config. Output: trained Mind (ONNX models, tokenizer, convention catalog, metadata, optional LoRA weights, optional embedded binary).

Source: `soma-synthesizer/` -- 4,368 lines of Python across 10 modules. Self-contained. Zero imports from `poc/` or `pow/`.


## Installation

```bash
cd soma-synthesizer
pip install -e .

# Or with dev tools (pytest, ruff)
pip install -e ".[dev]"
```

Requires Python 3.10+ and PyTorch 2.0+. Additional dependencies: onnx >= 1.14, onnxruntime >= 1.16, toml >= 0.10, tqdm >= 4.60.


## CLI Reference

The `soma-synthesize` command is the single entry point, with seven subcommands.

### soma-synthesize train

Train a new Mind from plugin training data. Full pipeline: collect, validate, expand, augment, build vocab, create model, train, export.

```bash
soma-synthesize train --plugins ./plugins --output ./models
soma-synthesize train --plugins ./plugins --domain ./helperbook/training.json \
  --config synthesis_config.toml --output ./models --target both
```

Flags: `--plugins` (required), `--config` (default: `synthesis_config.toml`), `--domain` (optional domain data), `--output` (default: `./models`), `--target` (`server`|`embedded`|`both`, default: `server`).

### soma-synthesize train-lora

Train LoRA weights for a specific plugin from a trained base Mind.

```bash
soma-synthesize train-lora --plugin postgres --base-model ./models --output ./lora
```

Flags: `--plugin` (required), `--base-model` (required), `--output` (default: `./lora`), `--config`.

### soma-synthesize export

Export a trained model to ONNX or `.soma-model` format.

```bash
soma-synthesize export --model ./models --output ./export --target server
soma-synthesize export --model ./models --target embedded \
  --embedded-ram 256000 --embedded-flash 4000000 --output ./models/esp32
```

Flags: `--model` (required), `--output` (default: `./models`), `--target` (`server`|`embedded`|`both`, default: `server`), `--embedded-ram` (default: 256000), `--embedded-flash` (default: 4000000).

### soma-synthesize validate

Validate plugin training data before training. Exits with code 1 on errors.

```bash
soma-synthesize validate --plugins ./plugins
soma-synthesize validate --plugins ./plugins --domain ./training/domain.json
```

### soma-synthesize test

Test a trained model against plugin data, reporting evaluation metrics.

```bash
soma-synthesize test --model ./models --plugins ./plugins
```

### soma-synthesize benchmark

Benchmark model inference speed using intents of varying lengths.

```bash
soma-synthesize benchmark --model ./models --iterations 100
```

### soma-synthesize export-experience

Export successful experiences from a SOMA checkpoint as training data for re-synthesis.

```bash
soma-synthesize export-experience --checkpoint ./checkpoints/soma-latest.ckpt \
  --output ./experience.json
```


## Synthesis Pipeline

The full pipeline from raw plugin data to exported artifacts:

```
Inputs:
  +-- Plugin training data (training/examples.json per plugin)
  +-- Architecture config (synthesis_config.toml)
  +-- Domain-specific training data (optional)
  +-- Target specification (server/embedded)

Pipeline:
  1. Collect training data from all plugin training/*.json files
  2. Build unified convention catalog (merge all plugins, add EMIT+STOP)
  3. Validate (convention checks, conflict detection, coverage balance)
  4. Expand training pairs (intents x param pools)
  5. Build tokenizer vocabulary from all intent text
  6. Augment (synonym replacement, word dropout, shuffle, typos)
  7. Split: 80% train, 10% validation, 10% test
  8. Train base Mind model (combined cross-entropy loss)
  9. Evaluate on held-out test set (7 metrics)
  10. Export:
      - encoder.onnx + decoder.onnx (server)
      - model.soma-model (embedded, int8 quantized)
      - tokenizer.json, catalog.json, meta.json
      - soma_mind.pt (checkpoint for later LoRA/re-export)

Outputs:
  models/
    encoder.onnx          # Intent encoding
    decoder.onnx          # Program generation (single decoder step)
    tokenizer.json        # Vocabulary
    catalog.json          # Convention catalog
    meta.json             # Metadata with SHA-256 model hash
    soma_mind.pt          # PyTorch checkpoint
  embedded/
    model.soma-model      # Int8 quantized binary format
```

Typical training output:

```
[Synthesis] Collecting training data...
  Plugins: 8, Total conventions: 87, Raw examples: 412
  Expanded training pairs: 15,234, After augmentation: 60,936
[Synthesis] Model: bilstm_gru, 2,100,000 parameters
[Synthesis] Training up to 200 epochs (patience=30)...
  Epoch  50 | Train loss=0.2340 | Val loss=0.1560 prog=0.940 e2e=0.890
  Early stopping at epoch 95 (best=82)
[Test] Op Accuracy: 0.982, Program Exact: 0.961, End-to-End: 0.943
```


## Training Data Format

Each plugin provides `training/examples.json` containing intent-to-program mappings:

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

Training data comes from three sources:

- **Plugin training data**: each plugin provides `training/examples.json` with (intent, program) pairs.
- **Cross-plugin training data**: multi-plugin operations (cache then query, query then email). Provided as `_cross_plugin` examples or synthesized from single-plugin examples.
- **Domain-specific training data**: application-specific intents, passed via `--domain`. For HelperBook: "show all contacts near me," "send a connection request to Ana."

The convention catalog is built automatically by scanning plugin directories for `manifest.toml` or `manifest.json`. Built-in opcodes EMIT and STOP are appended. During expansion, each example's `params` pools are combined with its `intents` templates to generate all concrete pairs. See `docs/plugin-development.md` for writing training data.


## Model Architecture

### Default: BiLSTM + GRU

The default architecture, proven from SOMA's proof-of-work implementations. Suitable for up to approximately 100 conventions and approximately 1M parameters.

```
Intent text ("list files in /tmp")
       |
  [Tokenizer -- word-level vocab lookup]
       |
  [Encoder -- BiLSTM, 2 layers, bidirectional]
       |                output: (B, L, hidden_dim * 2)
       |
  [Decoder -- GRU, autoregressive with attention]
       |  step 0: <START> -> predict opcode + args
       |  step 1: prev_op -> predict opcode + args
       |  ...
       |  step N: STOP predicted -> exit
       |
  [Program -- list of convention calls with resolved arguments]
```

**Encoder**: BiLSTM with 2 layers, bidirectional. Input is token embeddings (embed_dim=64). Output is encoder states of dimension hidden_dim * 2 (256 by default). Mean-pooled summary initializes the decoder hidden state via a learned projection.

**Decoder**: autoregressive GRU. At each step, the decoder receives the previous opcode embedding concatenated with an attention context vector over encoder states, and produces predictions through 11 output heads:

| Head | Output | Description |
|------|--------|-------------|
| `op_head` | (num_opcodes,) | Which convention to call |
| `a0t_head` | (4,) | Arg0 type: none / span / ref / literal |
| `a1t_head` | (4,) | Arg1 type: none / span / ref / literal |
| `s0s_q`, `s0e_q` | (L,) each | Span start/end pointers for arg0 |
| `s1s_q`, `s1e_q` | (L,) each | Span start/end pointers for arg1 |
| `r0q`/`r0k`, `r1q`/`r1k` | (max_steps,) | Ref pointers to previous step results |
| `lit0_head` | (vocab_size,) | Literal arg0 value (token from vocabulary) |
| `lit1_head` | (vocab_size,) | Literal arg1 value (token from vocabulary) |

Span heads use dot-product attention over encoder outputs. Ref heads use dot-product attention over accumulated previous decoder hidden states.

### Transformer Variant (Future)

For SOMAs with 100+ conventions (web applications with many plugins). Uses Transformer encoder (4-8 layers, 4-8 heads) and Transformer decoder with cross-attention. Requires approximately 10-50M parameters vs approximately 1M for BiLSTM+GRU. The `TransformerMind` class exists as a documented stub in `model.py`.


## Training Process

### Loss Function

Combined cross-entropy over all decoder steps and all output heads. Conditional heads (span positions, ref pointers, literal values) use masked cross-entropy -- loss is computed only where the target argument type matches (span loss only on steps where arg type is span, etc.):

```
loss = CE(op) + CE(a0_type) + CE(a1_type)
     + masked_CE(span_s0) + masked_CE(span_e0)
     + masked_CE(span_s1) + masked_CE(span_e1)
     + masked_CE(ref0)    + masked_CE(ref1)
     + masked_CE(lit0)    + masked_CE(lit1)
```

### Optimizer and Scheduling

- Optimizer: AdamW (configurable learning rate, default 1e-3, weight decay 1e-2)
- Scheduler: ReduceLROnPlateau (patience=10, factor=0.5)
- Gradient clipping: max norm 1.0
- Early stopping: patience=30 epochs without validation loss improvement
- Best model weights restored after early stopping

### Evaluation Metrics

| Metric | Description | Target |
|--------|-------------|--------|
| Op accuracy | Per-step opcode prediction accuracy | >95% |
| Program exact match | Entire program matches target (all opcodes) | >90% |
| Span accuracy | All span positions correct (start and end) | >90% |
| Ref accuracy | All ref pointers correct | >95% |
| Literal accuracy | Literal argument values match | >90% |
| End-to-end | Correct op + correct arg types + correct values, all steps | >85% |
| Novel intent accuracy | Accuracy on held-out intent phrasings | >75% |


## Data Augmentation

The augmentor generates diverse training data from templates, bridging the gap between template expansion and real user phrasing diversity. Each technique operates on intent text only; the target program is unchanged.

| Technique | Default Rate | Example |
|-----------|-------------|---------|
| Synonym replacement | 30% | "list files" -> "show files", "display files" |
| Word dropout | 20% | "please list all the files" -> "list files" |
| Word shuffle | 10% | "find contacts nearby" -> "contacts find nearby" |
| Typo injection | 5% | "search for contacts" -> "serach for contacts" |

Synonym replacement uses a built-in table of action verbs and filler words. Words that look like paths, SQL fragments, or parameter values (containing `/`, `$`, `=`, etc.) are never modified. Word dropout only removes words from a safe set (articles, prepositions, filler). Typo injection applies at most one typo per intent using common keyboard transposition patterns.

The `augmentation_factor` (default 3) controls how many augmented copies are generated per original example. With a factor of 3, a dataset of 1,000 examples produces 4,000 pairs (1,000 original + 3,000 augmented).


## Plugin LoRA Training

### When to Train Plugin LoRA

After the base Mind is trained, per-plugin LoRA can improve the Mind's proficiency with a specific plugin's conventions without retraining the full model. This is useful when a plugin has domain-specific patterns that the base Mind hasn't learned well.

### Process

1. Load base Mind weights from a previous `train` run
2. Freeze all base parameters (requires_grad = False)
3. Attach LoRA adapters (LoRALinear) to target modules
4. Train on ONLY this plugin's training examples
5. Save LoRA weights and metadata

LoRA uses low-rank decomposition: for each target linear layer, two small matrices A (rank x in_features) and B (out_features x rank) are added. Output becomes `base(x) + (x @ A^T) @ B^T * (alpha / rank)`. B is initialized to zero so LoRA starts as identity.

Default target modules: `op_head`, `gru`, `a0t_head`, `a1t_head`.

### Output

- `{plugin_name}.lora` -- LoRA A/B weight matrices per target module
- `{plugin_name}.lora.json` -- metadata (architecture, dimensions, rank, alpha, target layers, Mind version)

LoRA weights are loaded by the Rust SOMA Core at runtime via `MindEngine::attach_lora()`. Multiple LoRAs can be active simultaneously via Mixture-of-Experts gating at inference time.


## Export Targets

### ONNX (Server/Desktop)

Standard ONNX models consumed by `tract-onnx` in the Rust runtime:

- `encoder.onnx` -- BiLSTM encoder. The exporter splits the bidirectional LSTM into separate forward/backward passes with explicit masking (replacing `pack_padded_sequence` which is not ONNX-traceable).
- `decoder.onnx` -- single GRU decoder step with all output heads. Called autoregressively by the Rust runtime.

Model dimensions and special token IDs are recorded in `meta.json`.

### .soma-model (Embedded)

Custom binary format with int8 quantization for embedded targets (ESP32 with PSRAM):

Binary layout: 4-byte `SOMA` magic, followed by version (u8), quantization mode (u8: 0=f32, 1=f16, 2=int8), architecture (u8: 0=bilstm_gru), then model dimensions (vocab_size u32 BE, embed/hidden/decoder dims u16 BE, num_layers u8, num_conventions u16 BE, max_steps u8). Weight sections follow: each has a UTF-8 name, shape, raw data, and for int8 a per-tensor scale (f32) and zero_point (i8).

Quantization uses post-training calibration: activation ranges are observed by running 100-500 representative examples through the model, then per-tensor scale and zero-point are computed.

| Method | Model Size | Accuracy Impact |
|--------|-----------|-----------------|
| Float32 | 1x baseline | None |
| Float16 | 0.5x | Negligible (<0.1%) |
| Int8 symmetric | 0.25x | Small (1-3%) |
| Int8 asymmetric | 0.25x + scales | Smaller (0.5-2%) |

### Metadata (meta.json)

Exported alongside the models. Contains architecture parameters (vocab_size, embed_dim, hidden_dim, decoder_dim, num_layers), convention mapping (num_conventions, emit_id, stop_id, start_token), training statistics (examples, epochs, test_e2e_accuracy), SHA-256 hash over ONNX files, and export timestamp.


## Re-Synthesis

Re-synthesis is needed when plugins change, architecture is upgraded, or training data is improved.

### Incremental vs Full

| Change | Action |
|--------|--------|
| New plugin added | Train plugin LoRA only. Base Mind unchanged. |
| Convention added | Train plugin LoRA, or full retrain for best results. |
| Convention removed | Full re-synthesis (model must unlearn). |
| New domain training data | Fine-tune base Mind (few epochs) + retrain domain LoRA. |
| Architecture change | Full re-synthesis. All LoRAs invalidated. |

### Continuous Synthesis from Experience

For production SOMAs that accumulate experience, periodically re-synthesize incorporating experiential data:

```bash
# 1. Export successful experiences from a running SOMA
soma-synthesize export-experience \
  --checkpoint ./checkpoints/soma-latest.ckpt \
  --output ./experience.json

# 2. Add to training data and re-synthesize
soma-synthesize train \
  --plugins ./plugins \
  --domain ./experience.json \
  --output ./models

# 3. Deploy new model -- SOMA loads new base weights
# 4. Experiential LoRA reset (consolidated knowledge is now in base)
```

This closes the loop: runtime experience improves the next synthesis.


## Configuration

Create `synthesis_config.toml` to override defaults. Any section or key missing from the file falls back to the default value. Extra keys are silently ignored for forward compatibility.

```toml
[architecture]
type = "bilstm_gru"        # or "transformer" (future)
embed_dim = 64             # token embedding dimensionality
hidden_dim = 128           # unidirectional LSTM hidden size (encoder output = 2x)
decoder_dim = 256          # GRU decoder hidden size
num_encoder_layers = 2     # BiLSTM layers
num_decoder_layers = 1     # GRU uses 1, Transformer can use more
dropout = 0.3              # dropout in encoder and embeddings
max_program_steps = 16     # maximum decoder steps
opcode_embed_dim = 32      # embedding size for previous opcode feedback

[training]
epochs = 200               # maximum training epochs
batch_size = 32
learning_rate = 1e-3       # AdamW learning rate
weight_decay = 1e-2        # AdamW weight decay
patience = 30              # early stopping patience (epochs)
scheduler_patience = 10    # ReduceLROnPlateau patience
scheduler_factor = 0.5     # LR reduction factor
gradient_clip = 1.0        # max gradient norm
train_split = 0.8          # fraction of data for training
val_split = 0.1            # fraction for validation
test_split = 0.1           # fraction for test

[augmentation]
enabled = true
synonym_replace_rate = 0.3   # probability of synonym replacement
word_dropout_rate = 0.2      # probability of word dropout
word_shuffle_rate = 0.1      # probability of word shuffle
typo_rate = 0.05             # probability of typo injection
augmentation_factor = 3      # augmented copies per original example

[lora]
rank = 8                   # LoRA decomposition rank
alpha = 2.0                # scaling factor (effective scale = alpha / rank)
epochs = 40                # LoRA training epochs
learning_rate = 2e-3       # LoRA-specific learning rate
target_modules = ["op_head", "gru", "a0t_head", "a1t_head"]
```

TOML sections map directly to config dataclasses:

| Section | Dataclass | Description |
|---------|-----------|-------------|
| `[architecture]` | `ArchitectureConfig` | Model dimensions and structure |
| `[training]` | `TrainingConfig` | Optimizer, scheduler, data splits |
| `[augmentation]` | `AugmentationConfig` | Augmentation technique rates |
| `[lora]` | `LoRAConfig` | Per-plugin LoRA parameters |


## Validation

Run `soma-synthesize validate` before training to catch data quality issues early. The validator performs seven checks, classified as errors (fatal, block training) or warnings (non-fatal):

```
soma-synthesize validate --plugins ./plugins

[Validate] Loaded 412 examples from 8 plugins
[Validate] Convention catalog: 89 entries

  [ERROR] Example pg_003 step 2: convention 'mqtt.publish' not found in catalog
  [ERROR] CONFLICT: intent 'find contacts nearby' maps to different programs
          in examples pg_003, pg_007
  [WARN] IMBALANCE: postgres has 120 examples, smtp has 8 examples (15:1 ratio)
  [WARN] LOW COVERAGE: redis.subscribe has 0 training examples
  [WARN] DUPLICATE: geo_005 is identical to geo_002 after normalization

[Validate] FAILED -- 2 error(s), 3 warning(s)
```

### Checks Performed

| Check | Severity | Description |
|-------|----------|-------------|
| Convention exists | Error | Every convention referenced in programs must exist in the catalog |
| No conflicting examples | Error | Same intent text must not map to different programs |
| Ref indices valid | Error | `ref:N` must point to step N where N < current step (no self-ref, no forward-ref) |
| Coverage balance | Warning | Plugin example counts should be within 5:1 ratio |
| Zero-example conventions | Warning | Conventions with no training examples will be unusable |
| Duplicate examples | Warning | Identical (intents, program) pairs after normalization are redundant |
| Program length | Warning | Programs with > max_program_steps are un-learnable |
| Intent length | Warning | Intents > 100 tokens may exceed model capacity |


## Tokenizer

Two tokenizer strategies are supported, both with special tokens PAD (0), UNK (1), NULL (2):

| Strategy | Vocab Size | Handles OOV | Best For |
|----------|-----------|-------------|----------|
| Word-level (default) | 5,000-50,000 | OOV -> UNK | Fixed domains, fast |
| BPE | 1,000-10,000 | Subword fallback | Technical content (SQL, URLs), multilingual |

The word-level tokenizer builds vocabulary from all training intents (tokens assigned indices in order of first appearance). BPE tokenizer is auto-detected from tokenizer.json format by the Rust runtime.

Both support `find_span()` for locating parameter text within tokenized intents, which is essential for span-type argument extraction.

For embedded targets, `max_vocab_size` constrains vocabulary to fit in flash memory. Multilingual support (en/ro/ru for HelperBook) increases vocabulary to approximately 4,000-6,000 tokens due to additional character sets.


## Module Structure

```
soma_synthesizer/
  __init__.py          # Package metadata
  cli.py               # CLI entry point (soma-synthesize command, 7 subcommands)
  config.py            # SynthesisConfig from TOML (4 dataclass sections)
  tokenizer.py         # Word-level + BPE tokenizers, find_span
  model.py             # SomaMind (BiLSTM+GRU, 11 output heads) + TransformerMind (stub)
  data.py              # ConventionCatalog, training data collection, expansion, Dataset
  trainer.py           # SomaTrainer: combined loss, 7 eval metrics, early stopping
  augmentor.py         # Synonym replacement, word dropout, shuffle, typo injection
  validator.py         # Training data validation (7 checks, errors + warnings)
  exporter.py          # ONNX export, .soma-model binary, quantization, metadata
  lora.py              # LoRALinear, apply/remove/save/load/merge, plugin-specific training
```
