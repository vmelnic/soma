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
use tract_onnx::prelude::*;

use super::{
    ArgType, ArgValue, MindEngine, MindInfo, ModelMeta,
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
    /// SHA-256 hash of encoder.onnx + decoder.onnx, computed at load time.
    pub model_hash: String,
    /// Active LoRA layers applied to decoder output heads (Section 4.7).
    /// LoRA is applied as post-hoc logit adjustment: logits += scale * (hidden @ A.T) @ B.T
    active_lora: Vec<super::lora::LoRALayer>,
}

impl OnnxMindEngine {
    pub fn load(model_dir: &Path) -> Result<Self> {
        let meta_str = std::fs::read_to_string(model_dir.join("meta.json"))
            .context("Failed to read meta.json")?;
        let model_meta: ModelMeta = serde_json::from_str(&meta_str)?;

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
        Ok(Self { encoder, decoder, tokenizer, model_meta, temperature: 1.0, model_hash, active_lora: Vec::new() })
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
        let mut confidence = 0.0f32;

        for t in 0..ms {
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

            // Apply LoRA to opcode logits (Section 4.7)
            // Post-hoc adjustment: logits += scale * (hidden @ A.T) @ B.T
            for lora in &self.active_lora {
                if lora.name == "opcode" && !lora.b.is_empty() {
                    // lora delta = hidden @ A.T @ B.T * scale
                    // hidden is dd-dimensional, A is rank x dd, B is num_ops x rank
                    let rank = lora.rank;
                    // h @ A.T → rank-dimensional
                    let mut ha = vec![0.0f32; rank];
                    for r in 0..rank {
                        for d in 0..dd.min(lora.a.len() / rank) {
                            ha[r] += hidden[d] * lora.a[r * dd + d];
                        }
                    }
                    // ha @ B.T → num_ops-dimensional delta
                    let num_ops = op_l.len();
                    for o in 0..num_ops.min(lora.b.len() / rank) {
                        let mut delta = 0.0f32;
                        for r in 0..rank {
                            delta += ha[r] * lora.b[o * rank + r];
                        }
                        op_l[o] += delta * lora.scale;
                    }
                }
            }

            // Apply temperature scaling to logits (Section 2.3: deterministic execution)
            // Clamp to minimum 0.01 to prevent division by zero
            let temp = self.temperature.max(0.01);
            let op_l: Vec<f32> = op_l.iter().map(|x| x / temp).collect();
            let pred = argmax(&op_l);

            if t == 0 {
                let mx = op_l[pred];
                let es: f32 = op_l.iter().map(|x| (x - mx).exp()).sum();
                confidence = 1.0 / es;
            }

            hidden = new_h.iter().cloned().collect();
            for (i, &v) in hidden.iter().enumerate() { prev_hiddens[t * dd + i] = v; }

            if pred == self.model_meta.stop_id {
                steps.push(ProgramStep { conv_id: STOP_ID, arg0_type: ArgType::None, arg0_value: ArgValue::None, arg1_type: ArgType::None, arg1_value: ArgValue::None });
                break;
            }
            if pred == self.model_meta.emit_id {
                let r0: Vec<f32> = dr[8].to_array_view::<f32>()?.iter().cloned().collect();
                steps.push(ProgramStep { conv_id: EMIT_ID, arg0_type: ArgType::Ref, arg0_value: ArgValue::Ref(argmax(&r0)), arg1_type: ArgType::None, arg1_value: ArgValue::None });
            } else {
                let a0t: Vec<f32> = dr[2].to_array_view::<f32>()?.iter().cloned().collect();
                let a1t: Vec<f32> = dr[3].to_array_view::<f32>()?.iter().cloned().collect();
                let (a0ty, a0v) = resolve_arg(argmax(&a0t), &dr, 4, 5, 8, &tokens, &self.tokenizer);
                let (a1ty, a1v) = resolve_arg(argmax(&a1t), &dr, 6, 7, 9, &tokens, &self.tokenizer);
                steps.push(ProgramStep { conv_id: pred as i32, arg0_type: a0ty, arg0_value: a0v, arg1_type: a1ty, arg1_value: a1v });
            }
            prev_op = pred as i64;
        }
        Ok(Program { steps, confidence })
    }

    fn meta(&self) -> &ModelMeta { &self.model_meta }

    fn info(&self) -> MindInfo {
        let mag: f32 = self.active_lora.iter().map(|l| l.magnitude()).sum();
        MindInfo {
            backend: "OnnxMindEngine (tract)".into(),
            param_count: 0,
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

    fn detach_lora(&mut self, name: &str) -> Result<()> {
        self.active_lora.retain(|l| l.name != name);
        tracing::info!(name, "LoRA detached");
        Ok(())
    }

    fn merge_lora(&mut self, name: &str) -> Result<()> {
        // In tract, we can't modify base weights (they're compiled into the graph).
        // Instead, consolidation means: the LoRA has been validated and should remain permanently.
        // For now, log the intent. Full merge requires re-exporting the ONNX model.
        tracing::info!(name, "LoRA merge requested (tract: weights remain as runtime overlay)");
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

fn resolve_arg(tid: usize, o: &TVec<TValue>, ss: usize, se: usize, ri: usize, tokens: &[String], tok: &Tokenizer) -> (ArgType, ArgValue) {
    match tid {
        1 => {
            let s: Vec<f32> = o[ss].to_array_view::<f32>().unwrap().iter().cloned().collect();
            let e: Vec<f32> = o[se].to_array_view::<f32>().unwrap().iter().cloned().collect();
            (ArgType::Span, ArgValue::Span(tok.extract_span(tokens, argmax(&s), argmax(&e).max(argmax(&s)))))
        }
        2 => { let r: Vec<f32> = o[ri].to_array_view::<f32>().unwrap().iter().cloned().collect(); (ArgType::Ref, ArgValue::Ref(argmax(&r))) }
        _ => (ArgType::None, ArgValue::None),
    }
}

fn argmax(s: &[f32]) -> usize { s.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(i, _)| i).unwrap_or(0) }
