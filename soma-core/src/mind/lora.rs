//! LoRA layer management — experiential memory.
//! Placeholder for Rust-native LoRA. Currently adaptation is done in Python.
//! This module defines the types that the MindEngine trait uses.

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
