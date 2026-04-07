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