//! OnnxMindEngine — ONNX inference via tract for server/desktop targets.
//!
//! Spec Section 4.3 recommends the `ort` crate (ONNX Runtime with GPU/NPU
//! acceleration). We use `tract-onnx` instead because:
//! - Pure Rust: no C++ build dependency, simpler cross-compilation
//! - Single binary: no shared library requirements at runtime
//! - Sufficient for current model sizes (~800K params, <5ms inference)
//!
//! Migration to `ort` is tracked as a future optimization when GPU
//! acceleration becomes necessary for larger models (50M+ params).
//!
//! Implements MindEngine trait. Loads encoder.onnx + decoder.onnx.

use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;
use tract_onnx::prelude::*;

use super::{
    ArgType, ArgValue, CatalogEntry, MindEngine, MindInfo, ModelMeta,
    Program, ProgramStep, EMIT_ID, STOP_ID,
};
use super::tokenizer::Tokenizer;

type TractModel = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub struct OnnxMindEngine {
    encoder: TractModel,
    decoder: TractModel,
    pub tokenizer: Tokenizer,
    model_meta: ModelMeta,
    /// Softmax temperature for inference (Section 2.3). Lower = more deterministic.
    pub temperature: f32,
    /// Maximum inference time in seconds. If exceeded, the decoder loop breaks early
    /// and returns partial results with reduced confidence.
    pub max_inference_time_secs: u64,
    /// SHA-256 hash of encoder.onnx + decoder.onnx, computed at load time.
    pub model_hash: String,
    /// Active LoRA layers applied to decoder output heads (Section 4.7).
    /// LoRA is applied as post-hoc logit adjustment: logits += scale * (hidden @ A.T) @ B.T
    active_lora: Vec<super::lora::LoRALayer>,
    /// Permanently merged LoRA weight delta for opcode head (Section 6.3 consolidation).
    ///
    /// Since tract-onnx models are frozen (compiled into the graph), we cannot modify
    /// W_base in-place. Instead, during consolidation we compute `scale * B @ A` for each
    /// LoRA layer and accumulate the result here. During inference this delta is applied as:
    /// `output = base(hidden) + hidden @ merged_opcode_delta.T + lora_delta`
    ///
    /// Shape: (num_conventions * decoder_dim) in row-major order, or empty if no
    /// consolidation has occurred.
    pub merged_opcode_delta: Vec<f32>,
}

impl OnnxMindEngine {
    pub fn load(model_dir: &Path) -> Result<Self> {
        let meta_str = std::fs::read_to_string(model_dir.join("meta.json"))
            .context("Failed to read meta.json")?;
        let mut model_meta: ModelMeta = serde_json::from_str(&meta_str)?;

        // If catalog is not embedded in meta.json, load from catalog.json
        if model_meta.catalog.is_empty() {
            let catalog_path = model_dir.join("catalog.json");
            if catalog_path.exists() {
                let catalog_str = std::fs::read_to_string(&catalog_path)
                    .context("Failed to read catalog.json")?;
                let catalog_data: serde_json::Value = serde_json::from_str(&catalog_str)?;
                if let Some(entries) = catalog_data.get("entries").and_then(|e| e.as_array()) {
                    model_meta.catalog = entries.iter().filter_map(|e| {
                        let name = e.get("full_name")?.as_str()?.to_string();
                        let id = e.get("catalog_id")?.as_u64()? as usize;
                        Some(CatalogEntry {
                            id,
                            name: name.clone(),
                            function: name.clone(),
                            call_pattern: e.get("call_pattern").and_then(|v| v.as_str()).unwrap_or("direct").to_string(),
                            var_args: Vec::new(),
                            description: e.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        })
                    }).collect();
                    tracing::info!(entries = model_meta.catalog.len(), "Loaded catalog from catalog.json");
                }
            }
        }

        tracing::info!("Loading encoder.onnx...");
        let encoder = tract_onnx::onnx()
            .model_for_path(model_dir.join("encoder.onnx"))?
            .into_optimized()?.into_runnable()?;

        tracing::info!("Loading decoder.onnx...");
        let decoder = tract_onnx::onnx()
            .model_for_path(model_dir.join("decoder.onnx"))?
            .into_optimized()?.into_runnable()?;

        let tokenizer = Tokenizer::load(&model_dir.join("tokenizer.json"))?;

        // Compute SHA-256 hash of base model files for checkpoint integrity
        use sha2::{Sha256, Digest};
        let enc_bytes = std::fs::read(model_dir.join("encoder.onnx"))?;
        let dec_bytes = std::fs::read(model_dir.join("decoder.onnx"))?;
        let mut hasher = Sha256::new();
        hasher.update(&enc_bytes);
        hasher.update(&dec_bytes);
        let model_hash = format!("{:x}", hasher.finalize());

        tracing::info!(vocab = tokenizer.vocab_size(), conventions = model_meta.num_conventions, model_hash = %model_hash, "OnnxMindEngine loaded");
        Ok(Self { encoder, decoder, tokenizer, model_meta, temperature: 1.0, max_inference_time_secs: 5, model_hash, active_lora: Vec::new(), merged_opcode_delta: Vec::new() })
    }
}

/// Adaptation-specific methods for runtime LoRA training.
/// These expose internal encoder/decoder to the adaptation engine
/// without breaking the MindEngine trait abstraction.
impl OnnxMindEngine {
    /// Run the encoder on raw intent tokens and return (encoder_output, initial_hidden).
    /// The encoder_output is (1, seq_len, decoder_dim) flattened, hidden is (decoder_dim,).
    pub fn encode_for_adaptation(&self, intent_tokens: &[u32]) -> anyhow::Result<(Vec<f32>, Vec<f32>)> {
        let seq_len = self.model_meta.max_seq_len;
        let mut ids: Vec<i64> = intent_tokens.iter().map(|&t| t as i64).collect();
        let real_len = ids.len();
        let mut mask_vec = vec![1.0f32; real_len];

        while ids.len() < seq_len { ids.push(0); }
        while mask_vec.len() < seq_len { mask_vec.push(0.0); }
        ids.truncate(seq_len);
        mask_vec.truncate(seq_len);

        let enc_result = self.encoder.run(tvec![
            tract_ndarray::Array2::from_shape_vec((1, seq_len), ids)?.into_tvalue(),
            tract_ndarray::Array2::from_shape_vec((1, seq_len), mask_vec)?.into_tvalue(),
        ])?;
        let enc_out: Vec<f32> = enc_result[0].to_array_view::<f32>()?.iter().cloned().collect();
        let hidden: Vec<f32> = enc_result[2].to_array_view::<f32>()?.iter().cloned().collect();
        Ok((enc_out, hidden))
    }

    /// Run one decoder step and return (new_hidden, opcode_logits).
    /// This does NOT apply LoRA — the caller gets raw base logits for adaptation.
    pub fn decode_step_for_adaptation(
        &self,
        prev_op: i64,
        hidden: &[f32],
        enc_out: &[f32],
        prev_hiddens: &[f32],
        step: usize,
    ) -> anyhow::Result<(Vec<f32>, Vec<f32>)> {
        let dd = self.model_meta.decoder_dim;
        let seq_len = self.model_meta.max_seq_len;
        let ms = self.model_meta.max_steps;

        // Reconstruct mask (all 1s for seq_len) — we don't have the original mask,
        // but for adaptation purposes full attention is acceptable.
        let mask_vec = vec![1.0f32; seq_len];

        let dr = self.decoder.run(tvec![
            tract_ndarray::Array1::from_vec(vec![prev_op]).into_tvalue(),
            tract_ndarray::Array2::from_shape_vec((1, dd), hidden.to_vec())?.into_tvalue(),
            tract_ndarray::Array3::from_shape_vec((1, seq_len, dd), enc_out.to_vec())?.into_tvalue(),
            tract_ndarray::Array2::from_shape_vec((1, seq_len), mask_vec)?.into_tvalue(),
            tract_ndarray::Array3::from_shape_vec((1, ms, dd), prev_hiddens.to_vec())?.into_tvalue(),
            tract_ndarray::Array1::from_vec(vec![step as i64]).into_tvalue(),
        ])?;

        let new_hidden: Vec<f32> = dr[0].to_array_view::<f32>()?.iter().cloned().collect();
        let op_logits: Vec<f32> = dr[1].to_array_view::<f32>()?.iter().cloned().collect();

        Ok((new_hidden, op_logits))
    }

    /// Get a mutable reference to the active LoRA layers for adaptation.
    pub fn active_lora_mut(&mut self) -> &mut Vec<super::lora::LoRALayer> {
        &mut self.active_lora
    }

    /// Get a reference to the active LoRA layers.
    pub fn active_lora(&self) -> &[super::lora::LoRALayer] {
        &self.active_lora
    }

    /// Apply matching LoRA layers to a set of logits for a given output head.
    ///
    /// For each active LoRA layer whose name matches `head_name`, compute:
    ///   delta = scale * (hidden @ A.T) @ B.T
    /// and add to the logits in-place.
    fn apply_lora_to_logits(&self, head_name: &str, hidden: &[f32], logits: &mut [f32]) {
        for lora in &self.active_lora {
            if lora.name == head_name && !lora.b.is_empty() {
                let delta = lora.forward(hidden);
                for (i, d) in delta.iter().enumerate() {
                    if i < logits.len() {
                        logits[i] += d;
                    }
                }
            }
        }
    }

    /// Get the decoder dimension from model metadata.
    pub fn decoder_dim(&self) -> usize {
        self.model_meta.decoder_dim
    }

    /// Get max program steps from model metadata.
    pub fn max_steps(&self) -> usize {
        self.model_meta.max_steps
    }

    /// Get the number of conventions (opcode output size).
    pub fn num_conventions(&self) -> usize {
        self.model_meta.num_conventions
    }

    /// Get the start token ID.
    pub fn start_token(&self) -> usize {
        self.model_meta.start_token
    }

    /// Get the stop ID.
    pub fn stop_id(&self) -> usize {
        self.model_meta.stop_id
    }

    /// Get a reference to the consolidated weight delta.
    pub fn merged_opcode_delta(&self) -> &[f32] {
        &self.merged_opcode_delta
    }

    /// Set the consolidated weight delta (for checkpoint restore).
    pub fn set_merged_opcode_delta(&mut self, delta: Vec<f32>) {
        self.merged_opcode_delta = delta;
    }
}

impl MindEngine for OnnxMindEngine {
    fn infer(&self, text: &str) -> Result<Program> {
        let tokens = self.tokenizer.tokenize(text);
        let mut ids = self.tokenizer.encode_with_null(text);
        let real_len = ids.len();
        let seq_len = self.model_meta.max_seq_len;
        let dd = self.model_meta.decoder_dim;
        let ms = self.model_meta.max_steps;

        // Pad
        let mut mask_vec = vec![1.0f32; real_len];
        while ids.len() < seq_len { ids.push(0); }
        while mask_vec.len() < seq_len { mask_vec.push(0.0); }
        ids.truncate(seq_len); mask_vec.truncate(seq_len);

        // Encode
        let enc_result = self.encoder.run(tvec![
            tract_ndarray::Array2::from_shape_vec((1, seq_len), ids)?.into_tvalue(),
            tract_ndarray::Array2::from_shape_vec((1, seq_len), mask_vec.clone())?.into_tvalue(),
        ])?;
        let enc_out: Vec<f32> = enc_result[0].to_array_view::<f32>()?.iter().cloned().collect();
        let mut hidden: Vec<f32> = enc_result[2].to_array_view::<f32>()?.iter().cloned().collect();

        // Decode
        let mut prev_hiddens = vec![0.0f32; ms * dd];
        let mut prev_op = self.model_meta.start_token as i64;
        let mut steps = Vec::new();
        let mut step_confidences: Vec<f32> = Vec::new();
        let infer_start = Instant::now();
        let timeout_secs = self.max_inference_time_secs;

        for t in 0..ms {
            // Check inference timeout before each decoder step
            if timeout_secs > 0 && infer_start.elapsed().as_secs() >= timeout_secs {
                tracing::warn!(
                    elapsed_secs = infer_start.elapsed().as_secs(),
                    max_secs = timeout_secs,
                    steps_completed = steps.len(),
                    "Inference timeout — returning partial program"
                );
                break;
            }
            let dr = self.decoder.run(tvec![
                tract_ndarray::Array1::from_vec(vec![prev_op]).into_tvalue(),
                tract_ndarray::Array2::from_shape_vec((1, dd), hidden.clone())?.into_tvalue(),
                tract_ndarray::Array3::from_shape_vec((1, seq_len, dd), enc_out.clone())?.into_tvalue(),
                tract_ndarray::Array2::from_shape_vec((1, seq_len), mask_vec.clone())?.into_tvalue(),
                tract_ndarray::Array3::from_shape_vec((1, ms, dd), prev_hiddens.clone())?.into_tvalue(),
                tract_ndarray::Array1::from_vec(vec![t as i64]).into_tvalue(),
            ])?;

            let new_h = dr[0].to_array_view::<f32>()?;

            // Extract raw opcode logits from decoder output
            let mut op_l: Vec<f32> = dr[1].to_array_view::<f32>()?.iter().cloned().collect();

            // Apply consolidated weight delta (permanent merged LoRA knowledge, Section 6.3).
            // This is the accumulated result of past consolidations: merged_delta = sum(scale * B @ A)
            // Applied as: logits += hidden @ merged_delta.T
            if !self.merged_opcode_delta.is_empty() {
                let num_ops = op_l.len();
                for o in 0..num_ops {
                    let mut delta = 0.0f32;
                    for d in 0..dd {
                        delta += hidden[d] * self.merged_opcode_delta[o * dd + d];
                    }
                    op_l[o] += delta;
                }
            }

            // Apply LoRA to opcode logits (Section 4.7)
            // Post-hoc adjustment: logits += scale * (hidden @ A.T) @ B.T
            self.apply_lora_to_logits("opcode", &hidden, &mut op_l);

            // Apply temperature scaling to logits (Section 2.3: deterministic execution)
            // Clamp to minimum 0.01 to prevent division by zero
            let temp = self.temperature.max(0.01);
            let op_l: Vec<f32> = op_l.iter().map(|x| x / temp).collect();
            let pred = argmax(&op_l);

            // Compute softmax probability of chosen opcode for this step
            {
                let mx = op_l[pred];
                let es: f32 = op_l.iter().map(|x| (x - mx).exp()).sum();
                let step_prob = 1.0 / es; // softmax(pred) = exp(pred-max) / sum = 1 / sum (since pred IS max)
                step_confidences.push(step_prob);
            }

            hidden = new_h.iter().cloned().collect();
            for (i, &v) in hidden.iter().enumerate() { prev_hiddens[t * dd + i] = v; }

            if pred == self.model_meta.stop_id {
                steps.push(ProgramStep { conv_id: STOP_ID, arg0_type: ArgType::None, arg0_value: ArgValue::None, arg1_type: ArgType::None, arg1_value: ArgValue::None });
                break;
            }
            if pred == self.model_meta.emit_id {
                let mut r0: Vec<f32> = dr[8].to_array_view::<f32>()?.iter().cloned().collect();
                self.apply_lora_to_logits("r0", &hidden, &mut r0);
                steps.push(ProgramStep { conv_id: EMIT_ID, arg0_type: ArgType::Ref, arg0_value: ArgValue::Ref(argmax(&r0)), arg1_type: ArgType::None, arg1_value: ArgValue::None });
            } else {
                // Extract all output head logits and apply LoRA to each (Section 4.7)
                let mut a0t: Vec<f32> = dr[2].to_array_view::<f32>()?.iter().cloned().collect();
                let mut a1t: Vec<f32> = dr[3].to_array_view::<f32>()?.iter().cloned().collect();
                let mut s0s: Vec<f32> = dr[4].to_array_view::<f32>()?.iter().cloned().collect();
                let mut s0e: Vec<f32> = dr[5].to_array_view::<f32>()?.iter().cloned().collect();
                let mut s1s: Vec<f32> = dr[6].to_array_view::<f32>()?.iter().cloned().collect();
                let mut s1e: Vec<f32> = dr[7].to_array_view::<f32>()?.iter().cloned().collect();
                let mut r0: Vec<f32> = dr[8].to_array_view::<f32>()?.iter().cloned().collect();
                let mut r1: Vec<f32> = dr[9].to_array_view::<f32>()?.iter().cloned().collect();
                // Literal heads may not be present in older ONNX exports
                let mut lit0: Vec<f32> = if dr.len() > 10 {
                    dr[10].to_array_view::<f32>()?.iter().cloned().collect()
                } else { vec![0.0; self.model_meta.vocab_size] };
                let mut lit1: Vec<f32> = if dr.len() > 11 {
                    dr[11].to_array_view::<f32>()?.iter().cloned().collect()
                } else { vec![0.0; self.model_meta.vocab_size] };

                self.apply_lora_to_logits("a0t", &hidden, &mut a0t);
                self.apply_lora_to_logits("a1t", &hidden, &mut a1t);
                self.apply_lora_to_logits("s0s", &hidden, &mut s0s);
                self.apply_lora_to_logits("s0e", &hidden, &mut s0e);
                self.apply_lora_to_logits("s1s", &hidden, &mut s1s);
                self.apply_lora_to_logits("s1e", &hidden, &mut s1e);
                self.apply_lora_to_logits("r0", &hidden, &mut r0);
                self.apply_lora_to_logits("r1", &hidden, &mut r1);
                self.apply_lora_to_logits("lit0", &hidden, &mut lit0);
                self.apply_lora_to_logits("lit1", &hidden, &mut lit1);

                let (a0ty, a0v) = resolve_arg_from_logits(argmax(&a0t), &s0s, &s0e, &r0, &lit0, &tokens, &self.tokenizer);
                let (a1ty, a1v) = resolve_arg_from_logits(argmax(&a1t), &s1s, &s1e, &r1, &lit1, &tokens, &self.tokenizer);
                steps.push(ProgramStep { conv_id: pred as i32, arg0_type: a0ty, arg0_value: a0v, arg1_type: a1ty, arg1_value: a1v });
            }
            prev_op = pred as i64;
        }
        // Compute final confidence as minimum probability across all steps (conservative).
        // If inference timed out, apply an additional penalty.
        let mut confidence = step_confidences.iter().cloned().fold(f32::MAX, f32::min);
        if confidence == f32::MAX {
            confidence = 0.0; // no steps decoded
        }
        if timeout_secs > 0 && infer_start.elapsed().as_secs() >= timeout_secs {
            confidence *= 0.5; // penalty for incomplete program due to timeout
        }
        Ok(Program { steps, confidence })
    }

    fn meta(&self) -> &ModelMeta { &self.model_meta }

    fn info(&self) -> MindInfo {
        let mag: f32 = self.active_lora.iter().map(|l| l.magnitude()).sum();

        // Estimate param_count from model dimensions:
        // Embedding: vocab_size * decoder_dim
        // Encoder (BiLSTM+GRU approx): decoder_dim * decoder_dim * 8
        // Decoder attention + heads: decoder_dim * decoder_dim * 4
        // Output heads: num_conventions * decoder_dim (opcode) + vocab_size * decoder_dim (literals)
        let vs = self.model_meta.vocab_size;
        let dd = self.model_meta.decoder_dim;
        let nc = self.model_meta.num_conventions;
        let param_count = vs * dd               // embedding
            + dd * dd * 8                        // encoder (BiLSTM layers)
            + dd * dd * 4                        // decoder attention + hidden
            + nc * dd                            // opcode output head
            + vs * dd;                           // literal output head

        MindInfo {
            backend: "OnnxMindEngine (tract)".into(),
            param_count,
            conventions_known: self.model_meta.num_conventions,
            max_steps: self.model_meta.max_steps,
            lora_layers: self.active_lora.len(),
            lora_magnitude: mag,
        }
    }

    fn attach_lora(&mut self, name: &str, weights: &super::lora::LoRAWeights) -> Result<()> {
        self.active_lora.push(super::lora::LoRALayer {
            name: name.to_string(),
            base_weight_shape: (weights.b.len() / weights.rank, weights.a.len() / weights.rank),
            a: weights.a.clone(),
            b: weights.b.clone(),
            rank: weights.rank,
            scale: weights.scale,
        });
        tracing::info!(name, rank = weights.rank, "LoRA attached");
        Ok(())
    }

    fn attach_lora_bytes(&mut self, plugin_name: &str, data: &[u8]) -> Result<()> {
        let bundle: super::lora::LoRABundle = serde_json::from_slice(data)
            .context("Failed to deserialize plugin LoRA bundle")?;
        let count = bundle.layers.len();
        for weights in &bundle.layers {
            self.attach_lora(&weights.name, weights)?;
        }
        tracing::info!(
            plugin = plugin_name,
            layers = count,
            "Plugin LoRA bundle attached"
        );
        Ok(())
    }

    fn detach_lora(&mut self, name: &str) -> Result<()> {
        self.active_lora.retain(|l| l.name != name);
        tracing::info!(name, "LoRA detached");
        Ok(())
    }

    fn merge_lora(&mut self, name: &str) -> Result<()> {
        // In tract, we can't modify base weights (they're compiled into the graph).
        // Instead, we compute the weight delta (scale * B @ A) and accumulate it in
        // merged_opcode_delta, which is applied during inference as a permanent overlay.
        let idx = self.active_lora.iter().position(|l| l.name == name);
        if let Some(i) = idx {
            let delta = self.active_lora[i].compute_weight_delta();
            let dd = self.model_meta.decoder_dim;
            let num_ops = self.model_meta.num_conventions;
            let expected_size = num_ops * dd;

            // Initialize merged_opcode_delta if empty
            if self.merged_opcode_delta.is_empty() {
                self.merged_opcode_delta = vec![0.0f32; expected_size];
            }

            // Accumulate delta
            let merge_len = delta.len().min(self.merged_opcode_delta.len());
            for j in 0..merge_len {
                self.merged_opcode_delta[j] += delta[j];
            }

            // Reset the LoRA layer (A to small random, B to zero)
            self.active_lora[i].reset();

            tracing::info!(
                name,
                delta_nonzero = delta.iter().filter(|&&v| v != 0.0).count(),
                "LoRA merged into consolidated weight delta"
            );
        } else {
            tracing::warn!(name, "LoRA merge requested but layer not found");
        }
        Ok(())
    }

    fn checkpoint_lora(&self) -> Result<super::lora::LoRACheckpoint> {
        let layers = self.active_lora.iter().map(|l| super::lora::LoRALayerState {
            name: l.name.clone(),
            rank: l.rank,
            scale: l.scale,
            a: l.a.clone(),
            b: l.b.clone(),
        }).collect();
        Ok(super::lora::LoRACheckpoint {
            layers,
            adaptation_count: 0,
            experience_count: 0,
        })
    }

    fn restore_lora(&mut self, checkpoint: &super::lora::LoRACheckpoint) -> Result<()> {
        self.active_lora.clear();
        for layer in &checkpoint.layers {
            self.active_lora.push(super::lora::LoRALayer {
                name: layer.name.clone(),
                base_weight_shape: (layer.b.len() / layer.rank, layer.a.len() / layer.rank),
                a: layer.a.clone(),
                b: layer.b.clone(),
                rank: layer.rank,
                scale: layer.scale,
            });
        }
        tracing::info!(layers = self.active_lora.len(), "LoRA restored from checkpoint");
        Ok(())
    }
}

fn resolve_arg(tid: usize, o: &TVec<TValue>, ss: usize, se: usize, ri: usize, li: usize, tokens: &[String], tok: &Tokenizer) -> (ArgType, ArgValue) {
    match tid {
        1 => {
            let s: Vec<f32> = o[ss].to_array_view::<f32>().unwrap().iter().cloned().collect();
            let e: Vec<f32> = o[se].to_array_view::<f32>().unwrap().iter().cloned().collect();
            (ArgType::Span, ArgValue::Span(tok.extract_span(tokens, argmax(&s), argmax(&e).max(argmax(&s)))))
        }
        2 => { let r: Vec<f32> = o[ri].to_array_view::<f32>().unwrap().iter().cloned().collect(); (ArgType::Ref, ArgValue::Ref(argmax(&r))) }
        3 => {
            let lit: Vec<f32> = o[li].to_array_view::<f32>().unwrap().iter().cloned().collect();
            let vocab_idx = argmax(&lit);
            (ArgType::Literal, ArgValue::Literal(tok.decode_index(vocab_idx)))
        }
        _ => (ArgType::None, ArgValue::None),
    }
}

/// Resolve an argument from pre-extracted (and LoRA-adjusted) logits.
/// This variant works with logit slices directly instead of raw decoder tensors,
/// enabling LoRA application to all output heads before argument resolution.
fn resolve_arg_from_logits(tid: usize, span_start: &[f32], span_end: &[f32], ref_logits: &[f32], lit_logits: &[f32], tokens: &[String], tok: &Tokenizer) -> (ArgType, ArgValue) {
    match tid {
        1 => {
            let s = argmax(span_start);
            let e = argmax(span_end).max(s);
            (ArgType::Span, ArgValue::Span(tok.extract_span(tokens, s, e)))
        }
        2 => (ArgType::Ref, ArgValue::Ref(argmax(ref_logits))),
        3 => {
            let vocab_idx = argmax(lit_logits);
            (ArgType::Literal, ArgValue::Literal(tok.decode_index(vocab_idx)))
        }
        _ => (ArgType::None, ArgValue::None),
    }
}

fn argmax(s: &[f32]) -> usize { s.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(i, _)| i).unwrap_or(0) }
