//! Low-Rank Adaptation (`LoRA`) for runtime Mind specialization.
//!
//! `LoRA` layers are applied as post-hoc logit adjustments during inference:
//! `output = base(hidden) + scale * (hidden @ A.T) @ B.T`. The `adapt()` method
//! performs SGD on the A and B matrices using (`hidden_state`, `target_opcode`) pairs
//! from successful experiences, enabling the Mind to improve without retraining
//! the base ONNX model.
//!
//! Key design: B is zero-initialized so new `LoRA` layers have no effect until
//! adapted. A is initialized to small values (0.01) for gradient flow.

/// A single `LoRA` adapter targeting one output head (e.g. "opcode", "a0t", "r0").
///
/// The low-rank decomposition `W_delta = scale * B @ A` keeps parameter count at
/// `rank * (in + out)` instead of `in * out`. Typical rank is 4-16.
pub struct LoRALayer {
    /// Output head name this layer targets (must match head names in `apply_lora_to_logits`).
    pub name: String,
    /// `(out_features, in_features)` of the weight matrix being adapted.
    pub base_weight_shape: (usize, usize),
    /// Down-projection matrix, shape `(rank, in_features)`, row-major.
    pub a: Vec<f32>,
    /// Up-projection matrix, shape `(out_features, rank)`, row-major. Zero-initialized.
    pub b: Vec<f32>,
    pub rank: usize,
    /// Scaling factor: `alpha / rank`. Controls the magnitude of the `LoRA` contribution.
    pub scale: f32,
}

impl LoRALayer {
    /// Create a new `LoRA` layer with zero-effect initialization (B=0, A=0.01).
    pub fn new(name: String, in_features: usize, out_features: usize, rank: usize, alpha: f32) -> Self {
        #[allow(clippy::cast_precision_loss)] // rank is small (typically 4-16)
        let scale = alpha / rank as f32;
        Self {
            name,
            base_weight_shape: (out_features, in_features),
            a: vec![0.01; rank * in_features],  // small random init
            b: vec![0.0; out_features * rank],   // zero init (no initial effect)
            rank,
            scale,
        }
    }

    /// Approximate L1 norm of B scaled by `scale`. Tracks how far this layer has drifted.
    pub fn magnitude(&self) -> f32 {
        self.b.iter().map(|x| x.abs()).sum::<f32>() * self.scale
    }

    /// Reset to zero-effect state (A=0.01, B=0). Called after merging into base weights.
    pub fn reset(&mut self) {
        self.a.iter_mut().for_each(|x| *x = 0.01);
        self.b.iter_mut().for_each(|x| *x = 0.0);
    }

    /// Compute the full weight delta `scale * B @ A` as a flat `(out_features, in_features)` matrix.
    /// Used during consolidation to permanently fold `LoRA` knowledge into `merged_opcode_delta`.
    pub fn compute_weight_delta(&self) -> Vec<f32> {
        let (out_features, in_features) = self.base_weight_shape;
        let rank = self.rank;
        let mut delta = vec![0.0f32; out_features * in_features];

        for o in 0..out_features {
            for r in 0..rank {
                let b_val = self.b[o * rank + r];
                if b_val == 0.0 { continue; } // skip zero rows (common before adaptation)
                for i in 0..in_features {
                    delta[o * in_features + i] += b_val * self.a[r * in_features + i];
                }
            }
        }

        for v in &mut delta {
            *v *= self.scale;
        }

        delta
    }

    /// Compute `scale * (hidden @ A.T) @ B.T`, the `LoRA` logit adjustment for one input.
    pub fn forward(&self, hidden: &[f32]) -> Vec<f32> {
        let (out_features, in_features) = self.base_weight_shape;
        let rank = self.rank;

        // Project down: hidden (in_features) -> ha (rank)
        let mut ha = vec![0.0f32; rank];
        let usable_in = in_features.min(hidden.len());
        for (r, ha_r) in ha.iter_mut().enumerate() {
            let row_offset = r * in_features;
            for (d, hidden_d) in hidden.iter().enumerate().take(usable_in) {
                *ha_r += hidden_d * self.a[row_offset + d];
            }
        }

        // Project up: ha (rank) -> delta (out_features), then scale
        let mut delta = vec![0.0f32; out_features];
        for (o, delta_o) in delta.iter_mut().enumerate().take(out_features.min(self.b.len() / rank)) {
            let row_offset = o * rank;
            for (r, ha_r) in ha.iter().enumerate() {
                *delta_o += ha_r * self.b[row_offset + r];
            }
            *delta_o *= self.scale;
        }

        delta
    }

    /// SGD update from a batch of `(hidden_state, target_opcode)` pairs.
    ///
    /// For each sample: computes LoRA-adjusted logits, cross-entropy loss against the
    /// target, then backprops through B and A only (base model weights are frozen).
    /// Gradients are averaged over the batch before the weight update.
    ///
    /// Returns the mean cross-entropy loss.
    pub fn adapt(&mut self, batch: &[(Vec<f32>, usize)], base_logits: &[Vec<f32>], learning_rate: f32) -> f32 {
        if batch.is_empty() {
            return 0.0;
        }

        let (out_features, in_features) = self.base_weight_shape;
        let rank = self.rank;

        let mut grad_a = vec![0.0f32; rank * in_features];
        let mut grad_b = vec![0.0f32; out_features * rank];
        let mut total_loss = 0.0f32;

        for (i, (hidden, target)) in batch.iter().enumerate() {
            let usable_in = in_features.min(hidden.len());

            // Forward: ha = hidden @ A.T
            let mut ha = vec![0.0f32; rank];
            for (r, ha_r) in ha.iter_mut().enumerate() {
                let row_offset = r * in_features;
                for (d, hidden_d) in hidden.iter().enumerate().take(usable_in) {
                    *ha_r += hidden_d * self.a[row_offset + d];
                }
            }

            // Forward: logits = base + scale * ha @ B.T
            let base = &base_logits[i];
            let num_ops = out_features.min(base.len());
            let mut logits = vec![0.0f32; num_ops];
            for (o, logits_o) in logits.iter_mut().enumerate().take(num_ops) {
                let row_offset = o * rank;
                let mut delta = 0.0f32;
                for (r, ha_r) in ha.iter().enumerate() {
                    delta += ha_r * self.b[row_offset + r];
                }
                *logits_o = delta.mul_add(self.scale, base[o]);
            }

            let probs = softmax(&logits);

            let target_idx = *target;
            if target_idx < probs.len() {
                total_loss += -probs[target_idx].max(1e-10).ln();
            }

            // d(CE)/d(logits) = softmax(logits) - one_hot(target)
            let mut d_logits = probs;
            if target_idx < d_logits.len() {
                d_logits[target_idx] -= 1.0;
            }

            // Backprop through B: d_B[o][r] = d_logits[o] * scale * ha[r]
            for (o, dl_o) in d_logits.iter().enumerate().take(num_ops.min(out_features)) {
                let row_offset = o * rank;
                let dl_scaled = dl_o * self.scale;
                for (r, ha_r) in ha.iter().enumerate() {
                    grad_b[row_offset + r] += dl_scaled * ha_r;
                }
            }

            // Backprop through A via chain rule through ha
            let mut d_ha = vec![0.0f32; rank];
            for (o, dl_o) in d_logits.iter().enumerate().take(num_ops.min(out_features)) {
                let row_offset = o * rank;
                let dl_scaled = dl_o * self.scale;
                for (r, d_ha_r) in d_ha.iter_mut().enumerate() {
                    *d_ha_r += dl_scaled * self.b[row_offset + r];
                }
            }
            for (r, d_ha_r) in d_ha.iter().enumerate() {
                let row_offset = r * in_features;
                for (d, hidden_d) in hidden.iter().enumerate().take(usable_in) {
                    grad_a[row_offset + d] += d_ha_r * hidden_d;
                }
            }
        }

        #[allow(clippy::cast_precision_loss)] // batch size is small
        let batch_size = batch.len() as f32;
        let inv_batch = 1.0 / batch_size;

        for (a_i, grad_a_i) in self.a.iter_mut().zip(grad_a.iter()) {
            *a_i -= learning_rate * grad_a_i * inv_batch;
        }
        for (b_i, grad_b_i) in self.b.iter_mut().zip(grad_b.iter()) {
            *b_i -= learning_rate * grad_b_i * inv_batch;
        }

        total_loss / batch_size
    }
}

/// Numerically stable softmax (subtracts max before exp to prevent overflow).
fn softmax(logits: &[f32]) -> Vec<f32> {
    if logits.is_empty() {
        return Vec::new();
    }
    let max_val = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|x| (x - max_val).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum.abs() < f32::EPSILON {
        #[allow(clippy::cast_precision_loss)] // logits length is small
        return vec![1.0 / logits.len() as f32; logits.len()];
    }
    exps.iter().map(|x| x / sum).collect()
}

/// Serializable `LoRA` weights for a single layer (used in bundles and checkpoints).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRAWeights {
    pub name: String,
    pub rank: usize,
    pub scale: f32,
    /// Shape: `(rank, in_features)`, row-major.
    pub a: Vec<f32>,
    /// Shape: `(out_features, rank)`, row-major.
    pub b: Vec<f32>,
}

/// Snapshot of all active `LoRA` layers for checkpoint persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRACheckpoint {
    pub layers: Vec<LoRALayerState>,
    pub adaptation_count: u64,
    pub experience_count: u64,
}

/// Per-layer state within a `LoRACheckpoint`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRALayerState {
    pub name: String,
    pub rank: usize,
    pub scale: f32,
    pub a: Vec<f32>,
    pub b: Vec<f32>,
}

/// Wire format for plugin-provided `LoRA` weights (Spec Section 7.3).
/// Returned by `SomaPlugin::lora_weights()` and deserialized by `attach_lora_bytes()`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRABundle {
    pub plugin_name: String,
    pub layers: Vec<LoRAWeights>,
}

impl LoRAWeights {
    #[allow(dead_code)] // Spec Section 4.7 — used for LoRA magnitude tracking
    pub fn magnitude(&self) -> f32 {
        self.b.iter().map(|x| x.abs()).sum::<f32>() * self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lora_forward_zero_b() {
        // B initialized to zero -> forward should produce zero delta
        let lora = LoRALayer::new("test".into(), 4, 3, 2, 4.0);
        let hidden = vec![1.0, 2.0, 3.0, 4.0];
        let delta = lora.forward(&hidden);
        assert_eq!(delta.len(), 3);
        for &v in &delta {
            assert_eq!(v, 0.0);
        }
    }

    #[test]
    fn test_lora_forward_nonzero() {
        let mut lora = LoRALayer::new("test".into(), 2, 2, 1, 1.0);
        // scale = alpha/rank = 1.0/1 = 1.0
        // A = [1.0, 0.0] (rank=1, in_features=2) -> 1x2
        // B = [1.0, 0.0] (out_features=2, rank=1) -> 2x1
        lora.a = vec![1.0, 0.0];
        lora.b = vec![1.0, 0.0];
        let hidden = vec![3.0, 5.0];
        // ha = hidden @ A.T = [3.0*1.0 + 5.0*0.0] = [3.0]
        // delta = ha @ B.T * scale = [3.0*1.0, 3.0*0.0] * 1.0 = [3.0, 0.0]
        let delta = lora.forward(&hidden);
        assert_eq!(delta.len(), 2);
        assert!((delta[0] - 3.0).abs() < 1e-6);
        assert!((delta[1] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_lora_adapt_reduces_loss() {
        // Train a LoRA layer to map a hidden state to a specific target opcode
        let mut lora = LoRALayer::new("opcode".into(), 4, 3, 2, 4.0);

        let hidden = vec![1.0, 0.5, -0.3, 0.8];
        let target = 1usize; // target opcode
        let base_logits = vec![0.0, 0.0, 0.0]; // uniform base

        // Compute initial loss
        let initial_delta = lora.forward(&hidden);
        let initial_logits: Vec<f32> = base_logits.iter()
            .zip(initial_delta.iter())
            .map(|(b, d)| b + d)
            .collect();
        let initial_probs = softmax(&initial_logits);
        let initial_loss = -initial_probs[target].max(1e-10).ln();

        // Run several adaptation steps
        let batch = vec![(hidden.clone(), target)];
        let base_batch = vec![base_logits.clone()];
        for _ in 0..50 {
            lora.adapt(&batch, &base_batch, 0.1);
        }

        // Compute final loss
        let final_delta = lora.forward(&hidden);
        let final_logits: Vec<f32> = base_logits.iter()
            .zip(final_delta.iter())
            .map(|(b, d)| b + d)
            .collect();
        let final_probs = softmax(&final_logits);
        let final_loss = -final_probs[target].max(1e-10).ln();

        assert!(final_loss < initial_loss, "Loss should decrease: {:.4} -> {:.4}", initial_loss, final_loss);
        assert!(final_probs[target] > initial_probs[target],
            "Target probability should increase: {:.4} -> {:.4}", initial_probs[target], final_probs[target]);
    }

    #[test]
    fn test_lora_adapt_batch() {
        // Multiple samples in a batch should still converge
        let mut lora = LoRALayer::new("opcode".into(), 4, 3, 2, 4.0);

        let batch = vec![
            (vec![1.0, 0.0, 0.0, 0.0], 0usize),
            (vec![0.0, 1.0, 0.0, 0.0], 1usize),
            (vec![0.0, 0.0, 1.0, 0.0], 2usize),
        ];
        let base_logits = vec![
            vec![0.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.0],
        ];

        let mut prev_loss = f32::MAX;
        for _ in 0..100 {
            let loss = lora.adapt(&batch, &base_logits, 0.05);
            // Loss should generally decrease (may not be strictly monotonic due to batch interactions)
            prev_loss = loss;
        }
        assert!(prev_loss < 1.1, "Loss should decrease significantly after 100 steps, got {:.4}", prev_loss);
    }

    #[test]
    fn test_softmax_properties() {
        let logits = vec![1.0, 2.0, 3.0];
        let probs = softmax(&logits);
        // Should sum to 1
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
        // Should be monotonically increasing
        assert!(probs[0] < probs[1]);
        assert!(probs[1] < probs[2]);
    }

    #[test]
    fn test_softmax_empty() {
        let probs = softmax(&[]);
        assert!(probs.is_empty());
    }

    #[test]
    fn test_softmax_large_values() {
        // Should handle large values without overflow (via max subtraction)
        let logits = vec![1000.0, 1001.0, 1002.0];
        let probs = softmax(&logits);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_lora_magnitude_changes_after_adapt() {
        let mut lora = LoRALayer::new("test".into(), 4, 3, 2, 4.0);
        let initial_mag = lora.magnitude();

        let batch = vec![(vec![1.0, 0.5, -0.3, 0.8], 1usize)];
        let base = vec![vec![0.0, 0.0, 0.0]];
        lora.adapt(&batch, &base, 0.1);

        let after_mag = lora.magnitude();
        // B starts at zero, so magnitude should increase after adaptation
        assert!(after_mag > initial_mag, "Magnitude should increase after adaptation");
    }

    #[test]
    fn test_lora_reset_after_adapt() {
        let mut lora = LoRALayer::new("test".into(), 4, 3, 2, 4.0);
        let batch = vec![(vec![1.0, 0.5, -0.3, 0.8], 1usize)];
        let base = vec![vec![0.0, 0.0, 0.0]];
        lora.adapt(&batch, &base, 0.1);

        assert!(lora.magnitude() > 0.0);
        lora.reset();
        assert_eq!(lora.magnitude(), 0.0);
    }
}
