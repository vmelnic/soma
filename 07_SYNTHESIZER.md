# SOMA Synthesizer — Specification

**Status:** Design  
**Depends on:** Plugin System (for training data format), SOMA Core (for export targets)  
**Language:** Python + PyTorch (build tool, NOT runtime)

---

## 1. What the Synthesizer Is

The Synthesizer is the SOMA equivalent of a compiler. It takes a base neural architecture + a target body specification (plugins + hardware) and produces a trained Mind that can operate that body.

The Synthesizer is the ONLY component that uses Python and PyTorch. Everything it produces is consumed by the Rust SOMA Core at runtime. It is a build tool — used once during synthesis, not during operation.

---

## 2. Synthesis Pipeline

```
Inputs:
  ├── Base architecture config (hidden_dim, decoder_dim, layers, ...)
  ├── Target specification (server/embedded, RAM budget, convention count)
  ├── Plugin training data (from all plugins to be loaded)
  └── Plugin LoRA training data (optional, per-plugin)

Pipeline:
  1. Collect training data from all plugin training/*.json files
  2. Build unified convention catalog (merge all plugin conventions)
  3. Generate expanded training pairs (intents × param pools)
  4. Build tokenizer vocabulary from all training intents
  5. Train base Mind model (BiLSTM+GRU or Transformer)
  6. Evaluate on held-out test set
  7. Train per-plugin LoRA weights (optional)
  8. Export:
     a. ONNX models (server/desktop target)
     b. .soma-model (embedded target)
     c. Tokenizer (vocab.json)
     d. Convention catalog (catalog.json)
     e. LoRA weights per plugin (*.lora files)
     f. Metadata (meta.json)

Outputs:
  models/
    server/
      encoder.onnx
      decoder.onnx
    embedded/
      model.soma-model
    vocab.json
    catalog.json
    meta.json
  lora/
    postgres.lora
    redis.lora
    messaging.lora
    ...
```

---

## 3. Training Data Collection

### 3.1 Sources

Training data comes from three sources:

**Plugin training data:** Each plugin provides `training/examples.json` with (intent, program) pairs. Format specified in 03_PLUGINS.md Section 16.

**Cross-plugin training data:** Multi-plugin operations (cache then query, query then email). Provided as `_cross_plugin` examples or synthesized automatically from single-plugin examples.

**Domain-specific training data:** Application-specific intents. For HelperBook: "show all contacts near me," "send a connection request to Ana." Provided as a separate training file by the application builder.

### 3.2 Collection Process

```python
def collect_training_data(plugin_dirs: list[str], domain_file: str = None):
    all_examples = []
    
    for plugin_dir in plugin_dirs:
        training_file = os.path.join(plugin_dir, "training", "examples.json")
        if os.path.exists(training_file):
            data = json.load(open(training_file))
            # Validate: all conventions referenced exist in plugin manifest
            validate_conventions(data, plugin_dir)
            all_examples.extend(data["examples"])
    
    if domain_file:
        domain_data = json.load(open(domain_file))
        all_examples.extend(domain_data["examples"])
    
    return all_examples
```

### 3.3 Convention Catalog Build

```python
def build_catalog(plugin_dirs: list[str]):
    catalog = []
    for plugin_dir in plugin_dirs:
        manifest = toml.load(os.path.join(plugin_dir, "manifest.toml"))
        for conv in manifest["conventions"]:
            conv["full_name"] = f"{manifest['plugin']['name']}.{conv['name']}"
            conv["catalog_id"] = len(catalog)
            catalog.append(conv)
    
    # Add built-in control opcodes
    catalog.append({"full_name": "EMIT", "catalog_id": len(catalog)})
    catalog.append({"full_name": "STOP", "catalog_id": len(catalog)})
    
    return catalog
```

### 3.4 Training Pair Expansion

Each training example has multiple intents and parameter pools. Expansion generates all combinations:

```python
def expand_examples(examples, catalog):
    pairs = []
    for ex in examples:
        param_pools = ex.get("params", {})
        
        # Generate all combinations of param values
        param_combos = product_of_pools(param_pools)
        
        for intent_template in ex["intents"]:
            for param_values in param_combos:
                # Substitute params into intent
                intent = intent_template.format(**param_values)
                
                # Substitute params into program
                program = resolve_program(ex["program"], param_values, catalog)
                
                pairs.append({"intent": intent, "program": program})
    
    return pairs
```

---

## 4. Model Architecture

### 4.1 Default: BiLSTM + GRU (from POWs)

Proven architecture. Good for up to ~100 conventions.

```
Encoder: BiLSTM
  - 2 layers, bidirectional
  - Input: token embeddings (embed_dim)
  - Output: encoder_output (hidden_dim × 2)

Decoder: Autoregressive GRU
  - Input: previous opcode embedding + attention context
  - Output per step: 
    - Opcode logits (num_conventions + 2 for EMIT + STOP)
    - Arg0 type logits (none/literal/span/ref)
    - Arg1 type logits (none/literal/span/ref)
    - Span position logits (start/end for arg0 and arg1)
    - Ref logits (pointer to previous step for arg0 and arg1)
    - Literal value logits (for literal arguments — decoded from vocabulary)
```

### 4.2 Alternative: Transformer (for larger SOMAs)

For SOMAs with 100+ conventions (web applications with many plugins):

```
Encoder: Transformer encoder (4-8 layers, 4-8 heads)
  - Better at capturing long-range dependencies in intents
  - Supports longer intents (>50 tokens)

Decoder: Transformer decoder (4-8 layers, causal attention)
  - Cross-attention to encoder output
  - Better at generating longer programs (>8 steps)
```

The Transformer variant requires more parameters (~10-50M vs ~1M for BiLSTM) but handles complexity better.

### 4.3 Architecture Config

```toml
# synthesis_config.toml

[architecture]
type = "bilstm_gru"        # or "transformer"
embed_dim = 64
hidden_dim = 128
decoder_dim = 256
num_encoder_layers = 2
num_decoder_layers = 1      # GRU uses 1, Transformer can use more
dropout = 0.3
max_program_steps = 16
opcode_embed_dim = 32

[architecture.transformer]  # only if type = "transformer"
num_heads = 8
ff_dim = 512
```

---

## 5. Training Process

### 5.1 Loss Function

Combined loss over all program step predictions:

```python
loss = 0
for step in range(max_steps):
    loss += cross_entropy(op_logits[step], target_op[step])
    loss += cross_entropy(a0_type[step], target_a0_type[step])
    loss += cross_entropy(a1_type[step], target_a1_type[step])
    loss += masked_cross_entropy(span_s0[step], target_s0[step])  # only where type=span
    loss += masked_cross_entropy(span_e0[step], target_e0[step])
    loss += masked_cross_entropy(span_s1[step], target_s1[step])
    loss += masked_cross_entropy(span_e1[step], target_e1[step])
    loss += masked_cross_entropy(ref0[step], target_ref0[step])   # only where type=ref
    loss += masked_cross_entropy(ref1[step], target_ref1[step])
    loss += masked_cross_entropy(lit0[step], target_lit0[step])   # only where type=literal
    loss += masked_cross_entropy(lit1[step], target_lit1[step])
```

`masked_cross_entropy` computes loss only where the arg type matches (e.g., span loss only on steps where the target arg type is "span"). Literal values are decoded from vocabulary tokens (same as the intent tokenizer).

### 5.2 Training Hyperparameters

```toml
[training]
epochs = 200
batch_size = 32
learning_rate = 1e-3
weight_decay = 1e-2
optimizer = "adamw"
scheduler = "reduce_on_plateau"
scheduler_patience = 10
scheduler_factor = 0.5
gradient_clip = 1.0
early_stopping_patience = 30

[training.data_split]
train = 0.8
validation = 0.1
test = 0.1

[training.balancing]
# Ensure equal representation per plugin
balance_by = "plugin"
# Oversample zero-param conventions (system info, time, etc.)
oversample_zero_param = 8
```

### 5.3 Evaluation Metrics

| Metric | Description | Target |
|---|---|---|
| Op accuracy | Per-step opcode prediction accuracy | >95% |
| Program exact match | Entire program matches target | >90% |
| Span accuracy | All span positions correct | >90% |
| Ref accuracy | All ref pointers correct | >95% |
| Literal accuracy | Literal argument values match | >90% |
| End-to-end | Correct op + correct args (all types) | >85% |
| Novel intent accuracy | Accuracy on held-out intent phrasings | >75% |

### 5.4 Training Output

```
[Synthesis] Collecting training data...
  Plugins: postgres (12 conv), redis (14 conv), auth (12 conv), ...
  Total conventions: 87
  Training examples: 15,234 (expanded from 412 templates)
  Vocabulary: 1,456 tokens

[Synthesis] Model: BiLSTM+GRU, 2.1M parameters

[Synthesis] Training...
  Epoch  10 | Train loss=1.234 | Val loss=0.856 prog=0.72 e2e=0.65
  Epoch  50 | Train loss=0.234 | Val loss=0.156 prog=0.94 e2e=0.89
  Epoch  80 | Train loss=0.089 | Val loss=0.102 prog=0.97 e2e=0.94
  Early stopping at epoch 95 (best=82)

[Synthesis] Test set:
  Op Accuracy:       0.982
  Program Exact:     0.961
  End-to-End:        0.943
  Novel Intents:     0.871

[Synthesis] Exporting...
  → models/server/encoder.onnx (1.2MB)
  → models/server/decoder.onnx (3.4MB)
  → models/embedded/model.soma-model (420KB, int8)
  → models/vocab.json
  → models/catalog.json
  → models/meta.json
```

---

## 6. Plugin LoRA Training

### 6.1 When to Train Plugin LoRA

After the base Mind is trained, plugin-specific LoRA can be trained to improve the Mind's proficiency with each plugin's conventions.

Plugin LoRA training:
1. Freeze base Mind weights
2. Attach LoRA adapters to target layers
3. Train on ONLY this plugin's training examples
4. Save LoRA weights

### 6.2 Process

```python
def train_plugin_lora(base_model, plugin_name, plugin_training_data, config):
    # Freeze base
    for param in base_model.parameters():
        param.requires_grad_(False)
    
    # Attach LoRA
    lora_layers = apply_lora(base_model, 
        rank=config.lora_rank, 
        alpha=config.lora_alpha,
        target_modules=config.lora_targets)
    
    # Train on plugin-specific data only
    for epoch in range(config.lora_epochs):
        for batch in plugin_dataloader:
            loss = compute_loss(base_model, batch)
            loss.backward()  # gradients flow to LoRA params only
            optimizer.step()
    
    # Save LoRA weights
    save_lora(lora_layers, f"lora/{plugin_name}.lora")
    
    # Include metadata
    save_lora_metadata(f"lora/{plugin_name}.lora.json", {
        "plugin": plugin_name,
        "mind_architecture": config.architecture_type,
        "mind_version": config.version,
        "hidden_dim": config.hidden_dim,
        "decoder_dim": config.decoder_dim,
        "target_layers": config.lora_targets,
        "rank": config.lora_rank,
        "alpha": config.lora_alpha,
    })
```

---

## 7. Export Targets

### 7.1 ONNX Export (Server/Desktop)

```python
def export_onnx(model, output_dir):
    # Export encoder
    dummy_ids = torch.zeros(1, 20, dtype=torch.long)
    dummy_lens = torch.tensor([20])
    torch.onnx.export(
        model.encoder_wrapper,
        (dummy_ids, dummy_lens),
        f"{output_dir}/encoder.onnx",
        input_names=["token_ids", "lengths"],
        output_names=["encoder_output", "pooled"],
        dynamic_axes={"token_ids": {0: "batch", 1: "seq_len"}},
    )
    
    # Export decoder (single step)
    dummy_prev_op = torch.zeros(1, dtype=torch.long)
    dummy_context = torch.zeros(1, model.encoder_out_dim)
    dummy_hidden = torch.zeros(1, model.decoder_dim)
    torch.onnx.export(
        model.decoder_step_wrapper,
        (dummy_prev_op, dummy_context, dummy_hidden),
        f"{output_dir}/decoder.onnx",
        input_names=["prev_op", "context", "hidden"],
        output_names=["op_logits", "arg_logits", "new_hidden", "span_logits", "ref_logits"],
    )
```

### 7.2 .soma-model Export (Embedded)

```python
def export_embedded(model, output_path, config):
    quantized = quantize_int8(model)  # or float16
    
    with open(output_path, "wb") as f:
        # Header
        f.write(b"SOMA")                    # magic
        f.write(struct.pack("B", 1))        # version
        f.write(struct.pack("B", 2))        # quantization: int8
        f.write(struct.pack("B", 0))        # architecture: bilstm_gru
        
        # Dimensions
        f.write(struct.pack(">H", config.vocab_size))
        f.write(struct.pack(">H", config.embed_dim))
        f.write(struct.pack(">H", config.hidden_dim))
        f.write(struct.pack(">H", config.decoder_dim))
        f.write(struct.pack("B", config.num_layers))
        f.write(struct.pack(">H", config.num_conventions))
        f.write(struct.pack("B", config.max_steps))
        
        # Weight sections
        sections = model_to_sections(quantized)
        f.write(struct.pack(">H", len(sections)))
        for name, tensor in sections:
            name_bytes = name.encode("utf-8")
            f.write(struct.pack("B", len(name_bytes)))
            f.write(name_bytes)
            shape = tensor.shape
            f.write(struct.pack("B", len(shape)))
            for dim in shape:
                f.write(struct.pack(">H", dim))
            f.write(tensor.numpy().tobytes())
```

### 7.3 Metadata Export

```python
def export_metadata(config, training_stats, output_path):
    meta = {
        "soma_synthesizer_version": "0.1.0",
        "architecture": config.architecture_type,
        "vocab_size": config.vocab_size,
        "embed_dim": config.embed_dim,
        "hidden_dim": config.hidden_dim,
        "decoder_dim": config.decoder_dim,
        "num_conventions": config.num_conventions,
        "max_steps": config.max_steps,
        "plugins": config.plugin_names,
        "training": {
            "examples": training_stats.total_examples,
            "epochs": training_stats.best_epoch,
            "test_e2e_accuracy": training_stats.test_e2e,
            "training_time_seconds": training_stats.elapsed,
        },
        "export_timestamp": datetime.now().isoformat(),
        "model_hash": compute_model_hash(config.model_path),
    }
    json.dump(meta, open(output_path, "w"), indent=2)
```

---

## 8. CLI Interface

```
soma-synthesize [OPTIONS]

Commands:
  train           Train a new Mind from plugin training data
  train-lora      Train LoRA for a specific plugin
  export          Export trained model to ONNX / .soma-model
  validate        Validate plugin training data
  test            Test model on held-out intents
  benchmark       Benchmark inference speed

Options:
  --config <file>         Synthesis config file (synthesis_config.toml)
  --plugins <dir>         Plugin directory to collect training data from
  --domain <file>         Domain-specific training data
  --output <dir>          Output directory for exported models
  --target <target>       Export target: server, embedded, both (default: both)
  --embedded-ram <bytes>  RAM budget for embedded target
  --embedded-flash <bytes> Flash budget for embedded target

Examples:
  # Train base Mind for HelperBook with all plugins
  soma-synthesize train \
    --plugins ./plugins \
    --domain ./helperbook/training.json \
    --output ./models

  # Train postgres LoRA only
  soma-synthesize train-lora \
    --plugin postgres \
    --base-model ./models/server \
    --output ./lora

  # Export for ESP32
  soma-synthesize export \
    --target embedded \
    --embedded-ram 256000 \
    --embedded-flash 4000000 \
    --model ./models/server \
    --output ./models/esp32

  # Validate training data
  soma-synthesize validate --plugins ./plugins
```

---

## 9. Re-Synthesis

### 9.1 When Re-Synthesis Is Needed

- New plugin added (new conventions to learn)
- Plugin updated with new conventions (minor version bump)
- Plugin removed (conventions no longer needed)
- Architecture upgrade (larger model for more complex domain)
- Training data improved (more intents, better coverage)

### 9.2 Incremental Re-Synthesis

Not all changes require full retraining:

| Change | Action |
|---|---|
| New plugin added | Train plugin LoRA only. Base Mind unchanged. |
| Plugin convention added | Train plugin LoRA only. Or full retrain for best results. |
| Plugin convention removed | Re-synthesize (model must unlearn). |
| New domain training data | Fine-tune base Mind (few epochs) + retrain domain LoRA. |
| Architecture change | Full re-synthesis. All LoRAs invalidated. |

### 9.3 Continuous Synthesis

For production SOMAs that accumulate experience, periodically re-synthesize incorporating experiential data:

```
1. Export SOMA's experience buffer (successful (intent, program) pairs)
2. Add to training data
3. Re-synthesize with expanded training set
4. Deploy new model
5. SOMA loads new base weights
6. Experiential LoRA reset (consolidated knowledge is now in base)
```

This closes the loop: runtime experience improves the next synthesis.

---

## 10. Tokenizer Strategy

### 10.1 Options

| Strategy | Vocab Size | Handles OOV | Best For |
|---|---|---|---|
| Character-level | ~100 | Always (any UTF-8 char is in vocab) | Small models, embedded targets |
| BPE (Byte-Pair Encoding) | 1,000-10,000 | Subword fallback | Medium models, technical content |
| Word-level | 5,000-50,000 | OOV → UNK token | Large models, fixed domains |

### 10.2 Recommended: Hybrid Character+BPE

For SOMA, the ideal tokenizer handles:
- Natural language intents ("list all contacts near downtown")
- Technical content embedded in intents ("SELECT * FROM users WHERE id = $1")
- Paths and URLs ("/tmp/data.csv", "https://api.stripe.com/v1/charges")
- Multiple languages (en, ro, ru for HelperBook)
- Names and values that were never in training data

**Strategy: train a BPE tokenizer on the training corpus, with character-level fallback for OOV.**

```python
from tokenizers import Tokenizer, models, trainers, pre_tokenizers

tokenizer = Tokenizer(models.BPE(unk_token="[UNK]"))
tokenizer.pre_tokenizer = pre_tokenizers.ByteLevel(add_prefix_space=False)

trainer = trainers.BpeTrainer(
    vocab_size=4000,           # small enough for embedded
    special_tokens=["[PAD]", "[UNK]", "[START]", "[STOP]"],
    min_frequency=2,
    show_progress=True,
)

tokenizer.train_from_iterator(all_intent_texts, trainer)
tokenizer.save("vocab.json")
```

### 10.3 Embedded Tokenizer

For ESP32, the tokenizer must be tiny. Options:
- Use smaller vocab (1,000-2,000 tokens) with more aggressive BPE merges
- Store vocab in flash, not RAM
- Character-level fallback is essential (embedded SOMAs encounter novel text)

### 10.4 SQL and Code Tokens

Training data includes SQL queries, file paths, and API parameters. The BPE tokenizer naturally learns common SQL tokens ("SELECT", "FROM", "WHERE", "INSERT") as single tokens if they appear frequently. Rare patterns decompose to subwords. This handles the technical content without a separate tokenizer.

---

## 11. Data Augmentation

### 11.1 Why Augmentation Matters

Template expansion (intent × param pool) generates syntactically varied training data. But real users phrase things in ways templates don't cover. Augmentation bridges this gap.

### 11.2 Augmentation Techniques

**Synonym replacement:** Replace words with synonyms while preserving intent:
```
"list all files in /tmp"
→ "show all files in /tmp"
→ "display all files in /tmp"
→ "get all files in /tmp"
→ "enumerate files in /tmp"
```

**Word dropout:** Randomly remove non-essential words (trains robustness to terse intents):
```
"please list all of the files in the /tmp directory"
→ "list files in /tmp"
→ "list files /tmp"
→ "files in /tmp"
```

**Word order shuffle:** Rearrange non-critical words:
```
"find contacts near downtown"
→ "near downtown find contacts"
→ "contacts near downtown find"
```
(Only mild shuffles — the intent must remain parseable.)

**Typo injection:** Introduce realistic typos (trains robustness):
```
"search for electrician"
→ "serach for electrician"
→ "search for electritian"
→ "search for electrican"
```
(Low rate: ~5% of training examples. Don't corrupt all data.)

**Paraphrase generation (optional):** Use a language model to generate diverse phrasings:
```
"create a new appointment for Thursday at 3pm"
→ "schedule something for Thursday afternoon at 3"
→ "book Thursday 3pm"
→ "I need an appointment Thursday, 3 o'clock"
→ "set up a meeting for thurs 15:00"
```

### 11.3 Augmentation Config

```toml
[training.augmentation]
enabled = true
synonym_replace_rate = 0.3      # 30% of examples get synonym variants
word_dropout_rate = 0.2         # 20% get word dropout
word_shuffle_rate = 0.1         # 10% get mild reordering
typo_rate = 0.05                # 5% get typo injection
paraphrase_enabled = false       # requires external LLM, optional
augmentation_factor = 3          # generate 3 augmented versions per original
```

### 11.4 Augmentation Validation

After augmentation, validate that augmented intents still map to the same program. If synonym replacement changes the semantic meaning (e.g., "delete" → "create"), the training pair is corrupt. Filter by:
1. Run augmented intent through a simple intent classifier
2. Verify it maps to the same convention(s) as the original
3. Discard if mismatch

---

## 12. Multilingual Intents

### 12.1 The Challenge

HelperBook serves en/ro/ru users. The Mind must understand:
- "list all contacts" (English)
- "arată toate contactele" (Romanian)
- "показать все контакты" (Russian)

All three should produce the same program: `postgres.query("SELECT * FROM contacts")`.

### 12.2 Approach: Shared Multilingual Model

One Mind handles all languages. The tokenizer (BPE) is trained on a multilingual corpus. The vocabulary includes tokens from all target languages.

Training data includes all languages:

```json
{
  "intents": [
    "list all contacts",
    "show all contacts",
    "arată toate contactele",
    "arată-mi contactele",
    "показать все контакты",
    "покажи контакты"
  ],
  "program": [
    {"convention": "postgres.query", "args": [{"type": "literal", "value": "SELECT * FROM contacts"}]}
  ]
}
```

The model learns that these different surface forms map to the same program. This is natural for neural networks — they learn semantic similarity across languages when trained with parallel examples.

### 12.3 Vocabulary Impact

Multilingual BPE vocabulary is larger:
- English only: ~2,000 tokens sufficient
- English + Romanian + Russian: ~4,000-6,000 tokens
- Cyrillic characters double the character set

For embedded targets, this may be too large. Solution: embedded SOMAs serve a single language. Server SOMAs handle multilingual.

### 12.4 Language-Specific LoRA (Alternative)

Instead of one model for all languages, use language-specific LoRA:

```
Base Mind: English (default synthesis language)
  + ro.lora: Romanian language understanding
  + ru.lora: Russian language understanding
```

The Mind detects the input language (from character set or explicit locale) and activates the appropriate LoRA. This keeps the base model smaller while supporting multiple languages.

### 12.5 Mixed-Language Intents

Real users code-switch: "arată-mi contacts from last week" (Romanian + English). The multilingual model handles this naturally — BPE tokenizes each word regardless of language, and the encoder learns to understand mixed input.

---

## 13. Training Data Validation

### 13.1 Automated Checks

Before training, the Synthesizer validates all training data:

```
soma-synthesize validate --plugins ./plugins --domain ./training/domain.json

Validation Report:
  ✓ 412 examples loaded from 8 plugins
  ✓ All conventions referenced exist in plugin manifests
  ✗ CONFLICT: examples pg_003 and pg_007 have identical intent 
    "find contacts nearby" but different programs
  ⚠ IMBALANCE: postgres has 120 examples, smtp has 8 examples 
    (15:1 ratio, recommend ≤5:1)
  ⚠ LOW COVERAGE: redis.subscribe has 0 training examples
  ✓ No circular refs in program steps
  ✓ All ref indices point to valid previous steps
  ✓ All span extractions reference valid intent tokens
  ⚠ DUPLICATE: examples geo_002 and geo_005 are identical after 
    param expansion (redundant)
  ✗ INVALID: example msg_012 references convention "mqtt.publish" 
    but mqtt plugin is not in target plugins
```

### 13.2 Checks Performed

| Check | Severity | Description |
|---|---|---|
| Convention exists | Error | Referenced convention must exist in some loaded plugin |
| No conflicting examples | Error | Same intent must not map to different programs |
| Ref indices valid | Error | `ref:N` must point to step N where N < current step |
| Span extraction valid | Error | Span markers must reference tokens present in intent |
| No circular refs | Error | Step cannot reference itself or create a cycle |
| Coverage balance | Warning | Plugin examples should be within 5:1 ratio of each other |
| Zero-example conventions | Warning | Conventions with no training examples won't be usable |
| Duplicate examples | Warning | Identical (intent, program) pairs after expansion are wasted |
| Step count | Warning | Programs with >max_steps are un-learnable |
| Intent length | Warning | Intents >100 tokens may exceed model capacity |

### 13.3 Auto-Fix Suggestions

```
CONFLICT detected:
  Example pg_003: "find contacts nearby" → postgres.query(geo query)
  Example pg_007: "find contacts nearby" → geo.within_radius(...)
  
  Suggestion: Disambiguate intents:
    pg_003: "find contacts nearby in the database"
    pg_007: "find contacts within radius nearby"
  Or: merge into one cross-plugin example
```

---

## 14. Quantization Details

### 14.1 Quantization Methods

| Method | Precision | Model Size | Accuracy Impact | Target |
|---|---|---|---|---|
| Float32 | Full | 1× (baseline) | None | Server with GPU |
| Float16 | Half | 0.5× | Negligible (<0.1%) | Server CPU, Raspberry Pi |
| Int8 symmetric | 8-bit | 0.25× | Small (1-3%) | ESP32 with PSRAM |
| Int8 asymmetric | 8-bit | 0.25× + scales | Smaller (0.5-2%) | ESP32 with PSRAM |
| Int4 (future) | 4-bit | 0.125× | Moderate (3-8%) | ESP32 without PSRAM |

### 14.2 Recommended: Post-Training Quantization with Calibration

```python
def quantize_int8(model, calibration_data, method="asymmetric"):
    """
    Post-training quantization: run calibration data through the model,
    observe activation ranges, compute per-tensor scale and zero-point.
    """
    # Collect activation statistics
    ranges = {}
    def hook_fn(name):
        def fn(module, input, output):
            ranges[name] = (output.min().item(), output.max().item())
        return fn
    
    for name, module in model.named_modules():
        if isinstance(module, nn.Linear):
            module.register_forward_hook(hook_fn(name))
    
    # Run calibration data (100-500 examples)
    with torch.no_grad():
        for batch in calibration_data:
            model(batch)
    
    # Compute quantization parameters
    quant_params = {}
    for name, (min_val, max_val) in ranges.items():
        if method == "symmetric":
            scale = max(abs(min_val), abs(max_val)) / 127.0
            zero_point = 0
        else:  # asymmetric
            scale = (max_val - min_val) / 255.0
            zero_point = round(-min_val / scale)
        quant_params[name] = {"scale": scale, "zero_point": zero_point}
    
    # Quantize weights
    quantized_weights = {}
    for name, param in model.named_parameters():
        qp = quant_params.get(name.rsplit('.', 1)[0], {"scale": 1.0, "zero_point": 0})
        quantized = torch.round(param / qp["scale"]) + qp["zero_point"]
        quantized = quantized.clamp(-128, 127).to(torch.int8)
        quantized_weights[name] = quantized
    
    return quantized_weights, quant_params
```

### 14.3 Calibration Dataset

Use 100-500 representative intents from the training set. Must cover:
- Short intents ("list files")
- Long intents ("find all providers within 10km who offer plumbing services and are available next Thursday")
- All plugins (at least a few intents per plugin)
- Edge cases (empty string, very long string, special characters)

### 14.4 Accuracy Validation After Quantization

```
soma-synthesize export --target embedded --quantize int8

Quantization Report:
  Float32 baseline E2E accuracy: 94.3%
  Int8 quantized E2E accuracy:   92.1%
  Accuracy delta:                -2.2%
  
  Per-plugin impact:
    postgres: 95.2% → 93.8% (-1.4%)
    redis:    93.1% → 91.2% (-1.9%)
    smtp:     96.0% → 92.5% (-3.5%)  ← highest impact
    
  Recommendation: accuracy delta within acceptable range (< 5%).
  Proceed with int8 export.
```

If accuracy drops >5% for any plugin, options:
- Use float16 instead of int8
- Increase calibration dataset size
- Use per-channel quantization instead of per-tensor
- Increase model size (more params are more quantization-tolerant)

### 14.5 Quantization in .soma-model Format

The .soma-model header stores quantization metadata:

```
quantization byte:
  0x00 = float32 (4 bytes per weight)
  0x01 = float16 (2 bytes per weight)
  0x02 = int8 symmetric (1 byte per weight + per-tensor scale float32)
  0x03 = int8 asymmetric (1 byte per weight + per-tensor scale + zero_point)

Per weight section:
  [section_name]
  [shape]
  [scale: float32]             ← quantization scale
  [zero_point: int8]           ← only for asymmetric
  [data: int8[]]               ← quantized weights, row-major
```

The EmbeddedMindEngine reads these and performs fixed-point inference:

```rust
fn dequantize(value: i8, scale: f32, zero_point: i8) -> f32 {
    (value as f32 - zero_point as f32) * scale
}

// Or for efficiency, multiply in int8 and dequantize the result:
fn matmul_int8(a: &[i8], b: &[i8], scale_a: f32, scale_b: f32) -> Vec<f32> {
    // Accumulate in int32 to avoid overflow
    let acc: Vec<i32> = int8_matmul_accumulate(a, b);
    // Dequantize the result
    acc.iter().map(|&v| v as f32 * scale_a * scale_b).collect()
}