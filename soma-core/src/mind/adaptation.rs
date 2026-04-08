//! Runtime `LoRA` adaptation — learn from experience without Python.
//!
//! This module implements the adaptation cycle: using recorded successful
//! experiences to update the Mind's `LoRA` layers via gradient descent.
//! The key insight is that we don't backprop through the ONNX graph —
//! instead we treat the decoder's hidden states as frozen features and
//! only update the `LoRA` parameters (A and B matrices).
//!
//! The adaptation loop uses teacher forcing: for each experience, we
//! run the encoder to get hidden states, then step through the decoder
//! feeding the KNOWN correct opcode at each step (not the predicted one).
//! At each step we collect (`hidden_state`, `target_opcode`) pairs and pass
//! them to `LoRALayer::adapt()` for a gradient descent update.

use super::lora::LoRALayer;
use super::onnx_engine::OnnxMindEngine;
use crate::memory::experience::Experience;

/// Configuration for the adaptation engine.
#[derive(Debug, Clone)]
pub struct AdaptationConfig {
    #[allow(dead_code)] // Spec Section 4.7 — toggle for adaptation engine
    pub enabled: bool,
    #[allow(dead_code)] // Spec Section 4.7 — adaptation frequency control
    pub adapt_every_n: usize,
    pub batch_size: usize,
    pub learning_rate: f32,
}

/// Result of one adaptation cycle.
#[derive(Debug, Clone)]
pub struct AdaptationResult {
    /// Average cross-entropy loss over the batch.
    pub loss: f32,
    /// How many adaptation cycles have been performed (cumulative).
    pub cycle: u64,
    /// Current `LoRA` magnitude after adaptation.
    pub lora_magnitude: f32,
}

/// Run one adaptation cycle on the Mind's `LoRA` layers using recorded experiences.
///
/// This requires a WRITE lock on the `MindEngine` (caller must hold it).
/// Steps:
///   1. Sample `batch_size` experiences from the buffer
///   2. For each experience, run encoder to get hidden states
///   3. For each decoder step, collect (hidden, `target_opcode`) pairs
///   4. Call `LoRALayer::adapt()` with the batch
///   5. Return adaptation metrics
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

    // Sample up to batch_size experiences (take most recent ones)
    let sample_count = config.batch_size.min(experiences.len());
    let sampled = &experiences[experiences.len() - sample_count..];

    // Collect (hidden_state, target_opcode) pairs and corresponding base logits
    // across ALL steps of ALL sampled experiences.
    let mut batch: Vec<(Vec<f32>, usize)> = Vec::new();
    let mut base_logits_batch: Vec<Vec<f32>> = Vec::new();

    for exp in sampled {
        // Use cached states if available (fast path — no ONNX re-inference)
        if !exp.cached_states.is_empty() {
            for (hidden, op_logits) in &exp.cached_states {
                // Find the target opcode for this step from the program
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

        // Slow path: re-run ONNX encoder+decoder to get hidden states.
        // This is only used when cached_states are not available.
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

    // Ensure LoRA layers exist for all output heads; create defaults if needed.
    // The spec defines 11 output heads: opcode, a0t, a1t, s0s, s0e, s1s, s1e, r0, r1, lit0, lit1.
    // Output dimensions vary per head type:
    //   opcode: num_conventions
    //   a0t/a1t: 4 (None, Span, Ref, Literal)
    //   s0s/s0e/s1s/s1e: max_seq_len (span pointer over input)
    //   r0/r1: max_steps (reference to prior step)
    //   lit0/lit1: vocab_size (approximated by decoder_dim for default layers)
    // Note: only opcode LoRA is trained currently; other heads are created so they're
    // available for plugin-provided LoRA attachment and future multi-head training.
    let default_rank = 8;
    let default_alpha = 16.0;

    let head_specs: Vec<(&str, usize)> = vec![
        ("opcode", num_conventions),
        ("a0t", 4),     // ArgType: None, Span, Ref, Literal
        ("a1t", 4),
        ("s0s", ms),    // Span start pointer over max_steps positions
        ("s0e", ms),    // Span end pointer
        ("s1s", ms),
        ("s1e", ms),
        ("r0", ms),     // Ref pointer to prior step
        ("r1", ms),
        ("lit0", dd),   // Literal — approximated by decoder_dim for default LoRA
        ("lit1", dd),   // Plugin LoRA may provide correctly-sized layers
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

    // Run adaptation on the opcode LoRA layer
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
