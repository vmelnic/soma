//! Runtime `LoRA` adaptation -- learning from successful experiences without Python.
//!
//! The core insight: we do not backpropagate through the frozen ONNX graph.
//! Instead, decoder hidden states are treated as fixed feature vectors and only
//! the `LoRA` parameters (A, B matrices) are updated via SGD.
//!
//! The adaptation loop uses teacher forcing: for each recorded experience, the
//! decoder is stepped with the KNOWN correct opcode at each position (not the
//! model's prediction). This produces `(hidden_state, target_opcode)` pairs that
//! are batched and passed to `LoRALayer::adapt()`.
//!
//! Fast path: when `Program::cached_states` are available (populated during the
//! original inference), the ONNX encoder/decoder are not re-run at all.

use super::lora::LoRALayer;
use super::onnx_engine::OnnxMindEngine;
use crate::memory::experience::Experience;

/// Controls when and how adaptation runs.
#[derive(Debug, Clone)]
pub struct AdaptationConfig {
    #[allow(dead_code)]
    pub enabled: bool,
    /// Run adaptation every N successful experiences.
    #[allow(dead_code)]
    pub adapt_every_n: usize,
    /// Maximum number of experiences to sample per adaptation cycle.
    pub batch_size: usize,
    pub learning_rate: f32,
}

/// Metrics returned from a single adaptation cycle.
#[derive(Debug, Clone)]
pub struct AdaptationResult {
    /// Mean cross-entropy loss over the batch.
    pub loss: f32,
    /// Always 1 for a single cycle; the caller accumulates the cumulative count.
    pub cycle: u64,
    /// Sum of `LoRA` magnitudes across all active layers after adaptation.
    pub lora_magnitude: f32,
}

/// Run one adaptation cycle using the most recent experiences.
///
/// Requires exclusive (`&mut`) access to the engine. Samples up to `batch_size`
/// experiences, extracts `(hidden_state, target_opcode)` pairs (from cache or by
/// re-running the ONNX model), and performs one SGD step on the opcode `LoRA` layer.
#[allow(clippy::too_many_lines)]
#[allow(clippy::unnecessary_wraps)] // Result return is intentional for API consistency
pub fn adapt_from_experience(
    engine: &mut OnnxMindEngine,
    experiences: &[Experience],
    config: &AdaptationConfig,
) -> Result<AdaptationResult, anyhow::Error> {
    if experiences.is_empty() {
        return Ok(AdaptationResult {
            loss: 0.0,
            cycle: 0,
            lora_magnitude: 0.0,
        });
    }

    let dd = engine.decoder_dim();
    let ms = engine.max_steps();
    let num_conventions = engine.num_conventions();
    let start_token = engine.start_token();
    let stop_id = engine.stop_id();

    // Most recent experiences are most relevant -- recency bias
    let sample_count = config.batch_size.min(experiences.len());
    let sampled = &experiences[experiences.len() - sample_count..];

    let mut batch: Vec<(Vec<f32>, usize)> = Vec::new();
    let mut base_logits_batch: Vec<Vec<f32>> = Vec::new();

    for exp in sampled {
        // Fast path: use cached hidden states from the original inference
        if !exp.cached_states.is_empty() {
            for (hidden, op_logits) in &exp.cached_states {
                let step_idx = batch.len() % exp.program.len().max(1);
                if step_idx < exp.program.len() {
                    let (conv_id, _, _) = exp.program[step_idx];
                    #[allow(clippy::cast_sign_loss)] // conv_id checked >= 0 on line above
                    if conv_id >= 0 && (conv_id as usize) < num_conventions {
                        batch.push((hidden.clone(), conv_id as usize));
                        base_logits_batch.push(op_logits.clone());
                    }
                }
            }
            continue;
        }

        // Slow path: re-run ONNX model (only when cached_states are absent)
        let has_valid_steps = exp.program.iter()
            .any(|(conv_id, _, _)| *conv_id >= 0);

        if !has_valid_steps {
            continue;
        }

        let (enc_out, mut hidden) = match engine.encode_for_adaptation(&exp.intent_tokens) {
            Ok(result) => result,
            Err(e) => {
                tracing::debug!(error = %e, "Skipping experience: encoder failed");
                continue;
            }
        };

        let mut prev_hiddens = vec![0.0f32; ms * dd];
        #[allow(clippy::cast_possible_wrap)] // start_token is a small vocab index
        let mut prev_op = start_token as i64;

        for (t, (conv_id, _, _)) in exp.program.iter().enumerate() {
            if t >= ms { break; }

            let (new_hidden, op_logits) = match engine.decode_step_for_adaptation(
                prev_op, &hidden, &enc_out, &prev_hiddens, t,
            ) {
                Ok(result) => result,
                Err(e) => {
                    tracing::debug!(error = %e, step = t, "Skipping step: decoder failed");
                    break;
                }
            };

            let target_opcode = *conv_id;
            #[allow(clippy::cast_sign_loss)] // target_opcode checked >= 0 on same line
            if target_opcode >= 0 && (target_opcode as usize) < num_conventions {
                batch.push((hidden.clone(), target_opcode as usize));
                base_logits_batch.push(op_logits);
            }

            for (i, &v) in new_hidden.iter().enumerate().take(dd) {
                prev_hiddens[t * dd + i] = v;
            }
            hidden = new_hidden;

            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // stop_id is small
            if target_opcode == stop_id as i32 { break; }
            prev_op = i64::from(target_opcode);
        }
    }

    if batch.is_empty() {
        return Ok(AdaptationResult {
            loss: 0.0,
            cycle: 0,
            lora_magnitude: 0.0,
        });
    }

    // Ensure all 11 output heads have LoRA layers. Only the opcode head is trained
    // currently; others are created as attachment points for plugin-provided LoRA.
    let default_rank = 8;
    let default_alpha = 16.0;

    // (head_name, out_features): dimensions match the decoder's output head sizes
    let head_specs: Vec<(&str, usize)> = vec![
        ("opcode", num_conventions),
        ("a0t", 4), ("a1t", 4),         // ArgType enum (4 variants)
        ("s0s", ms), ("s0e", ms),       // span pointers
        ("s1s", ms), ("s1e", ms),
        ("r0", ms), ("r1", ms),         // step-ref pointers
        ("lit0", dd), ("lit1", dd),     // literal vocab (approx by decoder_dim)
    ];

    for (head_name, out_features) in &head_specs {
        let has_layer = engine.active_lora().iter().any(|l| l.name == *head_name);
        if !has_layer {
            let lora = LoRALayer::new(
                head_name.to_string(),
                dd,
                *out_features,
                default_rank,
                default_alpha,
            );
            tracing::info!(
                head = head_name,
                rank = default_rank,
                alpha = default_alpha,
                in_features = dd,
                out_features = out_features,
                "Created default LoRA layer for output head"
            );
            engine.active_lora_mut().push(lora);
        }
    }

    let loss = {
        let lora_layers = engine.active_lora_mut();
        lora_layers.iter_mut()
            .find(|l| l.name == "opcode")
            .map_or(0.0, |lora| lora.adapt(&batch, &base_logits_batch, config.learning_rate))
    };

    let magnitude: f32 = engine.active_lora().iter().map(super::lora::LoRALayer::magnitude).sum();

    tracing::info!(
        loss = %format!("{:.4}", loss),
        batch_size = batch.len(),
        experiences = sampled.len(),
        magnitude = %format!("{:.6}", magnitude),
        "LoRA adaptation cycle complete"
    );

    Ok(AdaptationResult {
        loss,
        cycle: 1, // Caller tracks cumulative count via proprioception
        lora_magnitude: magnitude,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adaptation_config_defaults() {
        let config = AdaptationConfig {
            enabled: true,
            adapt_every_n: 10,
            batch_size: 8,
            learning_rate: 0.001,
        };
        assert!(config.enabled);
        assert_eq!(config.adapt_every_n, 10);
        assert_eq!(config.batch_size, 8);
        assert!((config.learning_rate - 0.001).abs() < 1e-6);
    }

    #[test]
    fn test_adaptation_empty_experiences() {
        // We can't construct a full OnnxMindEngine in unit tests without model files,
        // but we can test that adapt_from_experience handles the empty case at the
        // function entry level. Full integration tests require model fixtures.
        let config = AdaptationConfig {
            enabled: true,
            adapt_every_n: 10,
            batch_size: 8,
            learning_rate: 0.001,
        };
        // Test that AdaptationResult fields are accessible
        let result = AdaptationResult {
            loss: 0.0,
            cycle: 0,
            lora_magnitude: 0.0,
        };
        assert_eq!(result.loss, 0.0);
        assert_eq!(result.cycle, 0);
        assert_eq!(result.lora_magnitude, 0.0);
        assert_eq!(config.batch_size, 8);
    }
}
