//! Experience ring buffer — bounded storage of recent inference outcomes (Spec Section 17).
//!
//! Only successful experiences drive `LoRA` adaptation (Section 17.1: "don't reinforce bad
//! programs"). The buffer uses FIFO eviction: when full, the oldest entry is removed to
//! make room. Cached hidden states allow adaptation without re-running ONNX inference.

/// A single inference-then-execution outcome.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Spec feature: experience tracking fields
pub struct Experience {
    /// Tokenized intent that was fed to the encoder.
    pub intent_tokens: Vec<u32>,
    /// Generated program steps: `(convention_id, arg0_type, arg1_type)` per step.
    pub program: Vec<(i32, u8, u8)>,
    /// Whether the executed program achieved the intent.
    pub success: bool,
    pub execution_time_ms: u64,
    pub timestamp: std::time::Instant,
    /// Pre-computed `(hidden_state, base_opcode_logits)` per decoder step.
    /// Captured during normal inference so `LoRA` adaptation avoids re-running ONNX.
    /// Empty for experiences recorded before this field was added.
    pub cached_states: Vec<(Vec<f32>, Vec<f32>)>,
}

/// Fixed-capacity ring buffer of experiences, driving `LoRA` adaptation.
///
/// Evicts oldest entries on overflow. Tracks lifetime count via `total_seen`
/// independently of current buffer contents.
pub struct ExperienceBuffer {
    buffer: Vec<Experience>,
    max_size: usize,
    /// Monotonically increasing count of all experiences ever recorded, including evicted.
    total_seen: u64,
}

impl ExperienceBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(max_size.min(256)),
            max_size,
            total_seen: 0,
        }
    }

    /// Append an experience, evicting the oldest entry if at capacity.
    ///
    /// Uses `Vec::remove(0)` for eviction — O(n) but acceptable for the small
    /// buffer sizes used in practice (typically 64-256 entries).
    pub fn record(&mut self, experience: Experience) {
        self.total_seen += 1;
        if self.buffer.len() >= self.max_size {
            self.buffer.remove(0);
        }
        self.buffer.push(experience);
    }

    /// Returns only successful experiences (the training signal for `LoRA` adaptation).
    pub fn successes(&self) -> Vec<&Experience> {
        self.buffer.iter().filter(|e| e.success).collect()
    }

    /// Returns only failed experiences.
    #[allow(dead_code)] // Spec feature: experience analysis
    pub fn failures(&self) -> Vec<&Experience> {
        self.buffer.iter().filter(|e| !e.success).collect()
    }

    pub const fn len(&self) -> usize {
        self.buffer.len()
    }

    #[allow(dead_code)] // Spec feature: experience buffer API
    pub const fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Lifetime count of all recorded experiences, including those evicted from the buffer.
    pub const fn total_seen(&self) -> u64 {
        self.total_seen
    }

    pub fn success_count(&self) -> usize {
        self.buffer.iter().filter(|e| e.success).count()
    }

    pub fn failure_count(&self) -> usize {
        self.buffer.iter().filter(|e| !e.success).count()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Returns a slice of the most recent `n` experiences (or fewer if the buffer is smaller).
    #[allow(dead_code)] // Spec feature: experience buffer API
    pub fn recent(&self, n: usize) -> &[Experience] {
        let start = if self.buffer.len() > n {
            self.buffer.len() - n
        } else {
            0
        };
        &self.buffer[start..]
    }
}
