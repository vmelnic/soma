# Mind Engine

Technical reference for the SOMA Mind Engine, Memory System, and LoRA adaptation.
Covers the implementation in `soma-core/src/mind/` and `soma-core/src/memory/`.
For theoretical background, see the [Whitepaper](../SOMA_Whitepaper.md) sections 4 and 7.

---

## Overview

The Mind maps structured intents to execution programs. It is the neural core of every SOMA instance.

An intent such as `"list files in /tmp"` enters the Mind as text, is tokenized, encoded by a BiLSTM, decoded autoregressively by a GRU, and exits as a `Program` -- an ordered sequence of plugin convention calls with resolved arguments. The rest of the SOMA runtime (Plugin Manager, MCP server, Synaptic Protocol) is completely decoupled from the inference backend.

Key properties:

- **MindEngine trait**: a single `infer(&str) -> Program` entry point. The tokenizer is internal to the engine -- callers never handle tokens directly.
- **Two backends**: `OnnxMindEngine` (server/desktop, tract-onnx) and `EmbeddedMindEngine` (ESP32/MCU, no_std Rust). Only the ONNX backend is implemented; the embedded backend is future work.
- **Backend transparency**: the Plugin Manager, MCP server, and protocol layers use only the `MindEngine` trait. Swapping backends requires zero changes outside `src/mind/`.

---

## MindEngine Trait

Defined in `soma-core/src/mind/mod.rs`:

```rust
pub trait MindEngine: Send + Sync {
    fn infer(&self, text: &str) -> Result<Program>;
    fn meta(&self) -> &ModelMeta;
    fn info(&self) -> MindInfo;

    // LoRA lifecycle
    fn attach_lora(&mut self, name: &str, weights: &LoRAWeights) -> Result<()>;
    fn attach_lora_bytes(&mut self, plugin_name: &str, data: &[u8]) -> Result<()>;
    fn detach_lora(&mut self, name: &str) -> Result<()>;
    fn merge_lora(&mut self, name: &str) -> Result<()>;
    fn checkpoint_lora(&self) -> Result<LoRACheckpoint>;
    fn restore_lora(&mut self, checkpoint: &LoRACheckpoint) -> Result<()>;
}
```

The spec (Section 4.2) shows `infer(tokens, length)` but the implementation takes `&str` because tokenization is internal to the engine. This preserves encapsulation and prevents callers from needing tokenizer access.

LoRA methods have default no-op implementations so backends that do not support adaptation still compile.

---

## Program Structure

A program is a sequence of steps. Each step contains a convention ID, two argument slots (each with type and value), and the decoder's confidence.

```rust
pub struct Program {
    pub steps: Vec<ProgramStep>,
    pub confidence: f32,
    pub cached_states: Vec<(Vec<f32>, Vec<f32>)>,  // (hidden, base_logits) per step
}

pub struct ProgramStep {
    pub conv_id: i32,        // convention ID, or EMIT_ID (-1), or STOP_ID (-2)
    pub arg0_type: ArgType,
    pub arg0_value: ArgValue,
    pub arg1_type: ArgType,
    pub arg1_value: ArgValue,
}

pub enum ArgType { None, Span, Ref, Literal }

pub enum ArgValue {
    None,
    Span(String),       // text extracted from intent
    Ref(usize),         // index of a prior step's result
    Literal(String),    // decoded from vocabulary
}
```

Special convention IDs: `EMIT_ID = -1` (return a previous step's result to the caller) and `STOP_ID = -2` (terminate program generation).

`cached_states` stores the decoder hidden state and base opcode logits at each step. This is pre-computed during normal inference so the adaptation engine can update LoRA without re-running the ONNX model.

### Intent Complexity Classes

| Class | Description | Example |
|-------|-------------|---------|
| 1 | Direct mapping. One or two program steps. | `"read file hello.txt"` -> `fs.read_file`, EMIT, STOP |
| 2 | Multi-step. Multiple plugins. | `"upload photo, thumbnail, cache, update profile"` -> 4-5 steps |
| 3 | LLM-decomposed. The LLM breaks complex requests into multiple Class 1/2 `soma.intent()` calls via MCP. The Mind stays simple. |

---

## Neural Architecture

The default architecture (proven in POW1-POW3) is a BiLSTM encoder with a GRU decoder. Good for up to ~100 conventions.

### Encoder (BiLSTM)

- 2 layers, bidirectional
- Input: token embeddings (`embed_dim`), padded to `max_seq_len`
- Output: `encoder_output` of shape `(1, seq_len, decoder_dim)` plus a pooled hidden state used as the decoder's initial context

### Decoder (Autoregressive GRU)

Runs one step per program operation, looping until STOP is predicted or `max_program_steps` is reached (default: 16). At each step it receives the previous opcode embedding, the current hidden state, encoder output, attention mask, and all prior hidden states.

Per-step output heads (11 total):

| Head | Output | Dimension |
|------|--------|-----------|
| Opcode logits | Which convention to call | `num_conventions + 2` (EMIT + STOP) |
| `a0t`, `a1t` | Argument type per slot | 4 (None, Span, Ref, Literal) |
| `s0s`, `s0e` | Span start/end for arg0 | `max_seq_len` |
| `s1s`, `s1e` | Span start/end for arg1 | `max_seq_len` |
| `r0`, `r1` | Ref pointer per slot | `max_steps` |
| `lit0`, `lit1` | Literal vocab logits per slot | `vocab_size` |

The opcode logits are temperature-scaled before argmax. Temperature is configurable via `[mind] temperature` (default 1.0; lower = more deterministic).

### Architecture Config (Synthesis)

```toml
[architecture]
type = "bilstm_gru"
embed_dim = 64
hidden_dim = 128
decoder_dim = 256
num_encoder_layers = 2
num_decoder_layers = 1
dropout = 0.3
max_program_steps = 16
opcode_embed_dim = 32
```

### Training Loss

Combined cross-entropy loss over all 11 output heads per decoder step. Heads that don't apply to a given step (e.g., span loss when the target arg type is Ref) are masked out. Target metrics: >95% opcode accuracy, >85% end-to-end program match.

---

## ONNX Engine

`OnnxMindEngine` in `soma-core/src/mind/onnx_engine.rs`. For macOS, Linux, Windows -- any system with an OS and at least 50MB RAM.

### Why tract-onnx (not ort)

The spec recommends `ort` (ONNX Runtime with GPU/NPU acceleration). The implementation uses `tract-onnx` instead because:

- Pure Rust: no C++ build dependency, simpler cross-compilation
- Single binary: no shared library requirements at runtime
- Sufficient for current model sizes (~800K params, <5ms inference)

Migration to `ort` is tracked for when GPU acceleration becomes necessary for larger models (50M+ params).

### Model Files

| File | Purpose | Loaded by |
|------|---------|-----------|
| `encoder.onnx` | Intent encoding | tract-onnx |
| `decoder.onnx` | Program generation (one step) | tract-onnx |
| `tokenizer.json` | Vocabulary (word-level or BPE) | `Tokenizer::load()` |
| `meta.json` | Model metadata (dims, IDs, catalog) | serde_json |
| `catalog.json` | Convention catalog (optional, if not in meta.json) | serde_json |

### ModelMeta

```rust
pub struct ModelMeta {
    pub vocab_size: usize,
    pub num_conventions: usize,
    pub max_steps: usize,
    pub max_seq_len: usize,     // default: 20
    pub decoder_dim: usize,
    pub emit_id: usize,
    pub stop_id: usize,
    pub start_token: usize,
    pub catalog: Vec<CatalogEntry>,
}
```

### OnnxMindEngine Struct

```rust
pub struct OnnxMindEngine {
    encoder: TractModel,
    decoder: TractModel,
    pub tokenizer: Tokenizer,
    model_meta: ModelMeta,
    pub temperature: f32,
    pub max_inference_time_secs: u64,
    pub model_hash: String,           // SHA-256 of encoder.onnx + decoder.onnx
    active_lora: Vec<LoRALayer>,
    pub merged_opcode_delta: Vec<f32>, // accumulated consolidation delta
}
```

`model_hash` is computed at load time from the raw bytes of both ONNX files. It is stored in checkpoints and verified on restore to detect model changes.

`merged_opcode_delta` accumulates `scale * B @ A` from past consolidations. Since tract models are frozen (compiled into the graph), base weights cannot be modified in-place. Instead, this delta is applied during inference as `logits += hidden @ merged_opcode_delta.T`.

### Inference Pipeline

```
Intent text
    |
[Tokenizer: text -> token IDs, padded to max_seq_len]
    |
[Encoder: token_ids + mask -> encoder_output + initial hidden]
    |
[Decoder loop (up to max_program_steps)]
    |  For each step t:
    |    1. Run decoder: prev_op, hidden, enc_out, mask, prev_hiddens, step -> outputs
    |    2. Cache (hidden, base_logits) for adaptation
    |    3. Apply merged_opcode_delta (consolidated LoRA knowledge)
    |    4. Apply active LoRA to all 11 output heads
    |    5. Temperature-scale opcode logits
    |    6. Argmax -> predicted opcode
    |    7. If STOP: break. If EMIT: extract ref. Else: resolve args from heads.
    |
[Program: steps + confidence + cached_states]
```

Confidence is the minimum softmax probability across all decoded steps (conservative estimate). If the inference timeout is exceeded, the partial program is returned with a 0.5x confidence penalty.

### Target Hardware

| Platform | Acceleration |
|----------|-------------|
| macOS Apple Silicon | CoreML via ONNX Runtime (future, when migrated to ort) |
| Linux + NVIDIA GPU | CUDA via ONNX Runtime (future) |
| Linux CPU | tract-onnx native (current) |
| Windows | DirectML via ONNX Runtime (future) |

---

## Embedded Engine (Future)

`EmbeddedMindEngine` -- custom inference in pure `no_std` Rust. Not yet implemented.

### Why ONNX Runtime Does Not Work on MCUs

- ONNX Runtime minimum footprint: ~50MB RAM, requires OS
- ESP32: 520KB SRAM (some variants + 2-8MB PSRAM), bare metal or FreeRTOS

### Design

The POW1 architecture (BiLSTM+GRU) needs only these operations: embedding lookup, LSTM forward, GRU forward, linear projection, softmax, argmax, mean pooling, dot-product attention. Estimated ~3-5K lines of Rust, no external dependencies.

### .soma-model Format

Custom binary format with int8 quantized weights:

```
magic: "SOMA" (4 bytes)
version: u8
quantization: u8 (0=float32, 1=float16, 2=int8)
architecture: u8 (0=bilstm_gru, 1=transformer_tiny, ...)
vocab_size: u32, embed_dim: u16, hidden_dim: u16, decoder_dim: u16
num_layers: u8, num_conventions: u16, max_steps: u8
[weight sections: name + shape + packed data]
```

### Memory Budget

| Target | Params | Quant | Model Size | Inference RAM | Conventions |
|--------|--------|-------|------------|---------------|-------------|
| ESP32 (no PSRAM) | ~50K | int8 | ~100KB | ~168KB | 8-16 |
| ESP32 (PSRAM) | ~200K | int8 | ~400KB | ~135KB SRAM + 2MB PSRAM | 16-32 |
| Raspberry Pi | ~800K | float16 | ~1.6MB | ~20MB | 32-64 |
| Desktop/Server | ~800K-50M | float32 | 3-200MB | 50MB-2GB | 64-500+ |

On ESP32 without PSRAM, the full inference stack (engine, tokenizer, encoder buffers, decoder state, attention, protocol, plugins, LoRA) fits in ~168KB of SRAM, leaving ~352KB for application use. Model weights live in flash via memory-mapped reads.

### LoRA on Embedded

Minimal: rank 2-4, applied only to output heads (opcode classifier, argument extractors). The GRU decoder is frozen. Consolidation (merge into flash-resident weights) is supported but infrequent (daily/weekly) to respect flash endurance (~100K write cycles).

Embedded SOMAs do not compute gradients locally. Instead, experiences are sent to a more powerful peer via Synaptic Protocol, which computes updated LoRA weights and sends them back ("learning by delegation").

---

## Tokenizer

`soma-core/src/mind/tokenizer.rs`. Auto-detected from `tokenizer.json` format.

### Two Modes

| Mode | Detection | Vocab Size | OOV Handling |
|------|-----------|------------|--------------|
| Word-level | Flat JSON `{"token": idx}` | 5K-50K | Maps to `<UNK>` |
| BPE | JSON with `"merges"` key | 1K-10K | Character-level fallback |

### BPE Algorithm

1. Split word into individual characters
2. Find the adjacent pair with the lowest merge index (highest priority)
3. Merge that pair into a single token
4. Repeat until no more merges apply
5. Map resulting subword tokens to vocabulary indices; unknowns map to `UNK_IDX`

### Key Properties

- Vocabulary: ~4,000 tokens (configurable at synthesis)
- Handles SQL (`SELECT`, `FROM`, `WHERE`), file paths, URLs via subword decomposition
- Case-insensitive: input is lowercased before tokenization
- `encode_with_null()` prepends a `<NULL>` token (index 2) for decoder span extraction offset

### Struct

```rust
pub struct Tokenizer {
    word2idx: HashMap<String, i64>,
    idx2word: HashMap<i64, String>,
    merges: Vec<(String, String)>,  // empty in word-level mode
}
```

Special token indices: `PAD_IDX = 0`, `UNK_IDX = 1`, `NULL_IDX = 2`.

### Embedded Tokenizer

For ESP32: smaller vocab (1,000-2,000 tokens), more aggressive BPE merges, vocab stored in flash. Character-level fallback is essential since embedded SOMAs encounter novel text.

---

## LoRA (Low-Rank Adaptation)

LoRA is SOMA's experiential memory mechanism. It allows the Mind to improve from successful executions without full retraining.

### LoRALayer

Defined in `soma-core/src/mind/lora.rs`:

```rust
pub struct LoRALayer {
    pub name: String,
    pub base_weight_shape: (usize, usize),  // (out_features, in_features)
    pub a: Vec<f32>,   // rank x in_features, row-major
    pub b: Vec<f32>,   // out_features x rank, row-major
    pub rank: usize,
    pub scale: f32,    // alpha / rank
}
```

### Forward Pass

For `nn.Linear` layers:

```
output = base(x) + scale * (x @ A.T) @ B.T
```

`B` is initialized to zero so LoRA has no initial effect. `A` is initialized to small random values (0.01). The `forward()` method computes the delta that gets added to base logits.

For `nn.GRUCell` (spec, not yet in runtime): LoRA on both `W_ih` and `W_hh` with reimplemented forward pass preserving correct gradient flow.

### Adaptation (Gradient Descent)

The `adapt()` method on `LoRALayer` performs in-place SGD:

1. For each (hidden_state, target_opcode) in the batch:
   - Compute `ha = hidden @ A.T` (rank-dimensional)
   - Compute `logits = base_logits + scale * ha @ B.T`
   - Softmax -> cross-entropy loss against target
   - Backprop gradients through B and A
2. Average gradients over batch
3. Update: `A -= lr * grad_A`, `B -= lr * grad_B`

Returns average loss over the batch.

### Magnitude Tracking

```rust
pub fn magnitude(&self) -> f32 {
    self.b.iter().map(|x| x.abs()).sum::<f32>() * self.scale
}
```

Magnitude measures how much a layer has adapted from its initial state. Used to determine consolidation eligibility.

### Weight Delta (for Consolidation)

```rust
pub fn compute_weight_delta(&self) -> Vec<f32>
```

Returns `scale * B @ A` as a flat `(out_features x in_features)` vector. This is the effective weight change that LoRA applies. Used during consolidation to accumulate into `merged_opcode_delta`.

### Serialization Types

```rust
pub struct LoRAWeights {
    pub name: String,
    pub rank: usize,
    pub scale: f32,
    pub a: Vec<f32>,
    pub b: Vec<f32>,
}

pub struct LoRACheckpoint {
    pub layers: Vec<LoRALayerState>,
    pub adaptation_count: u64,
    pub experience_count: u64,
}

pub struct LoRABundle {
    pub plugin_name: String,
    pub layers: Vec<LoRAWeights>,
}
```

`LoRABundle` is the wire format for plugin-provided LoRA weights. A plugin may provide LoRA layers for one or more output heads via `SomaPlugin::lora_weights()`.

### Plugin LoRA

Per-plugin LoRA weights are trained by the Synthesizer after the base Mind is trained. Process:

1. Freeze base Mind weights
2. Attach LoRA adapters to target layers
3. Train on ONLY that plugin's training examples
4. Export as `.lora` file (serialized `LoRABundle`)

At runtime, plugins provide their LoRA bundles via `attach_lora_bytes()`. Multiple plugin LoRAs are active simultaneously -- a future MoE gating network will dynamically weight which plugin's LoRA to activate per operation.

### Runtime Configuration

```toml
[mind.lora]
default_rank = 8
default_alpha = 16.0
adaptation_enabled = true
adapt_every_n_successes = 10
adapt_batch_size = 8
adapt_learning_rate = 0.001
max_lora_layers = 64
```

---

## Memory System

Defined in `soma-core/src/memory/`. Three submodules: `experience`, `checkpoint`, `consolidation`.

### Four-Tier Hierarchy

Inspired by complementary learning systems theory (McClelland et al., 1995) and sleep consolidation research (Diekelmann & Born, 2010).

| Tier | Biological Analogy | Implementation | Lifetime |
|------|-------------------|----------------|----------|
| Permanent | Neocortical long-term | Base model weights (ONNX) + `merged_opcode_delta` | Immutable until re-synthesis or consolidation |
| Experiential | Hippocampal recent | LoRA A/B matrices | Grows at runtime, checkpointable |
| Working | Active neural firing | Decoder hidden states, inference context | Per-execution, transient |
| Diffuse | Asking a colleague | Synaptic queries to peer SOMAs | Network-dependent (future) |

### Experience Recording

`soma-core/src/memory/experience.rs`:

```rust
pub struct Experience {
    pub intent_tokens: Vec<u32>,
    pub program: Vec<(i32, u8, u8)>,     // (conv_id, arg0_type, arg1_type)
    pub success: bool,
    pub execution_time_ms: u64,
    pub timestamp: Instant,
    pub cached_states: Vec<(Vec<f32>, Vec<f32>)>,  // (hidden, base_logits) per step
}
```

Only successful executions are recorded (Spec Section 17.1). Failed executions are not recorded -- the SOMA should not reinforce wrong programs. This may change with a more sophisticated adaptation strategy that includes negative examples.

### ExperienceBuffer

Ring buffer with configurable maximum size (default: 1000, set via `[memory] max_experience_buffer`).

```rust
pub struct ExperienceBuffer {
    buffer: Vec<Experience>,
    max_size: usize,
    total_seen: u64,
}
```

Methods: `record()` (evicts oldest if full), `successes()`, `failures()`, `recent(n)`, `success_count()`, `failure_count()`, `total_seen()`, `clear()`.

### Adaptation Loop

`soma-core/src/mind/adaptation.rs`. Configuration:

```rust
pub struct AdaptationConfig {
    pub enabled: bool,
    pub adapt_every_n: usize,
    pub batch_size: usize,
    pub learning_rate: f32,
}
```

The `adapt_from_experience()` function:

1. Samples up to `batch_size` experiences (most recent ones)
2. For each experience, uses `cached_states` if available (fast path -- no ONNX re-inference). Falls back to re-running encoder+decoder if cached states are missing.
3. Collects `(hidden_state, target_opcode)` pairs across all steps of all sampled experiences
4. Ensures LoRA layers exist for all 11 output heads; creates defaults (rank 8, alpha 16.0) if needed
5. Runs `LoRALayer::adapt()` on the opcode head
6. Returns `AdaptationResult { loss, cycle, lora_magnitude }`

Adaptation requires a write lock on the MindEngine. During steps 3-7 of the adaptation flow, inference is paused. This takes ~10-50ms on a server.

### Adaptation Triggers

| Trigger | Config Key | Default |
|---------|-----------|---------|
| Every N successes | `adapt_every_n_successes` | 10 |
| Manual command | intent: `"adapt now"` | -- |
| Scheduled | `adapt_schedule` | disabled |
| Experience buffer full | `adapt_on_buffer_full` | true |

### Checkpoint/Restore

`soma-core/src/memory/checkpoint.rs`. Version 2 format, backwards-compatible with v1 via `#[serde(default)]`.

```rust
pub struct Checkpoint {
    pub version: u32,                         // CHECKPOINT_VERSION = 2
    pub soma_id: String,
    pub timestamp: u64,
    pub lora_state: Vec<LoRALayerState>,
    pub experience_count: u64,
    pub adaptation_count: u64,
    pub plugin_states: Vec<PluginStateEntry>,  // plugin-specific state snapshots
    pub decisions: Vec<serde_json::Value>,     // decision log (institutional memory)
    pub recent_executions: Vec<serde_json::Value>,
    pub base_model_hash: String,               // SHA-256 of encoder+decoder ONNX
    pub plugin_manifest: Vec<PluginManifestEntry>,
    pub merged_opcode_delta: Vec<f32>,         // accumulated consolidation delta
}
```

Binary format: `"SOMA"` magic (4 bytes) + version u32 (4 bytes LE) + JSON body.

On restore, `base_model_hash` is compared against the currently loaded model. A mismatch warns that LoRA state may be incompatible (model was re-synthesized since the checkpoint was created).

Methods: `save(path)`, `load(path)`, `filename(soma_id)` (generates timestamped name), `list_checkpoints(dir)` (sorted newest-first), `prune_checkpoints(dir, max_keep)`.

### Configuration

```toml
[memory]
checkpoint_dir = "checkpoints"
auto_checkpoint = true
max_checkpoints = 5
max_experience_buffer = 1000
checkpoint_interval_secs = 3600
```

### Consolidation ("Sleep")

`soma-core/src/memory/consolidation.rs`. High-magnitude LoRA adaptations are permanently merged, making proven patterns part of permanent memory.

```rust
pub struct ConsolidationConfig {
    pub min_lora_magnitude: f32,  // default: 0.01
    pub threshold: u64,           // default: 100
}
```

`should_consolidate()` returns true when both conditions are met: adaptation count >= threshold AND max LoRA magnitude >= `min_lora_magnitude`.

The 5-step consolidation process:

1. Evaluate LoRA magnitude per layer
2. For each layer exceeding `min_lora_magnitude`: compute `delta = scale * B @ A`
3. Accumulate delta into `merged_opcode_delta`; reset the LoRA layer (`A = 0.01`, `B = 0.0`)
4. Caller creates checkpoint (new permanent state)
5. Log consolidation stats

After consolidation, `merged_opcode_delta` is applied during every subsequent inference as `logits += hidden @ merged_opcode_delta.T`. The effect is equivalent to having modified the base weights, but works with tract's frozen graph.

Consolidated knowledge cannot be un-learned. This is by design -- proven patterns become permanent.

### Consolidation Triggers

| Condition | Config Key |
|-----------|-----------|
| Experience count exceeds threshold | `consolidation.threshold` |
| LoRA magnitude exceeds threshold | `consolidation.min_lora_magnitude` |
| Manual command | intent: `"consolidate"` or `:consolidate` |
| Scheduled | `consolidation.schedule` |

### Configuration

```toml
[memory.consolidation]
enabled = true
trigger = "experience_count"
threshold = 500
min_lora_magnitude = 0.01
```

### Consolidation on Embedded

Writes to flash (~100ms per layer, consumes a flash write cycle). Should be infrequent -- daily or weekly -- to respect flash endurance limits (~100K write cycles).

---

## MindConfig

Runtime configuration passed during engine initialization:

```rust
pub struct MindConfig {
    pub max_program_steps: usize,
    pub temperature: f32,
    pub max_inference_time_secs: u64,
}
```

### MindInfo (Proprioception)

Returned by `MindEngine::info()` for self-model queries:

```rust
pub struct MindInfo {
    pub backend: String,          // e.g., "OnnxMindEngine (tract)"
    pub param_count: usize,       // estimated from model dimensions
    pub conventions_known: usize,
    pub max_steps: usize,
    pub lora_layers: usize,
    pub lora_magnitude: f32,      // sum of all active LoRA magnitudes
}
```

Exposed via MCP through `soma.get_experience()` and `soma.get_health()`.

---

## Transformer Variant (Future)

A stub exists in the synthesizer (`model.py: TransformerMind`), not yet implemented in the Rust runtime.

For SOMAs with 100+ conventions (web applications with many plugins), a Transformer encoder/decoder may perform better -- especially for long intents (>50 tokens) and complex programs (>8 steps). Architecture: 4-8 encoder layers, 4-8 decoder layers with cross-attention, 4-8 heads. Requires ~10-50M parameters vs ~1M for BiLSTM+GRU.

The `MindEngine` trait supports this transparently. A `TransformerMindEngine` would implement the same `infer()` method. The rest of the SOMA Core does not change.

---

## File Index

| File | Purpose |
|------|---------|
| `soma-core/src/mind/mod.rs` | `MindEngine` trait, `Program`, `ProgramStep`, `ArgType`, `ArgValue`, `MindConfig`, `MindInfo`, `ModelMeta` |
| `soma-core/src/mind/onnx_engine.rs` | `OnnxMindEngine`: tract-onnx inference, LoRA application, adaptation helpers |
| `soma-core/src/mind/tokenizer.rs` | `Tokenizer`: word-level and BPE modes, auto-detection |
| `soma-core/src/mind/lora.rs` | `LoRALayer`, `LoRAWeights`, `LoRACheckpoint`, `LoRABundle`, gradient descent |
| `soma-core/src/mind/adaptation.rs` | `adapt_from_experience()`, `AdaptationConfig`, `AdaptationResult` |
| `soma-core/src/memory/mod.rs` | Module declarations |
| `soma-core/src/memory/experience.rs` | `Experience`, `ExperienceBuffer` (ring buffer) |
| `soma-core/src/memory/checkpoint.rs` | `Checkpoint` (v2 format), save/load/prune |
| `soma-core/src/memory/consolidation.rs` | `ConsolidationConfig`, `consolidate()` |
| `soma-core/soma.toml.example` | All `[mind]`, `[mind.lora]`, `[memory]`, `[memory.consolidation]` config fields |
