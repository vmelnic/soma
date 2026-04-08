//! `LoRA` consolidation — permanently merging adapted weights into the base model (Spec Section 6.3).
//!
//! Because tract-onnx models are frozen (compiled into the inference graph), we cannot
//! modify base weights in-place. Instead, consolidation computes `scale * B @ A` for
//! each eligible `LoRA` layer and accumulates it into `OnnxMindEngine::merged_opcode_delta`.
//! During inference, this delta is applied as `logits += hidden @ delta.T`, equivalent
//! to having modified the original weight matrix.
//!
//! The 5-step process:
//!   1. Evaluate `LoRA` magnitude per layer
//!   2. Merge high-magnitude layers into `merged_opcode_delta`
//!   3. Reset merged `LoRA` layers (A to small random, B to zero)
//!   4. Save updated checkpoint (caller responsibility)
//!   5. Log consolidation event for auditability

use crate::mind::onnx_engine::OnnxMindEngine;

/// Governs when `LoRA` layers are eligible for permanent consolidation.
pub struct ConsolidationConfig {
    /// Minimum Frobenius norm of B matrix to consider a layer significantly adapted.
    pub min_lora_magnitude: f32,
    /// Minimum adaptation count before consolidation is even considered.
    pub threshold: u64,
}

/// Outcome of a consolidation pass.
pub struct ConsolidationResult {
    /// Total `LoRA` layers inspected.
    pub layers_evaluated: usize,
    /// Layers whose magnitude exceeded the threshold and were merged.
    pub layers_merged: usize,
    /// Peak `LoRA` magnitude remaining after consolidation (should be near zero if all merged).
    pub new_magnitude: f32,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            min_lora_magnitude: 0.01,
            threshold: 100,
        }
    }
}

impl ConsolidationConfig {
    pub const fn new(min_lora_magnitude: f32, threshold: u64) -> Self {
        Self {
            min_lora_magnitude,
            threshold,
        }
    }

    /// Returns `true` when both the adaptation count and peak `LoRA` magnitude
    /// meet their respective thresholds.
    pub fn should_consolidate(&self, adaptation_count: u64, max_magnitude: f32) -> bool {
        adaptation_count >= self.threshold && max_magnitude >= self.min_lora_magnitude
    }

    /// Execute the consolidation pass on an `OnnxMindEngine`.
    ///
    /// Evaluates each `LoRA` layer's magnitude, merges eligible layers into the
    /// permanent `merged_opcode_delta`, and resets their weights. The caller is
    /// responsible for persisting the updated checkpoint (step 4).
    pub fn consolidate(&self, engine: &mut OnnxMindEngine) -> ConsolidationResult {
        let dd = engine.decoder_dim();
        let num_ops = engine.num_conventions();
        let expected_size = num_ops * dd;

        let lora_layers = engine.active_lora_mut();
        let layers_evaluated = lora_layers.len();
        let mut layers_merged = 0usize;

        // Lazily initialize the delta accumulator on first consolidation
        if engine.merged_opcode_delta.is_empty() && layers_evaluated > 0 {
            engine.merged_opcode_delta = vec![0.0f32; expected_size];
        }

        // Two-pass: collect eligible deltas first, then apply, to avoid
        // borrowing active_lora immutably and mutably in the same loop.
        let mut merge_actions: Vec<(usize, Vec<f32>)> = Vec::new();

        for (idx, lora) in engine.active_lora().iter().enumerate() {
            let mag = lora.magnitude();
            tracing::debug!(
                name = %lora.name,
                magnitude = mag,
                threshold = self.min_lora_magnitude,
                "Evaluating LoRA layer for consolidation"
            );

            if mag >= self.min_lora_magnitude {
                let delta = lora.compute_weight_delta();
                tracing::info!(
                    name = %lora.name,
                    magnitude = mag,
                    delta_size = delta.len(),
                    delta_nonzero = delta.iter().filter(|&&v| v != 0.0).count(),
                    "LoRA layer eligible for consolidation"
                );
                merge_actions.push((idx, delta));
            }
        }

        for (idx, delta) in &merge_actions {
            // Accumulate delta into permanent weight offset
            let merge_len = delta.len().min(engine.merged_opcode_delta.len());
            for (target, source) in engine.merged_opcode_delta[..merge_len].iter_mut().zip(&delta[..merge_len]) {
                *target += source;
            }

            // Reset this layer so it can learn fresh adaptations
            engine.active_lora_mut()[*idx].reset();
            layers_merged += 1;
        }

        let new_magnitude: f32 = engine.active_lora().iter()
            .map(super::super::mind::lora::LoRALayer::magnitude)
            .fold(0.0f32, f32::max);

        if layers_merged > 0 {
            let total_delta_magnitude: f32 = engine.merged_opcode_delta.iter()
                .map(|x| x.abs())
                .sum();
            tracing::info!(
                layers_evaluated,
                layers_merged,
                new_magnitude,
                total_delta_magnitude,
                "Consolidation complete: LoRA knowledge merged into permanent weight delta"
            );
        } else {
            tracing::info!(
                layers_evaluated,
                "Consolidation: no layers exceeded magnitude threshold"
            );
        }

        ConsolidationResult {
            layers_evaluated,
            layers_merged,
            new_magnitude,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::lora::LoRALayer;

    #[test]
    fn test_should_consolidate_both_conditions() {
        let config = ConsolidationConfig::new(0.01, 100);
        assert!(!config.should_consolidate(99, 0.02));   // below adaptation count
        assert!(!config.should_consolidate(100, 0.005)); // below magnitude
        assert!(config.should_consolidate(100, 0.01));   // exactly at threshold
        assert!(config.should_consolidate(200, 0.5));    // above both
    }

    #[test]
    fn test_should_consolidate_defaults() {
        let config = ConsolidationConfig::default();
        assert_eq!(config.threshold, 100);
        assert_eq!(config.min_lora_magnitude, 0.01);
        assert!(!config.should_consolidate(50, 0.1));
        assert!(config.should_consolidate(100, 0.1));
    }

    #[test]
    fn test_lora_compute_weight_delta_zero_b() {
        // B is all zeros => delta should be all zeros
        let lora = LoRALayer::new("opcode".into(), 4, 3, 2, 1.0);
        let delta = lora.compute_weight_delta();
        assert_eq!(delta.len(), 3 * 4); // out_features * in_features
        assert!(delta.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_lora_compute_weight_delta_identity() {
        // Manual test: rank=1, in=2, out=2
        // A = [1.0, 0.0] (rank=1, in=2)
        // B = [1.0; 0.0] (out=2, rank=1)
        // B @ A = [[1.0, 0.0], [0.0, 0.0]]
        // scale = alpha / rank = 2.0 / 1 = 2.0
        // delta = scale * B @ A = [[2.0, 0.0], [0.0, 0.0]]
        let mut lora = LoRALayer::new("test".into(), 2, 2, 1, 2.0);
        lora.a = vec![1.0, 0.0];
        lora.b = vec![1.0, 0.0];
        let delta = lora.compute_weight_delta();
        assert_eq!(delta, vec![2.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_lora_compute_weight_delta_rank2() {
        // rank=2, in=2, out=2
        // A = [[1, 0], [0, 1]]  (2x2, rank x in)
        // B = [[1, 0], [0, 1]]  (2x2, out x rank)
        // B @ A = [[1, 0], [0, 1]] (identity)
        // scale = 1.0 / 2 = 0.5
        // delta = 0.5 * I = [[0.5, 0], [0, 0.5]]
        let mut lora = LoRALayer::new("test".into(), 2, 2, 2, 1.0);
        lora.a = vec![1.0, 0.0, 0.0, 1.0];
        lora.b = vec![1.0, 0.0, 0.0, 1.0];
        let delta = lora.compute_weight_delta();
        assert_eq!(delta, vec![0.5, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn test_lora_magnitude_after_reset() {
        let mut lora = LoRALayer::new("opcode".into(), 4, 3, 2, 1.0);
        // Set some non-zero B values
        lora.b = vec![0.5; 3 * 2];
        assert!(lora.magnitude() > 0.0);

        lora.reset();
        // After reset, B is all zeros => magnitude should be 0
        assert_eq!(lora.magnitude(), 0.0);
    }
}
