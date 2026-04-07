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
    - Opcode logits (num_conventions + 2)
    - Arg0 type logits (none/span/ref)
    - Arg1 type logits (none/span/ref)
    - Span position logits (start/end for arg0 and arg1)
    - Ref logits (pointer to previous step for arg0 and arg1)
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
```

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
| End-to-end | Correct op + correct spans + correct refs | >85% |
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
