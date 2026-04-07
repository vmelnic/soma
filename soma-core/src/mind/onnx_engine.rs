//! OnnxMindEngine — ONNX inference via tract for server/desktop targets.
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
        tracing::info!(vocab = tokenizer.vocab_size(), conventions = model_meta.num_conventions, "OnnxMindEngine loaded");
        Ok(Self { encoder, decoder, tokenizer, model_meta, temperature: 1.0 })
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
            // Apply temperature scaling to logits (Section 2.3: deterministic execution)
            // Clamp to minimum 0.01 to prevent division by zero
            let temp = self.temperature.max(0.01);
            let op_l: Vec<f32> = dr[1].to_array_view::<f32>()?.iter()
                .map(|x| x / temp)
                .collect();
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
        MindInfo {
            backend: "OnnxMindEngine (tract)".into(),
            param_count: 0,
            conventions_known: self.model_meta.num_conventions,
            max_steps: self.model_meta.max_steps,
            lora_layers: 0,
            lora_magnitude: 0.0,
        }
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
