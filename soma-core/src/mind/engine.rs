//! SOMA Mind Engine — ONNX inference via tract.
//!
//! Loads encoder.onnx + decoder.onnx.
//! Runs autoregressive program generation: one decoder step per program op.

use anyhow::{Context, Result};
use std::path::Path;
use tract_onnx::prelude::*;

use super::tokenizer::Tokenizer;

#[derive(Debug, Clone)]
pub struct ProgramStep {
    pub conv_id: i32,
    pub arg0_type: ArgType,
    pub arg0_value: ArgValue,
    pub arg1_type: ArgType,
    pub arg1_value: ArgValue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArgType { None, Span, Ref }

#[derive(Debug, Clone)]
pub enum ArgValue {
    None,
    Span(String),
    Ref(usize),
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelMeta {
    pub vocab_size: usize,
    pub num_conventions: usize,
    pub max_steps: usize,
    pub max_seq_len: usize,
    pub decoder_dim: usize,
    pub emit_id: usize,
    pub stop_id: usize,
    pub start_token: usize,
    pub catalog: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogEntry {
    pub id: usize,
    pub name: String,
    pub function: String,
    pub call_pattern: String,
    pub var_args: Vec<VarArg>,
    pub description: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct VarArg {
    pub name: String,
    #[serde(rename = "type")]
    pub arg_type: String,
}

pub const EMIT_ID: i32 = -1;
pub const STOP_ID: i32 = -2;

type TractModel = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub struct MindEngine {
    encoder: TractModel,
    decoder: TractModel,
    pub tokenizer: Tokenizer,
    pub meta: ModelMeta,
}

impl MindEngine {
    pub fn load(model_dir: &Path) -> Result<Self> {
        let meta_str = std::fs::read_to_string(model_dir.join("meta.json"))
            .context("Failed to read meta.json")?;
        let meta: ModelMeta = serde_json::from_str(&meta_str)?;

        tracing::info!("Loading encoder.onnx...");
        let encoder = tract_onnx::onnx()
            .model_for_path(model_dir.join("encoder.onnx"))?
            .into_optimized()?
            .into_runnable()?;

        tracing::info!("Loading decoder.onnx...");
        let decoder = tract_onnx::onnx()
            .model_for_path(model_dir.join("decoder.onnx"))?
            .into_optimized()?
            .into_runnable()?;

        let tokenizer = Tokenizer::load(&model_dir.join("tokenizer.json"))?;

        tracing::info!(
            vocab = tokenizer.vocab_size(),
            conventions = meta.num_conventions,
            "Mind engine loaded"
        );

        Ok(Self { encoder, decoder, tokenizer, meta })
    }

    pub fn predict(&self, text: &str) -> Result<(Vec<ProgramStep>, f32)> {
        let tokens = self.tokenizer.tokenize(text);
        let mut ids = self.tokenizer.encode_with_null(text);
        let real_len = ids.len();
        let seq_len = self.meta.max_seq_len;
        let decoder_dim = self.meta.decoder_dim;
        let max_steps = self.meta.max_steps;

        // Pad to fixed max_seq_len
        let mut mask_vec = vec![1.0f32; real_len];
        while ids.len() < seq_len { ids.push(0); }
        while mask_vec.len() < seq_len { mask_vec.push(0.0); }
        ids.truncate(seq_len);
        mask_vec.truncate(seq_len);

        // Encode
        let input_ids = tract_ndarray::Array2::from_shape_vec(
            (1, seq_len), ids)?;
        let mask = tract_ndarray::Array2::from_shape_vec(
            (1, seq_len), mask_vec)?;

        let enc_result = self.encoder.run(tvec![
            input_ids.into_tvalue(),
            mask.into_tvalue(),
        ])?;

        let enc_out_tensor = enc_result[0].to_array_view::<f32>()?;
        let init_hidden_tensor = enc_result[2].to_array_view::<f32>()?;

        // Decoder loop
        let mut hidden: Vec<f32> = init_hidden_tensor.iter().cloned().collect();
        let mut prev_hiddens = vec![0.0f32; max_steps * decoder_dim];
        let enc_out: Vec<f32> = enc_out_tensor.iter().cloned().collect();
        // Use same mask as encoder (with padding zeros)
        let enc_mask: Vec<f32> = {
            let mut m = vec![1.0f32; real_len];
            while m.len() < seq_len { m.push(0.0); }
            m
        };
        let mut prev_op = self.meta.start_token as i64;

        let mut steps = Vec::new();
        let mut first_confidence = 0.0f32;

        for t in 0..max_steps {
            let prev_op_arr = tract_ndarray::Array1::from_vec(vec![prev_op]);
            let hidden_arr = tract_ndarray::Array2::from_shape_vec(
                (1, decoder_dim), hidden.clone())?;
            let enc_out_arr = tract_ndarray::Array3::from_shape_vec(
                (1, seq_len, decoder_dim), enc_out.clone())?;
            let enc_mask_arr = tract_ndarray::Array2::from_shape_vec(
                (1, seq_len), enc_mask.clone())?;
            let prev_h_arr = tract_ndarray::Array3::from_shape_vec(
                (1, max_steps, decoder_dim), prev_hiddens.clone())?;
            let num_prev_arr = tract_ndarray::Array1::from_vec(vec![t as i64]);

            let dec_result = self.decoder.run(tvec![
                prev_op_arr.into_tvalue(),
                hidden_arr.into_tvalue(),
                enc_out_arr.into_tvalue(),
                enc_mask_arr.into_tvalue(),
                prev_h_arr.into_tvalue(),
                num_prev_arr.into_tvalue(),
            ])?;

            let new_hidden = dec_result[0].to_array_view::<f32>()?;
            let op_logits = dec_result[1].to_array_view::<f32>()?;

            // Argmax opcode
            let op_slice: Vec<f32> = op_logits.iter().cloned().collect();
            let pred_op = argmax(&op_slice);

            if t == 0 {
                let max_val = op_slice[pred_op];
                let exp_sum: f32 = op_slice.iter().map(|x| (x - max_val).exp()).sum();
                first_confidence = 1.0 / exp_sum;
            }

            // Update hidden
            hidden = new_hidden.iter().cloned().collect();
            for (i, &v) in hidden.iter().enumerate() {
                prev_hiddens[t * decoder_dim + i] = v;
            }

            if pred_op == self.meta.stop_id {
                steps.push(ProgramStep {
                    conv_id: STOP_ID,
                    arg0_type: ArgType::None, arg0_value: ArgValue::None,
                    arg1_type: ArgType::None, arg1_value: ArgValue::None,
                });
                break;
            }

            if pred_op == self.meta.emit_id {
                let r0: Vec<f32> = dec_result[8].to_array_view::<f32>()?.iter().cloned().collect();
                let ref_idx = argmax(&r0);
                steps.push(ProgramStep {
                    conv_id: EMIT_ID,
                    arg0_type: ArgType::Ref, arg0_value: ArgValue::Ref(ref_idx),
                    arg1_type: ArgType::None, arg1_value: ArgValue::None,
                });
            } else {
                let a0t_logits: Vec<f32> = dec_result[2].to_array_view::<f32>()?.iter().cloned().collect();
                let a1t_logits: Vec<f32> = dec_result[3].to_array_view::<f32>()?.iter().cloned().collect();
                let a0t = argmax(&a0t_logits);
                let a1t = argmax(&a1t_logits);

                let (a0_type, a0_val) = resolve_arg(a0t, &dec_result, 4, 5, 8, &tokens, &self.tokenizer);
                let (a1_type, a1_val) = resolve_arg(a1t, &dec_result, 6, 7, 9, &tokens, &self.tokenizer);

                steps.push(ProgramStep {
                    conv_id: pred_op as i32,
                    arg0_type: a0_type, arg0_value: a0_val,
                    arg1_type: a1_type, arg1_value: a1_val,
                });
            }

            prev_op = pred_op as i64;
        }

        Ok((steps, first_confidence))
    }
}

fn resolve_arg(
    type_id: usize,
    outputs: &TVec<TValue>,
    span_s_idx: usize,
    span_e_idx: usize,
    ref_idx: usize,
    tokens: &[String],
    tokenizer: &Tokenizer,
) -> (ArgType, ArgValue) {
    match type_id {
        0 => (ArgType::None, ArgValue::None),
        1 => {
            let ss: Vec<f32> = outputs[span_s_idx].to_array_view::<f32>().unwrap().iter().cloned().collect();
            let se: Vec<f32> = outputs[span_e_idx].to_array_view::<f32>().unwrap().iter().cloned().collect();
            let s = argmax(&ss);
            let e = argmax(&se).max(s);
            let text = tokenizer.extract_span(tokens, s, e);
            (ArgType::Span, ArgValue::Span(text))
        }
        2 => {
            let r: Vec<f32> = outputs[ref_idx].to_array_view::<f32>().unwrap().iter().cloned().collect();
            let idx = argmax(&r);
            (ArgType::Ref, ArgValue::Ref(idx))
        }
        _ => (ArgType::None, ArgValue::None),
    }
}

fn argmax(slice: &[f32]) -> usize {
    slice.iter().enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

impl ProgramStep {
    pub fn format(&self, step_idx: usize, catalog: &[CatalogEntry]) -> String {
        if self.conv_id == STOP_ID { return "STOP".to_string(); }
        if self.conv_id == EMIT_ID {
            let r = match &self.arg0_value {
                ArgValue::Ref(r) => format!("${}", r),
                _ => "?".to_string(),
            };
            return format!("${} = EMIT({})", step_idx, r);
        }
        let conv = &catalog[self.conv_id as usize];
        let mut args = Vec::new();
        for val in [&self.arg0_value, &self.arg1_value] {
            match val {
                ArgValue::Span(s) => args.push(format!("\"{}\"", s)),
                ArgValue::Ref(r) => args.push(format!("${}", r)),
                ArgValue::None => {}
            }
        }
        format!("${} = libc.{}({})", step_idx, conv.function, args.join(", "))
    }
}
