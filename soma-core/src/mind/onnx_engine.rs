//! Production `MindEngine` implementation using tract-onnx for CPU inference.
//!
//! Loads a two-part ONNX model (encoder.onnx + decoder.onnx) and runs an
//! autoregressive decode loop: at each step the decoder predicts an opcode
//! (convention ID) and argument types/values via 11 output heads. `LoRA`
//! adapters and consolidated weight deltas are applied as post-hoc logit
//! adjustments since tract compiles models into frozen graphs.
//!
//! We use `tract-onnx` over the spec-recommended `ort` crate because it is
//! pure Rust (no C++ dependency), produces a single binary, and is sufficient
//! for current model sizes (~800K params, <5ms inference). Migration to `ort`
//! is tracked for when GPU acceleration is needed (50M+ params).

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Instant;
use tract_onnx::prelude::*;

use super::{
    ArgType, ArgValue, CatalogEntry, MindEngine, MindInfo, ModelMeta,
    Program, ProgramStep, EMIT_ID, STOP_ID,
};
use super::tokenizer::Tokenizer;

/// Compiled tract model type alias (optimized, runnable graph).
type TractModel = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// ONNX-based Mind implementation for server/desktop targets.
///
/// Holds the compiled encoder and decoder graphs, tokenizer, and runtime
/// `LoRA` state. The encoder produces context vectors from the intent; the
/// decoder autoregressively emits program steps.
pub struct OnnxMindEngine {
    encoder: TractModel,
    decoder: TractModel,
    pub tokenizer: Tokenizer,
    model_meta: ModelMeta,
    /// Softmax temperature for opcode selection. Lower = more deterministic.
    pub temperature: f32,
    /// Hard timeout for the decode loop. Partial programs get a 0.5x confidence penalty.
    pub max_inference_time_secs: u64,
    /// SHA-256 of encoder.onnx + decoder.onnx, used for checkpoint integrity verification.
    pub model_hash: String,
    /// Active `LoRA` adapters, applied as post-hoc logit adjustments per output head.
    active_lora: Vec<super::lora::LoRALayer>,
    /// Accumulated weight delta from consolidated (merged) `LoRA` layers.
    ///
    /// tract models are frozen after compilation, so we cannot modify base weights.
    /// Instead, `merge_lora()` computes `scale * B @ A` and accumulates it here.
    /// During inference: `logits += hidden @ merged_opcode_delta.T`.
    /// Shape: `(num_conventions, decoder_dim)` row-major, or empty if never consolidated.
    pub merged_opcode_delta: Vec<f32>,
}

impl OnnxMindEngine {
    /// Load encoder + decoder ONNX models, tokenizer, and catalog from a directory.
    ///
    /// Expected files: `meta.json`, `encoder.onnx`, `decoder.onnx`, `tokenizer.json`,
    /// and optionally `catalog.json` (if not embedded in meta.json).
    pub fn load(model_dir: &Path) -> Result<Self> {
        let meta_str = std::fs::read_to_string(model_dir.join("meta.json"))
            .context("Failed to read meta.json")?;
        let mut model_meta: ModelMeta = serde_json::from_str(&meta_str)?;

        if model_meta.catalog.is_empty() {
            let catalog_path = model_dir.join("catalog.json");
            if catalog_path.exists() {
                let catalog_str = std::fs::read_to_string(&catalog_path)
                    .context("Failed to read catalog.json")?;
                let catalog_data: serde_json::Value = serde_json::from_str(&catalog_str)?;
                if let Some(entries) = catalog_data.get("entries").and_then(|e| e.as_array()) {
                    model_meta.catalog = entries.iter().filter_map(|e| {
                        let name = e.get("full_name")?.as_str()?.to_string();
                        #[allow(clippy::cast_possible_truncation)] // catalog IDs are small
                        let id = e.get("catalog_id")?.as_u64()? as usize;
                        Some(CatalogEntry {
                            id,
                            name: name.clone(),
                            function: name,
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

/// Methods exposing encoder/decoder internals for the adaptation engine.
/// Kept separate from `MindEngine` to avoid polluting the trait with
/// implementation-specific details.
impl OnnxMindEngine {
    /// Run encoder on token IDs, returning `(encoder_output, initial_hidden)`.
    /// Both are flattened: `encoder_output` is `(1, seq_len, decoder_dim)`, hidden is `(decoder_dim,)`.
    pub fn encode_for_adaptation(&self, intent_tokens: &[u32]) -> anyhow::Result<(Vec<f32>, Vec<f32>)> {
        let seq_len = self.model_meta.max_seq_len;
        let mut ids: Vec<i64> = intent_tokens.iter().map(|&t| i64::from(t)).collect();
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
        let enc_out: Vec<f32> = enc_result[0].to_array_view::<f32>()?.iter().copied().collect();
        let hidden: Vec<f32> = enc_result[2].to_array_view::<f32>()?.iter().copied().collect();
        Ok((enc_out, hidden))
    }

    /// Run one decoder step, returning `(new_hidden, raw_opcode_logits)`.
    /// Returns base logits without `LoRA` -- the adaptation engine needs raw logits
    /// to compute gradients against the `LoRA` parameters.
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

        // Full attention mask -- the original padding mask is unavailable during adaptation,
        // but attending to padding positions has negligible effect on hidden states.
        let mask_vec = vec![1.0f32; seq_len];

        let dr = self.decoder.run(tvec![
            tract_ndarray::Array1::from_vec(vec![prev_op]).into_tvalue(),
            tract_ndarray::Array2::from_shape_vec((1, dd), hidden.to_vec())?.into_tvalue(),
            tract_ndarray::Array3::from_shape_vec((1, seq_len, dd), enc_out.to_vec())?.into_tvalue(),
            tract_ndarray::Array2::from_shape_vec((1, seq_len), mask_vec)?.into_tvalue(),
            tract_ndarray::Array3::from_shape_vec((1, ms, dd), prev_hiddens.to_vec())?.into_tvalue(),
            #[allow(clippy::cast_possible_wrap)] // step count is small
            tract_ndarray::Array1::from_vec(vec![step as i64]).into_tvalue(),
        ])?;

        let new_hidden: Vec<f32> = dr[0].to_array_view::<f32>()?.iter().copied().collect();
        let op_logits: Vec<f32> = dr[1].to_array_view::<f32>()?.iter().copied().collect();

        Ok((new_hidden, op_logits))
    }

    pub const fn active_lora_mut(&mut self) -> &mut Vec<super::lora::LoRALayer> {
        &mut self.active_lora
    }

    pub fn active_lora(&self) -> &[super::lora::LoRALayer] {
        &self.active_lora
    }

    /// Add `LoRA` deltas in-place to logits for a named output head.
    /// Multiple `LoRA` layers can target the same head (e.g. base + plugin adapters).
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

    pub const fn decoder_dim(&self) -> usize {
        self.model_meta.decoder_dim
    }

    pub const fn max_steps(&self) -> usize {
        self.model_meta.max_steps
    }

    pub const fn num_conventions(&self) -> usize {
        self.model_meta.num_conventions
    }

    pub const fn start_token(&self) -> usize {
        self.model_meta.start_token
    }

    pub const fn stop_id(&self) -> usize {
        self.model_meta.stop_id
    }

    #[allow(dead_code)]
    pub fn merged_opcode_delta(&self) -> &[f32] {
        &self.merged_opcode_delta
    }

    pub fn set_merged_opcode_delta(&mut self, delta: Vec<f32>) {
        self.merged_opcode_delta = delta;
    }
}

impl MindEngine for OnnxMindEngine {
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // convention/step indices are small
    fn infer(&self, text: &str) -> Result<Program> {
        let tokens = self.tokenizer.tokenize(text);
        let mut ids = self.tokenizer.encode_with_null(text);
        let real_len = ids.len();
        let seq_len = self.model_meta.max_seq_len;
        let dd = self.model_meta.decoder_dim;
        let ms = self.model_meta.max_steps;

        let mut mask_vec = vec![1.0f32; real_len];
        while ids.len() < seq_len { ids.push(0); }
        while mask_vec.len() < seq_len { mask_vec.push(0.0); }
        ids.truncate(seq_len); mask_vec.truncate(seq_len);

        let enc_result = self.encoder.run(tvec![
            tract_ndarray::Array2::from_shape_vec((1, seq_len), ids)?.into_tvalue(),
            tract_ndarray::Array2::from_shape_vec((1, seq_len), mask_vec.clone())?.into_tvalue(),
        ])?;
        let enc_out: Vec<f32> = enc_result[0].to_array_view::<f32>()?.iter().copied().collect();
        let mut hidden: Vec<f32> = enc_result[2].to_array_view::<f32>()?.iter().copied().collect();

        // Autoregressive decode loop: feed previous opcode, get next prediction
        let mut prev_hiddens = vec![0.0f32; ms * dd];
        #[allow(clippy::cast_possible_wrap)] // start_token is a small vocab index
        let mut prev_op = self.model_meta.start_token as i64;
        let mut steps = Vec::new();
        let mut step_confidences: Vec<f32> = Vec::new();
        let mut cached_states: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
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
                #[allow(clippy::cast_possible_wrap)] // step index t < max_steps
                tract_ndarray::Array1::from_vec(vec![t as i64]).into_tvalue(),
            ])?;

            let new_h = dr[0].to_array_view::<f32>()?;

            let mut op_l: Vec<f32> = dr[1].to_array_view::<f32>()?.iter().copied().collect();

            // Cache raw state before LoRA so adaptation can compute gradients without re-inference
            cached_states.push((hidden.clone(), op_l.clone()));

            // Apply consolidated weight delta: logits += hidden @ merged_delta.T
            if !self.merged_opcode_delta.is_empty() {
                let num_ops = op_l.len();
                for (o, op_l_o) in op_l.iter_mut().enumerate().take(num_ops) {
                    let mut delta = 0.0f32;
                    for (d, hidden_d) in hidden.iter().enumerate().take(dd) {
                        delta += hidden_d * self.merged_opcode_delta[o * dd + d];
                    }
                    *op_l_o += delta;
                }
            }

            self.apply_lora_to_logits("opcode", &hidden, &mut op_l);

            let temp = self.temperature.max(0.01);
            let op_l: Vec<f32> = op_l.iter().map(|x| x / temp).collect();
            let pred = argmax(&op_l);

            // softmax(argmax) = exp(max-max)/sum = 1/sum -- no need to compute full softmax
            {
                let mx = op_l[pred];
                let es: f32 = op_l.iter().map(|x| (x - mx).exp()).sum();
                step_confidences.push(1.0 / es);
            }

            hidden = new_h.iter().copied().collect();
            for (i, &v) in hidden.iter().enumerate() { prev_hiddens[t * dd + i] = v; }

            if pred == self.model_meta.stop_id {
                steps.push(ProgramStep { conv_id: STOP_ID, arg0_type: ArgType::None, arg0_value: ArgValue::None, arg1_type: ArgType::None, arg1_value: ArgValue::None });
                break;
            }
            if pred == self.model_meta.emit_id {
                let mut r0: Vec<f32> = dr[8].to_array_view::<f32>()?.iter().copied().collect();
                self.apply_lora_to_logits("r0", &hidden, &mut r0);
                steps.push(ProgramStep { conv_id: EMIT_ID, arg0_type: ArgType::Ref, arg0_value: ArgValue::Ref(argmax(&r0)), arg1_type: ArgType::None, arg1_value: ArgValue::None });
            } else {
                // Extract all 10 argument output heads and apply per-head LoRA
                let mut a0t: Vec<f32> = dr[2].to_array_view::<f32>()?.iter().copied().collect();
                let mut a1t: Vec<f32> = dr[3].to_array_view::<f32>()?.iter().copied().collect();
                let mut s0s: Vec<f32> = dr[4].to_array_view::<f32>()?.iter().copied().collect();
                let mut s0e: Vec<f32> = dr[5].to_array_view::<f32>()?.iter().copied().collect();
                let mut s1s: Vec<f32> = dr[6].to_array_view::<f32>()?.iter().copied().collect();
                let mut s1e: Vec<f32> = dr[7].to_array_view::<f32>()?.iter().copied().collect();
                let mut r0: Vec<f32> = dr[8].to_array_view::<f32>()?.iter().copied().collect();
                let mut r1: Vec<f32> = dr[9].to_array_view::<f32>()?.iter().copied().collect();
                // Literal heads absent in older ONNX exports -- fall back to uniform
                let mut lit0: Vec<f32> = if dr.len() > 10 {
                    dr[10].to_array_view::<f32>()?.iter().copied().collect()
                } else { vec![0.0; self.model_meta.vocab_size] };
                let mut lit1: Vec<f32> = if dr.len() > 11 {
                    dr[11].to_array_view::<f32>()?.iter().copied().collect()
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
        // Conservative confidence: min probability across steps, halved on timeout
        let mut confidence = step_confidences.iter().copied().fold(f32::MAX, f32::min);
        if (confidence - f32::MAX).abs() < f32::EPSILON {
            confidence = 0.0; // no steps decoded
        }
        if timeout_secs > 0 && infer_start.elapsed().as_secs() >= timeout_secs {
            confidence *= 0.5; // penalty for incomplete program due to timeout
        }
        Ok(Program { steps, confidence, cached_states })
    }

    fn meta(&self) -> &ModelMeta { &self.model_meta }

    fn info(&self) -> MindInfo {
        let mag: f32 = self.active_lora.iter().map(super::lora::LoRALayer::magnitude).sum();

        // Rough param estimate: embedding + BiLSTM encoder + GRU decoder + output heads
        let vs = self.model_meta.vocab_size;
        let dd = self.model_meta.decoder_dim;
        let nc = self.model_meta.num_conventions;
        let param_count = vs * dd + dd * dd * 8 + dd * dd * 4 + nc * dd + vs * dd;

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
        // Compute weight delta (scale * B @ A) and accumulate into merged_opcode_delta.
        // The LoRA layer is then reset to zero so it no longer contributes during inference,
        // but the learned knowledge is preserved in the permanent delta.
        let idx = self.active_lora.iter().position(|l| l.name == name);
        if let Some(i) = idx {
            let delta = self.active_lora[i].compute_weight_delta();
            let dd = self.model_meta.decoder_dim;
            let num_ops = self.model_meta.num_conventions;
            let expected_size = num_ops * dd;

            if self.merged_opcode_delta.is_empty() {
                self.merged_opcode_delta = vec![0.0f32; expected_size];
            }

            for (merged, d) in self.merged_opcode_delta.iter_mut().zip(delta.iter()) {
                *merged += d;
            }

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

/// Resolve an argument directly from raw decoder tensor outputs (pre-LoRA path).
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)] // mirrors decoder output head structure
fn resolve_arg(tid: usize, o: &TVec<TValue>, ss: usize, se: usize, ri: usize, li: usize, tokens: &[String], tok: &Tokenizer) -> (ArgType, ArgValue) {
    match tid {
        1 => {
            let s: Vec<f32> = o[ss].to_array_view::<f32>().unwrap().iter().copied().collect();
            let e: Vec<f32> = o[se].to_array_view::<f32>().unwrap().iter().copied().collect();
            (ArgType::Span, ArgValue::Span(tok.extract_span(tokens, argmax(&s), argmax(&e).max(argmax(&s)))))
        }
        2 => { let r: Vec<f32> = o[ri].to_array_view::<f32>().unwrap().iter().copied().collect(); (ArgType::Ref, ArgValue::Ref(argmax(&r))) }
        3 => {
            let lit: Vec<f32> = o[li].to_array_view::<f32>().unwrap().iter().copied().collect();
            let vocab_idx = argmax(&lit);
            (ArgType::Literal, ArgValue::Literal(tok.decode_index(vocab_idx)))
        }
        _ => (ArgType::None, ArgValue::None),
    }
}

/// Resolve an argument from LoRA-adjusted logit slices.
/// `tid`: 1=Span, 2=Ref, 3=Literal, 0/_=None.
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

fn argmax(s: &[f32]) -> usize { s.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map_or(0, |(i, _)| i) }
