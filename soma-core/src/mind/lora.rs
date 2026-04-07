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
}

/// LoRA weights for a single layer.
#[derive(Debug, Clone)]
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

impl LoRAWeights {
    /// How much has this layer adapted from its base?
    pub fn magnitude(&self) -> f32 {
        // ||B @ A|| * scale — simplified as sum of absolute values
        self.b.iter().map(|x| x.abs()).sum::<f32>() * self.scale
    }
}
