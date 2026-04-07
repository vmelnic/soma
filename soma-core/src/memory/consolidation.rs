//! Memory consolidation — criteria for when LoRA changes should be
//! permanently integrated into the base model weights.

/// Configuration for memory consolidation thresholds.
pub struct ConsolidationConfig {
    /// Minimum LoRA magnitude to consider a layer "significantly adapted".
    pub min_lora_magnitude: f32,
    /// Number of successful adaptations before consolidation is considered.
    pub threshold: u64,
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
    pub fn new(min_lora_magnitude: f32, threshold: u64) -> Self {
        Self {
            min_lora_magnitude,
            threshold,
        }
    }

    /// Check whether consolidation should be attempted given the current
    /// adaptation count and maximum LoRA magnitude across layers.
    pub fn should_consolidate(&self, adaptation_count: u64, max_magnitude: f32) -> bool {
        adaptation_count >= self.threshold && max_magnitude >= self.min_lora_magnitude
    }
}
