# SOMA Core Refactoring Specification

**Status:** Design  
**Depends on:** Nothing (this is the foundation)  
**Blocks:** Everything else

---

## 1. Guiding Principle

The SOMA Core is the minimal, universal kernel that makes a SOMA a SOMA. Everything domain-specific is a plugin. The core should be small enough to run on an ESP32 and powerful enough to orchestrate a cloud backend.

**The core is NOT:** a web server, a database client, a renderer, a file handler, or anything application-specific.

**The core IS:** a neural mind that generates programs, a protocol for communication, a plugin system for extensible capabilities, and a memory system that grows from experience.

---

## 2. Runtime Language: Rust

### Why Rust

- **Single binary.** `./soma` — one file, no runtime, no dependencies, no pip, no node_modules. The SOMA philosophy applied to itself.
- **Performance.** No garbage collector pauses during real-time signal processing. Zero-cost abstractions. Predictable latency.
- **True concurrency.** No GIL. Async runtime (tokio) handles thousands of concurrent synaptic connections.
- **Memory safety.** No segfaults, no buffer overflows. Critical for a system that directly controls hardware.
- **Cross-compilation.** Same codebase compiles to x86-64, ARM64, RISC-V, and ESP32 (via `no_std` embedded Rust).
- **Small binaries.** A minimal SOMA binary can be under 5MB. With plugins, 10-20MB. Compare to Python+PyTorch at 2GB+.

### Python Stays for Synthesis Only

The Synthesizer (training tool) remains Python + PyTorch. It is a BUILD tool, not a runtime component. Models are exported to ONNX format after training. The Rust runtime loads ONNX models via the `ort` crate (ONNX Runtime bindings).

```
[Python + PyTorch]        [Rust + ONNX Runtime]
   Synthesizer      →      SOMA Binary
   (build time)             (runtime)
   
   Trains model             Loads .onnx model
   Exports to ONNX          Runs inference
   Produces artifacts        Handles plugins
                             Speaks Synaptic Protocol
                             Manages memory/LoRA
```

### Key Rust Crates

| Crate | Purpose |
|---|---|
| `ort` | ONNX Runtime inference (replaces PyTorch at runtime) |
| `tokio` | Async runtime for concurrent signal handling |
| `serde` / `serde_json` / `rmp-serde` | Signal serialization (JSON + MessagePack) |
| `libloading` | Dynamic plugin loading (.so/.dylib/.dll) |
| `memmap2` | Memory-mapped model files for fast loading |
| `tracing` | Structured logging and diagnostics |

---

## 3. Core Architecture

```
┌─────────────────────────────────────────────┐
│  SOMA Core Binary                            │
│                                              │
│  ┌─────────────┐  ┌──────────────────────┐  │
│  │  Mind Engine │  │  Synaptic Protocol   │  │
│  │  (ONNX       │  │  Server + Client     │  │
│  │   inference)  │  │  (TCP/Unix socket)   │  │
│  └──────┬──────┘  └──────────┬───────────┘  │
│         │                     │              │
│  ┌──────┴─────────────────────┴───────────┐  │
│  │  Plugin Manager                         │  │
│  │  (discover, load, compose, route)       │  │
│  └──────┬─────────────────────────────────┘  │
│         │                                    │
│  ┌──────┴──────┐  ┌──────────────────────┐  │
│  │  Memory     │  │  Proprioception      │  │
│  │  (LoRA +    │  │  (self-model,        │  │
│  │  checkpoint) │  │   capabilities,      │  │
│  │             │  │   state tracking)     │  │
│  └─────────────┘  └──────────────────────┘  │
│                                              │
└─────────────────────────────────────────────┘
         │
    ┌────┴────┐
    │ Plugins │  (.so/.dylib files OR built-in)
    │ loaded  │
    │ at      │
    │ runtime │
    └─────────┘
```

---

## 4. Mind Engine

### 4.1 Responsibilities

- Load model(s) from disk (ONNX or native format depending on target)
- Run inference: tokenize intent → encode → decode program → extract arguments
- Manage LoRA adaptation layers (experiential memory)
- Scale from ESP32 (200KB RAM) to cloud server (32GB+ RAM) using the same abstract interface

### 4.2 Dual Backend Architecture

The Mind Engine defines an abstract trait. Two backends implement it — one for server/desktop, one for embedded. The rest of the SOMA Core doesn't know which backend is running.

```rust
pub trait MindEngine: Send + Sync {
    /// Load model from path
    fn load(&mut self, model_path: &Path, config: &MindConfig) -> Result<()>;
    
    /// Run full inference: intent text → program steps
    fn infer(&self, tokens: &[u32], length: usize) -> Result<Program>;
    
    /// Get model metadata (param count, conventions known, etc.)
    fn info(&self) -> MindInfo;
    
    /// LoRA operations
    fn attach_lora(&mut self, name: &str, weights: &LoRAWeights) -> Result<()>;
    fn detach_lora(&mut self, name: &str) -> Result<()>;
    fn merge_lora(&mut self, name: &str) -> Result<()>;
    fn checkpoint_lora(&self) -> Result<LoRACheckpoint>;
    fn restore_lora(&mut self, checkpoint: &LoRACheckpoint) -> Result<()>;
}
```

```
┌─────────────────────────────────────────────┐
│  MindEngine trait (abstract)                 │
├─────────────────────┬───────────────────────┤
│  OnnxMindEngine     │  EmbeddedMindEngine   │
│  (server / desktop) │  (ESP32 / MCU)        │
│                     │                       │
│  Uses: ort crate    │  Uses: no_std Rust    │
│  (ONNX Runtime)     │  custom inference     │
│                     │                       │
│  Models: .onnx      │  Models: .soma-model  │
│  (float32/float16)  │  (int8 quantized)     │
│                     │                       │
│  RAM: 50MB-8GB+     │  RAM: 200-400KB       │
│  Any OS             │  Bare metal / RTOS    │
│  Dynamic plugins    │  Built-in plugins     │
│  Full LoRA (rank    │  Minimal LoRA (rank   │
│   4-64, all layers) │   2-4, output heads)  │
└─────────────────────┴───────────────────────┘
```

### 4.3 Backend 1: OnnxMindEngine (Server / Desktop)

For macOS, Linux, Windows — any system with an OS and at least 50MB RAM.

**Model format:** Standard ONNX. The synthesizer exports:

| File | Purpose | Input | Output |
|---|---|---|---|
| `encoder.onnx` | Intent encoding | token_ids, lengths | encoder_output, pooled |
| `decoder.onnx` | Program generation (one step) | prev_op, context, hidden | op_logits, arg_logits, new_hidden |
| `tokenizer.json` | Vocabulary | — | word→id mapping |

**Inference:** The `ort` crate (Rust bindings to ONNX Runtime) handles model loading, session management, and GPU/NPU acceleration where available. The decoder runs autoregressively: one step per program operation, looping until STOP is predicted.

**LoRA:** Full-rank LoRA on all target layers. LoRA weights are stored as separate tensors and composed with base weights during inference. ONNX Runtime's custom operator support allows injecting LoRA computation into the inference graph.

**Target hardware:**

| Platform | Acceleration |
|---|---|
| macOS Apple Silicon | CoreML via ONNX Runtime |
| Linux + NVIDIA GPU | CUDA via ONNX Runtime |
| Linux CPU | OpenVINO or native ONNX CPU |
| Windows | DirectML via ONNX Runtime |
| Cloud (AWS/GCP) | GPU instances, ONNX serving |

### 4.4 Backend 2: EmbeddedMindEngine (ESP32 / Microcontrollers)

For bare metal or RTOS targets with 200KB-4MB RAM and no OS-level inference runtime.

**Why ONNX Runtime doesn't work here:**

- ONNX Runtime minimum footprint: ~50MB RAM, requires OS
- ESP32 has 520KB SRAM (some + 2-8MB PSRAM)
- ESP32 runs bare metal or FreeRTOS, not Linux

**Solution:** A custom, minimal inference engine written in `no_std` Rust. No allocator dependency for the core math. No external libraries. Just matrix operations, activations, and argmax.

The POW1 architecture (BiLSTM encoder + GRU decoder) uses only these operations:

| Operation | Implementation |
|---|---|
| Embedding lookup | Array index into weight table |
| LSTM forward | Matrix multiply + sigmoid + tanh + element-wise ops |
| GRU forward | Matrix multiply + sigmoid + tanh + element-wise ops |
| Linear projection | Matrix multiply + bias add |
| Softmax | Exp + sum + divide |
| Argmax | Iterate and track max |
| Mean pooling | Sum + divide |
| Attention (dot product) | Matrix multiply + mask + softmax |

This is ~3-5K lines of Rust. No framework. No runtime. Pure math.

**Model format:** Custom `.soma-model` binary format:

```
soma-model format:
  magic: "SOMA" (4 bytes)
  version: u8
  quantization: u8 (0=float32, 1=float16, 2=int8)
  architecture: u8 (0=bilstm_gru, 1=transformer_tiny, ...)
  
  vocab_size: u32
  embed_dim: u16
  hidden_dim: u16
  decoder_dim: u16
  num_layers: u8
  num_conventions: u16
  max_steps: u8
  
  [weight sections]
  section_count: u16
  for each section:
    name_len: u8
    name: [u8; name_len]
    shape: [u16; ndim]
    data: [u8 or i8 or f16] (row-major, packed)
```

**Quantization:** The synthesizer quantizes to int8 during export:

```
# Synthesis for ESP32
soma synthesize \
  --target esp32 \
  --quantize int8 \
  --max-ram 256kb \
  --max-flash 4mb \
  --embed-dim 32 \
  --hidden-dim 64 \
  --decoder-dim 128 \
  --max-conventions 16
```

This produces a model that's ~200KB-1MB depending on vocabulary size and convention count.

**Memory budget for ESP32 (520KB SRAM, no PSRAM):**

| Component | RAM Usage |
|---|---|
| EmbeddedMindEngine code | ~20KB |
| Tokenizer (vocab lookup) | ~10KB |
| Encoder hidden states (2 layers, bidirectional) | ~32KB |
| Decoder hidden state + context | ~16KB |
| Attention buffers | ~8KB |
| Program output buffer | ~2KB |
| Synaptic Protocol (minimal) | ~15KB |
| Plugin Manager + built-in plugins | ~20KB |
| LoRA layers (rank 2, output heads only) | ~15KB |
| Stack + overhead | ~30KB |
| **Total** | **~168KB** |
| **Remaining for application** | **~352KB** |

Model weights are stored in flash (not RAM) and accessed via memory-mapped reads. Only the active computation buffers live in SRAM.

**Memory budget for ESP32 with PSRAM (520KB SRAM + 4MB PSRAM):**

| Component | SRAM | PSRAM |
|---|---|---|
| Hot buffers (active inference) | ~100KB | — |
| Model weights (memory-mapped) | — | ~1-2MB |
| LoRA layers (higher rank) | — | ~50KB |
| Larger vocabulary | — | ~30KB |
| Synaptic Protocol buffers | ~15KB | — |
| Plugins | ~20KB | ~50KB |
| **Total** | **~135KB** | **~2.1MB** |

PSRAM variant supports larger models, more conventions, and richer LoRA adaptation.

**LoRA on embedded:** Minimal — rank 2-4, applied only to output heads (opcode classifier, argument extractors). The GRU decoder is frozen. This limits adaptation capacity but keeps RAM usage under control. Consolidation (merge) is still supported — LoRA merges into flash-resident weights during a sleep cycle (requires flash write, which ESP32 supports).

### 4.5 Model Size Comparison

| Target | Params | Quant | Model Size | RAM for Inference | Conventions |
|---|---|---|---|---|---|
| ESP32 (no PSRAM) | ~50K | int8 | ~100KB | ~168KB | 8-16 |
| ESP32 (PSRAM) | ~200K | int8 | ~400KB | ~135KB SRAM + 2MB PSRAM | 16-32 |
| Raspberry Pi | ~800K | float16 | ~1.6MB | ~20MB | 32-64 |
| Desktop/Server | ~800K-50M | float32 | 3MB-200MB | 50MB-2GB | 64-500+ |
| Cloud (large) | 50M-200M | float32/16 | 200MB-800MB | 2-8GB | 500+ |

The synthesizer scales model architecture to the target. Same conceptual design (BiLSTM+GRU or Transformer), different dimensions.

### 4.6 Synthesizer Export Targets

The Python synthesizer is the ONLY component that uses PyTorch. It exports to both formats:

```python
# After training
model = SomaMind(...)
train(model, data)

# Export for server/desktop
torch.onnx.export(model.encoder, ..., "encoder.onnx")
torch.onnx.export(model.decoder_step, ..., "decoder.onnx")

# Export for embedded
export_soma_model(model, 
    path="model.soma-model",
    quantize="int8",
    target_ram=256_000,  # bytes
    target_flash=4_000_000)
```

Both exports come from the same trained model. The embedded export is a quantized, dimension-reduced version of the server export. Behavior should be equivalent (within quantization tolerance).

### 4.7 LoRA in Rust (Both Backends)

Both backends share the same LoRA abstraction:

```rust
pub struct LoRALayer {
    base_weight: Tensor,      // frozen, from model file
    lora_a: Tensor,           // trainable, rank x in_features
    lora_b: Tensor,           // trainable, out_features x rank
    scale: f32,               // alpha / rank
}

impl LoRALayer {
    pub fn forward(&self, x: &Tensor) -> Tensor {
        let base = x.matmul(&self.base_weight.t());
        let lora = x.matmul(&self.lora_a.t()).matmul(&self.lora_b.t()) * self.scale;
        base + lora
    }
    
    pub fn merge(&mut self) {
        // Consolidation: LoRA → permanent memory
        self.base_weight += &(self.lora_b.matmul(&self.lora_a) * self.scale);
        self.lora_a.fill_randn(0.01);
        self.lora_b.fill_zero();
    }
    
    pub fn magnitude(&self) -> f32 {
        // How much has this layer adapted?
        (self.lora_b.matmul(&self.lora_a) * self.scale).norm()
    }
}
```

On embedded, `Tensor` is a fixed-size array with int8 values and fixed-point arithmetic. On server, it's a float32/float16 dynamic tensor backed by ONNX Runtime or ndarray.

LoRA adaptation (gradient update) uses:
- **Server:** Minimal autograd in Rust, or Python sidecar for complex adaptation
- **Embedded:** Pre-computed LoRA updates sent from a peer SOMA (e.g., a more powerful SOMA trains the adaptation and sends the weights via Synaptic Protocol). The ESP32 doesn't compute gradients — it receives updated LoRA weights and applies them.

### 4.8 Inference Pipeline (Both Backends)

The pipeline is identical regardless of backend:

```
Intent text ("list files in /tmp")
       │
  [Tokenizer — vocab lookup]
       │
  [Encoder — model inference → encoder_output]
       │                                        
  [Decoder loop — autoregressive]               
       │  step 0: START token → predict op, args
       │  step 1: prev_op → predict op, args    
       │  ...                                   
       │  step N: STOP predicted → exit loop    
       │
  [Program — list of (plugin_id, convention_id, arg_types, arg_values)]
       │
  [Plugin Manager — route each step to the appropriate plugin]
       │
  [Results collected, EMIT/SEND to requester]
```

The only difference: OnnxMindEngine calls `ort::Session::run()` per step. EmbeddedMindEngine calls hand-written matrix operations per step. The output is the same: a program of convention calls.

### 4.9 Future: Transformer Backend

The BiLSTM+GRU architecture was chosen for POWs because it's simple and proven. For larger SOMA instances (web applications, complex domains), a Transformer-based architecture may perform better — especially for long intents and complex program generation.

The MindEngine trait supports this transparently. A TransformerMindEngine would implement the same `infer()` method, using attention-based encoding and decoding instead of recurrent networks. The rest of the SOMA Core doesn't change.

For embedded Transformers: TinyBERT, DistilBERT, and MobileBERT demonstrate that Transformer models can be compressed to run on edge devices. The synthesizer would export a quantized, pruned Transformer in `.soma-model` format.

---

## 5. Plugin Manager

### 5.1 Responsibilities

- Discover available plugins (scan plugin directory)
- Load plugins at startup or runtime (dynamic .so/.dylib loading)
- Maintain a unified calling convention catalog (merged from all loaded plugins)
- Route program steps to the correct plugin based on convention ID
- Handle plugin lifecycle (load, unload, hot-reload)

### 5.2 Plugin Interface

Every plugin implements a standard trait (Rust interface):

```rust
pub trait SomaPlugin: Send + Sync {
    /// Plugin identity
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    
    /// Calling conventions this plugin provides
    fn conventions(&self) -> Vec<CallingConvention>;
    
    /// Execute a calling convention
    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;
    
    /// LoRA knowledge this plugin provides (optional)
    fn lora_weights(&self) -> Option<LoRAWeights>;
    
    /// Lifecycle
    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError>;
    fn on_unload(&mut self) -> Result<(), PluginError>;
}
```

### 5.3 Plugin Loading

Plugins can be:

- **Built-in:** Compiled into the SOMA binary (for embedded targets where dynamic loading isn't available).
- **Dynamic:** Loaded from `.so` (Linux), `.dylib` (macOS), `.dll` (Windows) files at runtime.
- **Remote:** Loaded from a plugin registry (future — download and cache plugins on demand).

### 5.4 Convention Catalog

When plugins load, their conventions are merged into a global catalog. Each convention gets a globally unique ID within this SOMA instance. The mind's model knows convention IDs from synthesis — so plugins must match the IDs the model was synthesized with, OR the model must be re-synthesized when plugins change.

Resolution: use named conventions. The model predicts convention NAMES (or name hashes), and the plugin manager resolves names to runtime IDs. This decouples model training from plugin loading order.

### 5.5 Plugin Composition

Multiple plugins can be active simultaneously. The Mind generates programs that span plugins:

```
Step 0: [PostgreSQL plugin] query("SELECT * FROM contacts")
Step 1: [Redis plugin] cache_set("contacts:list", $0, ttl=300)
Step 2: [EMIT] $0
```

The Plugin Manager routes each step to the correct plugin. Plugins don't know about each other. The Mind orchestrates.

---

## 6. Memory System

### 6.1 Memory Hierarchy (from Whitepaper Section 12)

| Type | Implementation | Lifetime |
|---|---|---|
| Permanent | ONNX base weights | Immutable until re-synthesis |
| Experiential | LoRA A/B matrices | Grows at runtime, checkpointable |
| Working | Decoder hidden states | Per-execution, transient |
| Diffuse | Synaptic queries to other SOMAs | Network-dependent |

### 6.2 Checkpoint Format

```
soma_checkpoint.bin:
  magic: "SOMA" (4 bytes)
  version: u32
  base_model_hash: [u8; 32]     // SHA-256 of ONNX models
  plugin_manifest: [PluginInfo]   // which plugins were loaded
  lora_layers: [LoRAState]        // all LoRA A/B matrices
  experience_stats: ExperienceStats
  metadata: {timestamp, soma_id, custom_fields}
```

### 6.3 Consolidation ("Sleep")

Triggered by: explicit command, experience count threshold, scheduled timer, or low-activity period.

Process:
1. Evaluate LoRA magnitude per layer (how much has changed)
2. Merge high-magnitude, stable layers into base weights
3. Reset merged LoRA layers
4. Create checkpoint
5. Report consolidation stats via proprioception

---

## 7. Proprioception

### 7.1 What the SOMA Knows About Itself

- Loaded plugins and their conventions
- Current LoRA magnitude (how much it has adapted)
- Experience count and adaptation cycle count
- Memory usage and computational load
- Connected peers (via Synaptic Protocol)
- Uptime and execution stats (success/error counts)

### 7.2 Queryable via Intent

```
"what can you do" → lists all loaded plugin conventions
"how much have you learned" → LoRA magnitude + experience stats
"who are you connected to" → peer list from Synaptic Protocol
"how are you doing" → health metrics (memory, CPU, error rate)
```

---

## 8. Binary Structure

### 8.1 Build Variants

| Target | Mind Backend | Plugins | Features | Binary Size (est.) |
|---|---|---|---|---|
| `soma-server` | OnnxMindEngine | Dynamic (.so) | Full async networking, ONNX Runtime, full LoRA | ~15MB |
| `soma-desktop` | OnnxMindEngine | Dynamic (.dylib) | Full + GUI plugin support | ~20MB |
| `soma-embedded` | EmbeddedMindEngine | Built-in only | no_std, int8 inference, minimal LoRA, minimal Synaptic | ~200KB-2MB |
| `soma-rpi` | OnnxMindEngine (CPU) | Dynamic (.so) | Server variant, ARM-optimized, lighter ONNX | ~10MB |

The embedded variant compiles with `#![no_std]` and does not link ONNX Runtime, tokio, or any OS-dependent crate. It uses `embassy` or bare-metal async for the Synaptic Protocol listener.

### 8.2 Directory Structure

```
soma/
  src/
    main.rs              # Entry point, CLI
    mind/
      mod.rs             # MindEngine trait definition
      onnx_engine.rs     # OnnxMindEngine (server/desktop)
      embedded_engine.rs # EmbeddedMindEngine (ESP32/MCU, no_std)
      tokenizer.rs       # Vocabulary + tokenization
      lora.rs            # LoRA layer management (shared by both engines)
      tensor.rs          # Minimal tensor ops (for embedded, no_std)
    protocol/
      signal.rs          # Signal type definitions
      server.rs          # Synaptic Protocol listener (tokio for server, embassy for embedded)
      client.rs          # Synaptic Protocol sender
      discovery.rs       # Peer discovery
      codec.rs           # Binary encode/decode
    plugin/
      manager.rs         # Plugin loading, catalog, routing
      interface.rs       # SomaPlugin trait definition
      builtin/           # Built-in plugins (for embedded targets)
    memory/
      checkpoint.rs      # Serialize/restore mind state
      experience.rs      # Experience buffer + adaptation
      consolidation.rs   # Sleep cycle (LoRA merge)
    proprioception/
      self_model.rs      # Self-knowledge queries
  plugins/               # External plugin sources (each is its own crate)
    postgres/
    redis/
    dom_renderer/
    gpio/                # ESP32 GPIO plugin
    i2c/                 # ESP32 I2C plugin
    ...
  models/                # Exported models
    server/
      encoder.onnx
      decoder.onnx
      tokenizer.json
    embedded/
      model.soma-model
      tokenizer.json
  Cargo.toml
```

### 8.3 CLI Interface

```
soma [OPTIONS]

Options:
  --plugins <dir>       Plugin directory (default: ./plugins/)
  --model <dir>         Model directory (default: ./models/)
  --bind <addr:port>    Synaptic Protocol listen address
  --peer <addr:port>    Connect to peer SOMA
  --checkpoint <file>   Restore from checkpoint
  --config <file>       Configuration file
  --repl                Interactive intent REPL
  --log-level <level>   Logging verbosity
```

---

## 9. Migration from Python POWs

### 9.1 Phase 1: Export Models to Both Formats

```python
# In Python synthesizer
model = SomaMind(...)
train(model, data)

# Export for server/desktop (ONNX)
torch.onnx.export(model.encoder, ..., "encoder.onnx")
torch.onnx.export(model.decoder_step, ..., "decoder.onnx")

# Export for embedded (.soma-model, int8 quantized)
export_soma_model(model, "model.soma-model", quantize="int8",
                  target_ram=256_000, target_flash=4_000_000)
```

### 9.2 Phase 2: Rust Core with Python Plugins

Initially, plugins can be Python scripts called via subprocess or FFI. This allows reusing existing POW code while migrating to native Rust plugins incrementally.

### 9.3 Phase 3: Native Rust Plugins

Replace Python plugins one by one with native Rust implementations. PostgreSQL via `tokio-postgres`. Redis via `redis-rs`. DOM via WebAssembly. Each migration improves performance and removes Python dependency.

### 9.4 Phase 4: Full Native

Remove Python runtime dependency entirely. The SOMA is a single Rust binary + ONNX models + plugin .so files.

---

## 10. Testing Strategy

### 10.1 Unit Tests

- Mind Engine: verify ONNX inference produces same output as PyTorch
- LoRA: verify merge/checkpoint/restore preserves weights exactly
- Plugin Manager: verify convention routing, dynamic loading
- Synaptic Protocol: verify signal encode/decode roundtrip

### 10.2 Integration Tests

- Full pipeline: intent → mind → plugin → result
- Multi-plugin programs: steps spanning different plugins
- Checkpoint cycle: execute → checkpoint → kill → restore → verify state
- Consolidation: execute → adapt → consolidate → verify permanence

### 10.3 Compatibility Tests

- ONNX model compatibility between Python export and Rust OnnxMindEngine inference
- `.soma-model` compatibility between Python export and Rust EmbeddedMindEngine inference
- Quantization tolerance: int8 embedded model produces same program as float32 server model for all test intents
- Plugin ABI compatibility across Rust compiler versions
- Cross-platform: same model + plugins work on x86 and ARM

### 10.4 Embedded-Specific Tests

- Memory usage stays within budget (measured with ESP32 simulator or real hardware)
- Inference latency on ESP32: target <500ms for simple intents, <2s for complex
- Flash wear: consolidation (LoRA merge → flash write) frequency stays within flash endurance limits
- LoRA weight transfer: receive LoRA update via Synaptic Protocol from peer SOMA, apply, verify behavior change
- Minimal Synaptic Protocol: verify signal exchange works with reduced buffer sizes

---

## 11. Startup Sequence

### 11.1 Boot Order

```
1. Parse CLI arguments and load config file
         │
2. Initialize logging (tracing subscriber)
         │
3. Load Mind Engine
   ├── Detect target (server → Onnx, embedded → Embedded)
   ├── Load model files (ONNX or .soma-model)
   ├── Load tokenizer
   └── Verify: model loads, test inference on "ping" → expect valid program
         │
4. Load Plugins
   ├── Scan plugin directory (or use built-in list for embedded)
   ├── For each plugin:
   │   ├── Load .so/.dylib (or init built-in)
   │   ├── Call plugin.on_load(config)
   │   ├── Register conventions in catalog
   │   └── If plugin has LoRA knowledge → attach to Mind
   ├── Build global convention catalog (name → runtime ID mapping)
   └── Verify: all conventions the model expects are available
         │
5. Restore Checkpoint (if --checkpoint flag or auto-checkpoint found)
   ├── Load checkpoint file
   ├── Verify base_model_hash matches loaded model
   ├── Restore LoRA states to Mind
   └── Log: restored experience stats
         │
6. Start Synaptic Protocol
   ├── Bind listener on configured address:port
   ├── Connect to known peers (from --peer flags or config)
   ├── Broadcast discovery to peers
   └── Begin accepting incoming connections
         │
7. Ready
   ├── Log: "SOMA ready. Plugins: N, Conventions: M, Peers: P"
   ├── If --repl: start interactive REPL
   └── Enter main event loop
```

### 11.2 Failure Handling During Boot

| Failure | Behavior |
|---|---|
| Model file missing/corrupt | Fatal. Exit with error. Cannot operate without a mind. |
| Plugin fails to load | Warning. Skip plugin. Continue with remaining plugins. Log which conventions are unavailable. |
| Plugin's LoRA weights incompatible | Warning. Load plugin without LoRA. Mind can still use conventions but without pre-trained knowledge. |
| Checkpoint file corrupt | Warning. Start fresh (no experiential memory). Log that checkpoint was skipped. |
| Checkpoint model hash mismatch | Warning. Checkpoint was for a different model version. Start fresh. |
| Synaptic bind fails (port in use) | Fatal for server. Retry with backoff. For embedded, continue without networking if plugin doesn't require it. |
| Peer connection fails | Warning. Retry in background. Not fatal — peers may come online later. |
| Convention mismatch (model expects conventions not available) | Warning. Log missing conventions. Mind may generate programs with unavailable steps — these fail at execution time with clear error. |

### 11.3 Hot-Reload

After initial boot, plugins can be loaded/unloaded at runtime:

```
intent> "load the stripe plugin"
  [Plugin Manager] Loading stripe.so...
  [Plugin Manager] 8 conventions registered
  [LoRA Manager] Attaching Stripe LoRA
  [Catalog] Rebuilt: 24 → 32 conventions

intent> "unload the redis plugin"
  [Plugin Manager] Calling redis.on_unload()...
  [Plugin Manager] 6 conventions removed
  [Catalog] Rebuilt: 32 → 26 conventions
```

The Mind may generate programs referencing conventions that no longer exist after unload. These fail at execution time with a clear "convention unavailable" error, triggering the retry/adapt loop (Section 13).

---

## 12. Concurrency Model

### 12.1 The Problem

The Mind's GRU decoder maintains hidden state across program steps. If two intents arrive simultaneously, they cannot share decoder state — they'd corrupt each other's programs.

### 12.2 Solution: Per-Request Inference Context

Each incoming intent gets its own inference context:

```rust
pub struct InferenceContext {
    encoder_output: Tensor,     // shared read-only after encoding
    decoder_hidden: Tensor,     // per-request, mutable
    program_steps: Vec<ProgramStep>,
    results: Vec<Value>,        // step results for ref resolution
}
```

The encoder is stateless (same input → same output) and can be shared. The decoder state is per-request. LoRA weights are shared (read-only during inference, written only during adaptation which holds a write lock).

### 12.3 Server Concurrency

```rust
// Server: tokio async, multiple concurrent requests
let mind = Arc<RwLock<MindEngine>>;

async fn handle_intent(mind: Arc<RwLock<MindEngine>>, intent: String) -> Program {
    let mind_read = mind.read().await;  // shared read lock
    let context = mind_read.create_context();
    let program = mind_read.infer_with_context(&context, &tokens);
    program
}

// Adaptation takes a write lock (blocks inference briefly)
async fn adapt(mind: Arc<RwLock<MindEngine>>, experiences: Vec<Experience>) {
    let mut mind_write = mind.write().await;  // exclusive write lock
    mind_write.apply_lora_update(&experiences);
}
```

Multiple intents execute simultaneously. Adaptation briefly pauses inference (write lock). On a server handling 100 requests/sec, adaptation once per 5 seconds causes ~10ms pause — negligible.

### 12.4 Embedded Concurrency

ESP32 is typically single-core (or dual-core with one core for WiFi). Intents are processed sequentially. No concurrent inference. The async runtime (embassy) handles Synaptic Protocol I/O concurrently, but Mind inference is single-threaded and blocking.

```
[Synaptic signal arrives]
  → [Queue intent]
  → [Process queue: one intent at a time]
  → [Send result]
```

This is acceptable because embedded SOMAs handle far fewer concurrent requests (typically one user, one device).

### 12.5 Plugin Execution Concurrency

Plugin execution (database queries, network calls) is async. Multiple program steps can execute concurrently IF they don't have dependencies (no ref chains between them). However, the initial implementation executes steps sequentially for simplicity. Parallel step execution is an optimization for later.

---

## 13. Error Handling Strategy

### 13.1 Error Categories

| Category | Examples | Response |
|---|---|---|
| **Inference error** | Model produces invalid opcode, NaN in logits, decoder loop doesn't terminate | Return error signal to requester. Log diagnostics. Do NOT adapt from this experience. |
| **Plugin error** | Database connection refused, file not found, network timeout | Retry with variation (Section 13.3). Report to requester if all retries fail. |
| **Protocol error** | Malformed signal, checksum mismatch, unknown signal type | Drop signal. Log warning. Send ERROR signal to sender if identifiable. |
| **Resource error** | Out of memory, disk full, too many connections | Reject new requests with backpressure signal. Continue serving existing connections. Alert via proprioception. |
| **Panic** | Rust panic in plugin code, integer overflow, assertion failure | Catch at plugin boundary (catch_unwind). Unload crashed plugin. Log. Continue operating with reduced capabilities. |

### 13.2 Error Propagation

```rust
pub enum SomaError {
    Inference(InferenceError),      // mind failed
    Plugin(String, PluginError),    // (plugin_name, error)
    Protocol(ProtocolError),        // signal handling failed
    Resource(ResourceError),        // system resource exhaustion
    Convention(ConventionError),    // requested convention not available
}

// Every error carries context for diagnostics
pub struct PluginError {
    pub convention: String,
    pub step_index: usize,
    pub message: String,
    pub retryable: bool,
    pub suggestion: Option<String>,  // "check database connection", etc.
}
```

### 13.3 Retry with Variation (Whitepaper Section 11.2)

When a program step fails and the error is retryable:

```
Step 2: postgres.query("SELECT * FROM contacts") → Error: connection refused

Retry strategy:
  1. Wait 100ms → retry same step
  2. If still fails → Mind re-infers from the same intent 
     (may produce a different program due to softmax temperature)
  3. If still fails → degrade: skip the failed step, 
     attempt to produce partial result
  4. If nothing works → report error to requester with explanation:
     "I tried to query the database but couldn't connect. 
      Check if PostgreSQL is running."
```

The retry logic lives in the Core, not in plugins. Plugins return errors with `retryable: bool`. The Core decides the retry strategy.

### 13.4 Graceful Degradation

If a plugin becomes unavailable during operation:

```
Redis plugin crashes → 
  Core catches panic, unloads plugin →
  Subsequent programs that reference Redis conventions fail →
  Mind's feedback layer (experiential memory) learns to avoid Redis-dependent programs →
  SOMA continues operating with reduced capabilities →
  Proprioception reports: "Redis plugin unavailable. Caching disabled."
```

The SOMA never crashes entirely because a plugin failed. It degrades, adapts, and reports.

---

## 14. Signal Routing

### 14.1 The Problem

Incoming Synaptic signals have different destinations:

- **INTENT** → needs Mind inference → produces a program → executes
- **DATA** (response to a previous request) → goes to a waiting task
- **STREAM_DATA** → goes directly to the plugin handling that stream
- **DISCOVER** → handled by protocol layer
- **CHUNK_DATA** → goes to the chunk reassembly buffer
- **SUBSCRIBE** → registers a channel subscription
- **PING** → immediate PONG response

The Core needs a router that inspects signal type and dispatches correctly.

### 14.2 Router Architecture

```rust
pub struct SignalRouter {
    mind: Arc<RwLock<MindEngine>>,
    plugin_manager: Arc<PluginManager>,
    pending_requests: DashMap<u64, oneshot::Sender<Signal>>,
    stream_handlers: DashMap<u32, mpsc::Sender<Signal>>,  // channel_id → handler
    chunk_buffers: DashMap<u32, ChunkReassembly>,
    subscriptions: DashMap<u32, Vec<SynapseConnection>>,
}

impl SignalRouter {
    pub async fn route(&self, signal: Signal, connection: &SynapseConnection) {
        match signal.signal_type {
            // Protocol-level: handle immediately
            SignalType::Ping => connection.send(Signal::pong()).await,
            SignalType::Discover => self.handle_discovery(signal, connection).await,
            SignalType::DiscoverAck => self.update_peer_info(signal).await,
            SignalType::Handshake => self.handle_handshake(signal, connection).await,
            SignalType::Close => self.handle_close(connection).await,
            
            // Intent: needs Mind inference
            SignalType::Intent => self.handle_intent(signal, connection).await,
            
            // Data: check if it's a response to a pending request
            SignalType::Data | SignalType::Result => {
                if let Some(waiter) = self.pending_requests.remove(&signal.sequence) {
                    let _ = waiter.1.send(signal);
                } else {
                    // Unsolicited data — route to appropriate plugin
                    self.handle_unsolicited_data(signal).await;
                }
            }
            
            // Streaming: route to channel handler
            SignalType::StreamStart => self.open_stream(signal).await,
            SignalType::StreamData => {
                if let Some(handler) = self.stream_handlers.get(&signal.channel_id) {
                    let _ = handler.send(signal).await;
                }
            }
            SignalType::StreamEnd => self.close_stream(signal).await,
            
            // Chunked transfer: route to reassembly buffer
            SignalType::ChunkStart => self.start_chunk_transfer(signal).await,
            SignalType::ChunkData => self.receive_chunk(signal, connection).await,
            SignalType::ChunkEnd => self.finalize_chunk(signal, connection).await,
            
            // Subscriptions
            SignalType::Subscribe => self.add_subscription(signal, connection).await,
            SignalType::Unsubscribe => self.remove_subscription(signal, connection).await,
            
            // Errors
            SignalType::Error => self.handle_error(signal).await,
            
            _ => { /* log unknown signal type */ }
        }
    }
    
    async fn handle_intent(&self, signal: Signal, connection: &SynapseConnection) {
        let intent_text = signal.payload_as_string();
        let mind = self.mind.read().await;
        
        let program = mind.infer(&tokenize(&intent_text));
        let result = self.plugin_manager.execute_program(program).await;
        
        let response = Signal::result(result, signal.sequence);
        connection.send(response).await;
    }
}
```

### 14.3 Request-Response Correlation

When a SOMA sends an intent to a peer and expects a result, it stores a one-shot channel keyed by sequence number:

```rust
// Sending side
let (tx, rx) = oneshot::channel();
router.pending_requests.insert(sequence_id, tx);
connection.send(intent_signal).await;
let response = rx.await?;  // blocks until response arrives
```

Timeout: if no response within a configurable duration (default 30s), the pending request is cancelled and an error is returned.

### 14.4 Embedded Routing

On embedded, the router is simpler — no DashMap, no tokio. A fixed-size array of pending requests. Signals processed one at a time. No concurrent routing.

---

## 15. Configuration System

### 15.1 Config File Format (TOML)

```toml
# soma.toml

[soma]
id = "helperbook-backend"          # unique identifier
log_level = "info"                  # trace, debug, info, warn, error

[mind]
backend = "onnx"                    # "onnx" or "embedded"
model_dir = "./models/server"       # path to model files
max_inference_time = "5s"           # timeout per inference
max_program_steps = 16              # override default (8)
softmax_temperature = 1.0           # for program generation diversity

[mind.lora]
default_rank = 8
default_alpha = 2.0
adaptation_enabled = true
adapt_every_n_successes = 5         # trigger adaptation after N successful executions
adapt_batch_size = 16
adapt_learning_rate = 0.002

[memory]
checkpoint_dir = "./checkpoints"
auto_checkpoint = true
checkpoint_interval = "1h"          # or "every 100 executions"
max_checkpoints = 10                # keep last N, delete older

[memory.consolidation]
enabled = true
trigger = "experience_count"        # "experience_count", "schedule", "manual"
threshold = 500                     # consolidate after 500 experiences
min_lora_magnitude = 0.01           # only merge layers with magnitude > threshold

[protocol]
bind = "0.0.0.0:9001"
max_connections = 100
max_signal_size = "10MB"
keepalive_interval = "30s"
connection_timeout = "60s"

[protocol.encryption]
enabled = true
key_file = "./soma_key.ed25519"     # auto-generated on first run if missing

[protocol.peers]
# Static peer list (also discovered dynamically)
helperbook-interface = "localhost:9002"
helperbook-worker = "10.0.1.5:9001"

[plugins]
directory = "./plugins"
# Per-plugin config
[plugins.postgres]
host = "localhost"
port = 5432
database = "helperbook"
username = "soma"
password_env = "SOMA_PG_PASSWORD"   # read from environment variable
max_connections = 10
query_timeout = "30s"

[plugins.redis]
url = "redis://localhost:6379/0"
max_connections = 5

[plugins.smtp]
host = "smtp.provider.com"
port = 587
username_env = "SOMA_SMTP_USER"
password_env = "SOMA_SMTP_PASS"

[plugins.s3]
endpoint = "https://s3.amazonaws.com"
bucket = "helperbook-media"
region = "eu-west-1"
access_key_env = "SOMA_S3_KEY"
secret_key_env = "SOMA_S3_SECRET"

[resources]
max_memory = "512MB"                # total memory budget
max_concurrent_inferences = 10
max_concurrent_plugin_calls = 50
```

### 15.2 Embedded Config

Embedded targets use a simplified config compiled into the binary or stored in flash:

```toml
# soma-embedded.toml (stored in flash)

[soma]
id = "greenhouse-sensor"

[mind]
backend = "embedded"
max_program_steps = 8

[mind.lora]
default_rank = 2
adaptation_enabled = false          # receive LoRA from peer instead

[protocol]
bind = "0.0.0.0:9001"
max_connections = 3
max_signal_size = "4KB"

[protocol.peers]
hub = "192.168.1.10:9001"

[plugins]
# Built-in only, no directory scanning
builtin = ["gpio", "i2c", "timer", "adc"]

[plugins.gpio]
# ESP32-specific pin mapping
led_pin = 2
relay_pins = [12, 13, 14, 15]

[plugins.i2c]
sda_pin = 21
scl_pin = 22
clock_speed = 100000
```

### 15.3 Config Resolution Order

1. Default values (compiled into binary)
2. Config file (soma.toml)
3. Environment variables (SOMA_* prefix)
4. CLI arguments (highest priority)

Environment variables override config file. CLI overrides everything. This allows the same config file to work across environments with secrets injected via env vars.

---

## 16. Graceful Shutdown

### 16.1 Shutdown Trigger

Shutdown is triggered by: SIGTERM, SIGINT (Ctrl+C), explicit intent ("shutdown"), or fatal error.

### 16.2 Shutdown Sequence

```
1. Stop accepting new Synaptic connections
         │
2. Signal all connected peers: CLOSE signal
   (peers know this SOMA is going away)
         │
3. Wait for in-flight inferences to complete (max 10s timeout)
   (don't interrupt a program mid-execution)
         │
4. Flush pending signals in outbound queues
         │
5. Auto-checkpoint if enabled and there's unsaved experience
   ├── Save LoRA state
   ├── Save experience stats
   └── Log: "Checkpoint saved: {path}"
         │
6. Unload plugins (in reverse load order)
   ├── Call plugin.on_unload() for each
   ├── Plugins close connections, flush buffers, release resources
   └── Log: "Plugin {name} unloaded"
         │
7. Close Synaptic listeners
         │
8. Final log: "SOMA shutdown complete. Uptime: {duration}, 
   Executions: {count}, Experiences: {count}"
         │
9. Exit
```

### 16.3 Embedded Shutdown

On embedded, shutdown is typically a hardware reset or power-off. Before reset:

1. Save LoRA state to flash (if changed since last save)
2. Send CLOSE signal to connected peers (if connected)
3. Set GPIO pins to safe states (all outputs low/high-impedance)

Flash write during shutdown must complete — interrupted flash writes can corrupt data. Use a double-buffer strategy: write new state to alternate flash sector, then flip the "active" pointer. If power dies mid-write, the old state is still intact.

### 16.4 Crash Recovery

If a SOMA crashes without graceful shutdown:

- On restart, the last checkpoint is restored automatically
- Any experience since the last checkpoint is lost
- Connected peers detect the disconnection (TCP keepalive timeout) and can reconnect when the SOMA comes back
- Plugin state (database connections, etc.) is re-established during boot

---

## 17. Adaptation Loop

### 17.1 Experience Recording

After every successful program execution:

```rust
pub struct Experience {
    pub intent_tokens: Vec<u32>,
    pub intent_length: usize,
    pub program: Program,        // the program that was executed
    pub success: bool,           // did execution complete without error?
    pub execution_time: Duration,
    pub timestamp: Instant,
}
```

Only successful executions are recorded. Failed executions are NOT recorded — the SOMA should not learn from its mistakes by reinforcing the wrong programs. (This may change with a more sophisticated adaptation strategy that includes negative examples.)

### 17.2 When Adaptation Triggers

Configurable via `[mind.lora]` config:

| Trigger | Config Key | Default |
|---|---|---|
| Every N successes | `adapt_every_n_successes` | 5 |
| Manual | intent: "adapt now" | — |
| Scheduled | `adapt_schedule` | disabled |
| Experience buffer full | `adapt_on_buffer_full` | true |

### 17.3 Adaptation Flow (Server)

```
1. Trigger fires (e.g., 5 successful executions since last adaptation)
         │
2. Sample batch from experience buffer
   (random sample, size = adapt_batch_size)
         │
3. Acquire write lock on Mind
         │
4. Forward pass: compute loss between
   model's current prediction and recorded programs
         │
5. Backward pass: compute gradients
   (ONLY on LoRA parameters — base weights frozen)
         │
6. Update LoRA parameters: A -= lr * grad_A, B -= lr * grad_B
         │
7. Release write lock
         │
8. Log: "Adapted. Loss: {loss:.4f}, Cycle: {count}, 
   LoRA magnitude: {mag:.6f}"
```

During step 3-7, inference is paused (write lock). This takes ~10-50ms on a server — acceptable.

### 17.4 Adaptation Flow (Embedded)

Embedded SOMAs don't compute gradients locally (too expensive for ESP32). Instead:

```
1. Embedded SOMA records experiences locally
         │
2. Periodically (or when connected to a more powerful peer):
   send experience batch to peer via Synaptic Protocol
         │
   Embedded → Peer: DATA {
     type: "adaptation_request",
     experiences: [{intent_tokens, program, ...}, ...]
   }
         │
3. Peer SOMA (server-class) computes adaptation:
   runs forward/backward pass, produces updated LoRA weights
         │
   Peer → Embedded: DATA {
     type: "lora_update",
     weights: {layer_name: {A: [...], B: [...]}, ...}
   }
         │
4. Embedded SOMA applies received LoRA weights
         │
5. Embedded verifies: re-run a few experiences,
   check that predictions improved
```

This is "learning by delegation" — the embedded SOMA delegates the expensive computation to a capable peer while retaining the experience.

### 17.5 Consolidation Trigger

Consolidation (LoRA merge into base weights) is triggered when:

| Condition | Config Key |
|---|---|
| Experience count exceeds threshold | `consolidation.threshold` |
| LoRA magnitude exceeds threshold | `consolidation.min_lora_magnitude` |
| Manual command | intent: "consolidate" or ":consolidate" |
| Scheduled | `consolidation.schedule` |

Consolidation process:

```
1. Acquire write lock on Mind
2. For each LoRA layer:
   a. Check magnitude (skip if below threshold)
   b. Merge: base_weight += scale * B @ A
   c. Reset: A = randn(0.01), B = zeros
3. Create checkpoint (new permanent state)
4. Reset experience buffer
5. Release write lock
6. Log: "Consolidated. {N} layers merged. Permanent memory grew."
```

On embedded, step 2b writes to flash. This is a slow operation (~100ms per layer) and consumes a flash write cycle. Consolidation on embedded should be infrequent (daily or weekly, not hourly).

---

## 18. Observability

### 18.1 Structured Logging

All SOMA components emit structured log events via the `tracing` crate:

```rust
tracing::info!(
    intent = %intent_text,
    program_steps = program.len(),
    confidence = %confidence,
    inference_time_ms = %elapsed.as_millis(),
    "Intent processed"
);

tracing::warn!(
    plugin = %plugin_name,
    convention = %conv_name,
    error = %error_msg,
    retryable = retryable,
    "Plugin execution failed"
);
```

Log output formats:
- **Development:** pretty-printed, colored terminal output
- **Production:** JSON lines (for log aggregation — Loki, ELK, etc.)
- **Embedded:** minimal, UART serial output

### 18.1.1 Log Schema (JSON Lines Format)

Every log event in production has these fields:

```json
{
  "ts": "2026-04-07T14:30:01.234Z",
  "level": "info",
  "soma_id": "helperbook-backend",
  "component": "mind",
  "trace_id": "a1b2c3d4e5f6",
  "span_id": "x1y2z3",
  "parent_span_id": "p1q2r3",
  "msg": "Intent processed",
  "fields": {
    "intent": "list all contacts near downtown",
    "program_steps": 3,
    "confidence": 0.968,
    "inference_time_ms": 23,
    "plugins_used": ["postgres", "geo"]
  }
}
```

| Field | Always Present | Description |
|---|---|---|
| `ts` | Yes | ISO 8601 timestamp with millisecond precision |
| `level` | Yes | trace, debug, info, warn, error |
| `soma_id` | Yes | Which SOMA emitted this log |
| `component` | Yes | mind, protocol, plugin, memory, router |
| `trace_id` | If in request context | Unique ID for the entire request chain |
| `span_id` | If in request context | ID for this specific operation |
| `parent_span_id` | If nested | Parent span for hierarchical tracing |
| `msg` | Yes | Human-readable message |
| `fields` | Optional | Structured key-value data specific to the event |

### 18.1.2 Trace ID Propagation

When SOMA-A sends an intent to SOMA-B, the trace_id propagates via Synaptic Protocol metadata:

```
SOMA-A: generates trace_id = "a1b2c3"
SOMA-A → SOMA-B: INTENT {metadata: {trace_id: "a1b2c3"}, ...}
SOMA-B: receives signal, extracts trace_id, uses it in all logs for this request
SOMA-B → Plugin: logs with trace_id = "a1b2c3"
SOMA-B → SOMA-A: RESULT {metadata: {trace_id: "a1b2c3"}, ...}
```

All logs across the entire SOMA network for a single user request share the same trace_id. This enables distributed tracing: find all logs for one request across all SOMAs.

Span IDs create a tree:
```
trace_id: a1b2c3
  └── span: interface-send (Interface SOMA)
      └── span: backend-infer (Backend SOMA, Mind)
          ├── span: postgres-query (Backend SOMA, postgres plugin)
          └── span: redis-cache (Backend SOMA, redis plugin)
```

### 18.2 Program Trace

Every inference produces a trace that can be inspected:

```
[Trace] Intent: "list files in /tmp and send to soma-b"
[Trace] Confidence: 94.2%
[Trace] Program:
  [0] libc.opendir("/tmp")           → ok, handle=0x7f3a
  [1] libc.readdir($0)              → ok, 12 entries
  [2] libc.closedir($0)             → ok
  [3] synapse.send("soma-b", $1)    → ok, sent to peer
  [4] STOP
[Trace] Total: 4 steps, 23ms, success
```

Trace verbosity is configurable:
- `trace_level = "none"` — no tracing
- `trace_level = "summary"` — one line per intent (confidence, steps, time, success)
- `trace_level = "steps"` — per-step results
- `trace_level = "full"` — includes tensor values, LoRA activations, attention weights

### 18.3 Debug REPL

When started with `--repl`, the SOMA provides an interactive shell:

```
soma> list files in /tmp
  [Mind] Program (5 steps, 97.3%):
    $0 = libc.opendir("/tmp")
    ...
  [Body] (12 items): ...

soma> :status
  Mind: 823,456 params, OnnxMindEngine
  LoRA: 15,232 trainable, magnitude 0.023
  Plugins: 5 loaded (postgres, redis, smtp, s3, dom-renderer)
  Connections: 2 peers (soma-interface:9002, soma-worker:9003)
  Experience: 142 recorded, 28 adaptations
  Uptime: 2h 34m

soma> :trace on
  Trace level set to "steps"

soma> :inspect mind
  Encoder: BiLSTM 2-layer, hidden=128, bidirectional
  Decoder: GRU, hidden=256
  Conventions known: 32
  LoRA layers: 10 (rank 8, alpha 2.0)
  Top-5 most used conventions:
    postgres.query (67 times)
    synapse.send (45 times)
    redis.cache_get (23 times)
    ...

soma> :inspect plugin postgres
  Name: postgres
  Version: 0.1.0
  Conventions: 12
  Config: host=localhost, port=5432, db=helperbook
  Active connections: 3/10
  Queries executed: 67
  Avg query time: 12ms

soma> :checkpoint
  Checkpoint saved: ./checkpoints/soma-1712504400.ckpt
  LoRA state: 15,232 params
  Experience: 142 entries
```

### 18.4 Metrics Export

For production monitoring, the SOMA exports metrics:

| Metric | Type | Description |
|---|---|---|
| `soma_inferences_total` | Counter | Total intents processed |
| `soma_inference_duration_ms` | Histogram | Inference latency |
| `soma_inference_confidence` | Histogram | Model confidence distribution |
| `soma_plugin_calls_total` | Counter (per plugin) | Plugin execution count |
| `soma_plugin_errors_total` | Counter (per plugin) | Plugin error count |
| `soma_plugin_duration_ms` | Histogram (per plugin) | Plugin execution latency |
| `soma_lora_magnitude` | Gauge | Current LoRA adaptation magnitude |
| `soma_experience_count` | Counter | Total experiences recorded |
| `soma_adaptations_total` | Counter | LoRA adaptation cycles |
| `soma_connections_active` | Gauge | Active Synaptic connections |
| `soma_signals_sent_total` | Counter | Signals sent |
| `soma_signals_received_total` | Counter | Signals received |
| `soma_memory_bytes` | Gauge | Memory usage |

Metrics exposed via a lightweight endpoint (if the http-bridge plugin is loaded) or via Synaptic Protocol to a monitoring SOMA.

### 18.5 Embedded Observability

On embedded targets, observability is limited:

- UART serial logging (minimal format, key events only)
- LED status codes (blink patterns indicating state: boot, ready, error, adapting)
- Proprioception queries via Synaptic Protocol from a more capable peer

---

## 19. Versioning and Compatibility

### 19.1 Version Components

A SOMA instance has multiple version dimensions:

| Component | Versioned How | Breaking Change Means |
|---|---|---|
| SOMA Core binary | Semantic versioning (0.1.0) | Plugin ABI changed, config format changed |
| ONNX model format | Model metadata version field | New layer types, changed input/output schema |
| .soma-model format | Header version byte | Changed binary layout, new quantization type |
| Synaptic Protocol | Protocol version in HANDSHAKE | Changed wire format, new signal types |
| Plugin ABI | Trait version + Rust edition | Changed SomaPlugin trait, changed Value enum |
| Checkpoint format | Checkpoint header version | Changed LoRA serialization, new fields |
| Config format | Config version field | Changed key names, removed options |

### 19.2 Compatibility Matrix

```
SOMA Core 0.2.0 loads:
  ├── Plugins compiled for Core 0.2.x  ✓
  ├── Plugins compiled for Core 0.1.x  ✗ (ABI mismatch)
  ├── ONNX models from Synthesizer 0.2.x  ✓
  ├── ONNX models from Synthesizer 0.1.x  ✓ (forward compatible)
  ├── Checkpoints from Core 0.2.x  ✓
  ├── Checkpoints from Core 0.1.x  ⚠ (best-effort migration)
  ├── Config from Core 0.2.x  ✓
  ├── Config from Core 0.1.x  ⚠ (deprecated keys warned)
  └── Peers running Core 0.1.x  ✓ (protocol negotiation)
```

### 19.3 Protocol Negotiation

During HANDSHAKE, both SOMAs declare their protocol version. The connection uses the lower version's feature set. This allows mixed-version SOMA networks.

```
SOMA-A (protocol v2) ↔ SOMA-B (protocol v1)
  → Connection uses v1 features only
  → v2-only signal types are not sent
```

### 19.4 Model-Plugin Convention Mismatch

The most common compatibility issue: the model was synthesized with plugin X providing conventions A, B, C — but at runtime, plugin X v2 provides conventions A, B, D (C was renamed/removed, D is new).

Resolution:
- Convention matching is by NAME, not by numeric ID
- If model references convention "postgres.query" and the plugin provides it → match
- If model references "redis.cache_get" but Redis plugin isn't loaded → fail at execution time with clear error
- If plugin provides "redis.cache_getex" that the model doesn't know about → ignored (available for future synthesis)

Re-synthesis is needed to take advantage of new plugin conventions. But old models continue working with old conventions.

### 19.5 Checkpoint Migration

When loading a checkpoint from an older version:

1. Read checkpoint header version
2. If same version → load directly
3. If older version → attempt migration:
   - LoRA weight dimensions match? → load
   - LoRA weight dimensions changed? → discard LoRA, start fresh (warn user)
   - Unknown fields in old checkpoint? → ignore
   - Missing fields expected by new version? → use defaults

Checkpoint migration is best-effort. The worst case is losing experiential memory — the SOMA starts fresh but with the correct base model. This is equivalent to "amnesia" — all permanent memory (base weights) is intact, only experience is lost.

---

## 20. Resource Limits

### 20.1 Default Limits

| Resource | Default | Configurable | Embedded Default |
|---|---|---|---|
| Max inference time | 5s | `mind.max_inference_time` | 2s |
| Max program steps | 16 | `mind.max_program_steps` | 8 |
| Max concurrent inferences | 10 | `resources.max_concurrent_inferences` | 1 |
| Max concurrent plugin calls | 50 | `resources.max_concurrent_plugin_calls` | 4 |
| Max signal payload size | 10MB | `protocol.max_signal_size` | 4KB |
| Max chunk transfer size | 100MB | `protocol.max_chunk_size` | disabled |
| Max Synaptic connections | 100 | `protocol.max_connections` | 3 |
| Max experience buffer size | 1000 | `memory.max_experience_buffer` | 50 |
| Max LoRA layers | 64 | `mind.lora.max_layers` | 4 |
| Max plugin memory | 512MB total | `resources.max_memory` | 100KB total |
| Max plugins loaded | 50 | `plugins.max_loaded` | 4 (built-in) |

### 20.2 Enforcement

Limits are enforced at the Core level, not by individual plugins:

```rust
// Before inference
if active_inferences.count() >= config.max_concurrent_inferences {
    return Err(SomaError::Resource(ResourceError::InferenceLimitReached));
}

// Before program step execution
if step_index >= config.max_program_steps {
    break;  // force STOP
}

// Before signal acceptance
if signal.payload.len() > config.max_signal_size {
    return Err(SomaError::Protocol(ProtocolError::PayloadTooLarge));
}
```

### 20.3 Backpressure

When resource limits are approached:

1. **80% threshold:** Log warning. Proprioception reports "approaching limits."
2. **95% threshold:** Reject new incoming intents with a backpressure signal. Continue serving in-flight requests.
3. **100%:** Hard rejection. Error signal to all new requests. Existing requests continue to completion.

Backpressure signal:

```
Signal {
    type: CONTROL,
    payload: {
        control_type: "backpressure",
        reason: "inference_limit",
        retry_after_ms: 1000
    }
}
```

The sending SOMA can queue the intent and retry after the suggested delay.

### 20.4 Embedded Resource Management

On embedded, memory is the critical resource. The SOMA tracks heap usage and refuses operations that would exceed the budget:

```rust
// Embedded memory tracking
static HEAP_USED: AtomicUsize = AtomicUsize::new(0);

fn allocate(size: usize) -> Result<*mut u8> {
    let current = HEAP_USED.load(Ordering::Relaxed);
    if current + size > MAX_HEAP {
        return Err(OutOfMemory);
    }
    HEAP_USED.fetch_add(size, Ordering::Relaxed);
    // ... allocate
}
```

Flash write cycles are also tracked. ESP32 flash endurance is ~100,000 write cycles per sector. Consolidation (which writes to flash) must respect this:

```rust
if flash_write_count > FLASH_ENDURANCE_LIMIT * 0.8 {
    tracing::warn!("Flash wear at 80%. Reducing consolidation frequency.");
    consolidation_interval *= 2;
}
```
