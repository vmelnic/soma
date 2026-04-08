//! Memory consolidation — criteria for when `LoRA` changes should be
//! permanently integrated into the base model weights (Spec Section 6.3).
//!
//! The 5-step consolidation process:
//!   1. Evaluate `LoRA` magnitude per layer
//!   2. Merge high-magnitude layers: `merged_delta += scale * B @ A`
//!   3. Reset merged `LoRA` layers to initial values
//!   4. Save updated base weights checkpoint (handled by caller)
//!   5. Log consolidation event for auditability
//!
//! Because tract-onnx models are frozen (compiled into the graph), we cannot
//! modify `W_base` in-place. Instead, consolidation computes the weight delta
//! `scale * B @ A` for each eligible `LoRA` layer and accumulates it into
//! `OnnxMindEngine::merged_opcode_delta`. During inference, this delta is
//! applied as: `logits += hidden @ merged_opcode_delta.T`, equivalent to
//! having modified the base weights.

use crate::mind::onnx_engine::OnnxMindEngine;

/// Configuration for memory consolidation thresholds.
pub struct ConsolidationConfig {
    /// Minimum `LoRA` magnitude to consider a layer "significantly adapted".
    pub min_lora_magnitude: f32,
    /// Number of successful adaptations before consolidation is considered.
    pub threshold: u64,
}

/// Result of a consolidation attempt.
pub struct ConsolidationResult {
    /// Number of `LoRA` layers evaluated for merge eligibility.
    pub layers_evaluated: usize,
    /// Number of layers whose magnitude exceeded the threshold and were merged.
    pub layers_merged: usize,
    /// Maximum `LoRA` magnitude observed across all layers after consolidation.
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

    /// Check whether consolidation should be attempted given the current
    /// adaptation count and maximum `LoRA` magnitude across layers.
    pub fn should_consolidate(&self, adaptation_count: u64, max_magnitude: f32) -> bool {
        adaptation_count >= self.threshold && max_magnitude >= self.min_lora_magnitude
    }

    /// Perform consolidation on an `OnnxMindEngine` (Section 6.3).
    ///
    /// The 5-step process:
    ///   1. Evaluate `LoRA` magnitude per layer
    ///   2. Merge high-magnitude layers: `merged_delta += scale * B @ A`
    ///   3. Reset merged `LoRA` layers to initial values (A=small random, B=zero)
    ///   4. Save updated checkpoint (handled externally by the caller)
    ///   5. Log consolidation event
    ///
    /// Returns a [`ConsolidationResult`] describing what was merged.
    pub fn consolidate(&self, engine: &mut OnnxMindEngine) -> ConsolidationResult {
        let dd = engine.decoder_dim();
        let num_ops = engine.num_conventions();
        let expected_size = num_ops * dd;

        // Step 1: Evaluate LoRA magnitude per layer
        let lora_layers = engine.active_lora_mut();
        let layers_evaluated = lora_layers.len();
        let mut layers_merged = 0usize;

        // Ensure merged_opcode_delta is initialized
        if engine.merged_opcode_delta.is_empty() && layers_evaluated > 0 {
            engine.merged_opcode_delta = vec![0.0f32; expected_size];
        }

        // Collect indices and deltas for layers that should be merged.
        // We iterate through the layers, compute deltas for eligible ones,
        // then apply them all.
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
                // Step 2: Compute weight delta for this layer
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

        // Apply all deltas to merged_opcode_delta and reset merged layers
        for (idx, delta) in &merge_actions {
            // Accumulate into merged_opcode_delta
            let merge_len = delta.len().min(engine.merged_opcode_delta.len());
            for (target, source) in engine.merged_opcode_delta[..merge_len].iter_mut().zip(&delta[..merge_len]) {
                *target += source;
            }

            // Step 3: Reset this LoRA layer (A to small random, B to zero)
            engine.active_lora_mut()[*idx].reset();
            layers_merged += 1;
        }

        // Compute remaining magnitude after consolidation
        let new_magnitude: f32 = engine.active_lora().iter()
            .map(super::super::mind::lora::LoRALayer::magnitude)
            .fold(0.0f32, f32::max);

        // Step 5: Log consolidation event
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
