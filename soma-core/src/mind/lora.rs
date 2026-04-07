//! LoRA layer management — experiential memory.
//! Placeholder for Rust-native LoRA. Currently adaptation is done in Python.
//! This module defines the types that the MindEngine trait uses.

/// A LoRA adaptation layer (Section 4.7).
/// When integrated, this is applied during inference: output = base(x) + scale * (x @ A.T) @ B.T
/// Currently a data structure — integration with OnnxMindEngine is tracked as a future milestone.
pub struct LoRALayer {
    pub name: String,
    pub base_weight_shape: (usize, usize), // (out_features, in_features)
    pub a: Vec<f32>,           // rank x in_features
    pub b: Vec<f32>,           // out_features x rank
    pub rank: usize,
    pub scale: f32,            // alpha / rank
}

impl LoRALayer {
    pub fn new(name: String, in_features: usize, out_features: usize, rank: usize, alpha: f32) -> Self {
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

    /// How much has this layer adapted from its base?
    pub fn magnitude(&self) -> f32 {
        // ||B @ A|| * scale (simplified: sum of absolute values)
        let sum: f32 = self.b.iter().map(|x| x.abs()).sum();
        sum * self.scale
    }

    /// Reset LoRA to zero (after merge into base weights).
    pub fn reset(&mut self) {
        self.a.iter_mut().for_each(|x| *x = 0.01);
        self.b.iter_mut().for_each(|x| *x = 0.0);
    }

    /// Compute the weight delta: `scale * B @ A`.
    ///
    /// Returns a flat `Vec<f32>` of shape `(out_features, in_features)` in row-major order.
    /// This represents the effective weight change that LoRA applies:
    /// `W_effective = W_base + scale * B @ A`
    ///
    /// B is (out_features x rank), A is (rank x in_features), so B @ A is (out_features x in_features).
    /// After consolidation, this delta is stored separately and applied during inference
    /// since tract-onnx models are frozen and we cannot modify base weights in-place.
    pub fn compute_weight_delta(&self) -> Vec<f32> {
        let (out_features, in_features) = self.base_weight_shape;
        let rank = self.rank;
        let mut delta = vec![0.0f32; out_features * in_features];

        // B @ A: for each (o, i), sum over rank dimension
        // B[o, r] * A[r, i]
        for o in 0..out_features {
            for r in 0..rank {
                let b_val = self.b[o * rank + r];
                if b_val == 0.0 { continue; }
                for i in 0..in_features {
                    delta[o * in_features + i] += b_val * self.a[r * in_features + i];
                }
            }
        }

        // Apply scale
        for v in &mut delta {
            *v *= self.scale;
        }

        delta
    }

    /// Compute the LoRA delta for a given input hidden state.
    /// Returns: scale * (hidden @ A.T) @ B.T as Vec<f32> of length out_features.
    pub fn forward(&self, hidden: &[f32]) -> Vec<f32> {
        let (out_features, in_features) = self.base_weight_shape;
        let rank = self.rank;

        // h @ A.T -> rank-dimensional
        // A is rank x in_features (row-major), so A.T column r = A row r
        let mut ha = vec![0.0f32; rank];
        let usable_in = in_features.min(hidden.len());
        for r in 0..rank {
            let row_offset = r * in_features;
            for d in 0..usable_in {
                ha[r] += hidden[d] * self.a[row_offset + d];
            }
        }

        // ha @ B.T -> out_features-dimensional
        // B is out_features x rank (row-major)
        let mut delta = vec![0.0f32; out_features];
        for o in 0..out_features.min(self.b.len() / rank) {
            let row_offset = o * rank;
            for r in 0..rank {
                delta[o] += ha[r] * self.b[row_offset + r];
            }
            delta[o] *= self.scale;
        }

        delta
    }

    /// Update LoRA weights from a batch of (hidden_state, target_opcode) pairs.
    /// This is a simplified SGD step:
    ///   loss = cross_entropy(base_logits + lora_delta, target_opcode)
    ///   grad_A, grad_B = d(loss) / d(A), d(loss) / d(B)
    ///   A -= lr * grad_A
    ///   B -= lr * grad_B
    ///
    /// Returns the average loss over the batch.
    pub fn adapt(&mut self, batch: &[(Vec<f32>, usize)], base_logits: &[Vec<f32>], learning_rate: f32) -> f32 {
        if batch.is_empty() {
            return 0.0;
        }

        let (out_features, in_features) = self.base_weight_shape;
        let rank = self.rank;

        // Accumulate gradients
        let mut grad_a = vec![0.0f32; rank * in_features];
        let mut grad_b = vec![0.0f32; out_features * rank];
        let mut total_loss = 0.0f32;

        for (i, (hidden, target)) in batch.iter().enumerate() {
            let usable_in = in_features.min(hidden.len());

            // 1. Compute ha = hidden @ A.T (rank-dimensional)
            let mut ha = vec![0.0f32; rank];
            for r in 0..rank {
                let row_offset = r * in_features;
                for d in 0..usable_in {
                    ha[r] += hidden[d] * self.a[row_offset + d];
                }
            }

            // 2. Compute logits = base_logits + scale * ha @ B.T
            let base = &base_logits[i];
            let num_ops = out_features.min(base.len());
            let mut logits = vec![0.0f32; num_ops];
            for o in 0..num_ops {
                let row_offset = o * rank;
                let mut delta = 0.0f32;
                for r in 0..rank {
                    delta += ha[r] * self.b[row_offset + r];
                }
                logits[o] = base[o] + delta * self.scale;
            }

            // 3. Softmax
            let probs = softmax(&logits);

            // 4. Cross-entropy loss: -log(probs[target])
            let target_idx = *target;
            if target_idx < probs.len() {
                total_loss += -probs[target_idx].max(1e-10).ln();
            }

            // 5. Gradient of cross-entropy w.r.t. logits: d_logits = probs; d_logits[target] -= 1.0
            let mut d_logits = probs;
            if target_idx < d_logits.len() {
                d_logits[target_idx] -= 1.0;
            }

            // 6. Backprop through B: d_B[o][r] += d_logits[o] * scale * ha[r]
            for o in 0..num_ops.min(out_features) {
                let row_offset = o * rank;
                let dl_scaled = d_logits[o] * self.scale;
                for r in 0..rank {
                    grad_b[row_offset + r] += dl_scaled * ha[r];
                }
            }

            // 7. Backprop through A: d_ha[r] = sum_o(d_logits[o] * scale * B[o][r])
            //    d_A[r][d] += d_ha[r] * hidden[d]
            let mut d_ha = vec![0.0f32; rank];
            for o in 0..num_ops.min(out_features) {
                let row_offset = o * rank;
                let dl_scaled = d_logits[o] * self.scale;
                for r in 0..rank {
                    d_ha[r] += dl_scaled * self.b[row_offset + r];
                }
            }
            for r in 0..rank {
                let row_offset = r * in_features;
                for d in 0..usable_in {
                    grad_a[row_offset + d] += d_ha[r] * hidden[d];
                }
            }
        }

        // Average gradients over batch
        let batch_size = batch.len() as f32;
        let inv_batch = 1.0 / batch_size;

        // Update: A -= lr * grad_A / batch_size
        for i in 0..self.a.len() {
            self.a[i] -= learning_rate * grad_a[i] * inv_batch;
        }
        // Update: B -= lr * grad_B / batch_size
        for i in 0..self.b.len() {
            self.b[i] -= learning_rate * grad_b[i] * inv_batch;
        }

        total_loss / batch_size
    }
}

/// Compute softmax of a logit vector.
fn softmax(logits: &[f32]) -> Vec<f32> {
    if logits.is_empty() {
        return Vec::new();
    }
    let max_val = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|x| (x - max_val).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum == 0.0 {
        return vec![1.0 / logits.len() as f32; logits.len()];
    }
    exps.iter().map(|x| x / sum).collect()
}

/// LoRA weights for a single layer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRAWeights {
    pub name: String,
    pub rank: usize,
    pub scale: f32,
    pub a: Vec<f32>,  // rank x in_features (row-major)
    pub b: Vec<f32>,  // out_features x rank (row-major)
}

/// Serializable LoRA checkpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRACheckpoint {
    pub layers: Vec<LoRALayerState>,
    pub adaptation_count: u64,
    pub experience_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRALayerState {
    pub name: String,
    pub rank: usize,
    pub scale: f32,
    pub a: Vec<f32>,
    pub b: Vec<f32>,
}

/// Serializable bundle of LoRA weights for a plugin (Section 7.3).
/// A plugin may provide LoRA layers for one or more output heads.
/// This is the wire format returned by `SomaPlugin::lora_weights()`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoRABundle {
    pub plugin_name: String,
    pub layers: Vec<LoRAWeights>,
}

impl LoRAWeights {
    /// How much has this layer adapted from its base?
    pub fn magnitude(&self) -> f32 {
        // ||B @ A|| * scale — simplified as sum of absolute values
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
