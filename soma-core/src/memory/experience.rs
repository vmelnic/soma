//! Experience buffer — stores recent inference outcomes for LoRA adaptation.

/// A single experience record from an inference+execution cycle.
#[derive(Debug, Clone)]
pub struct Experience {
    pub intent_tokens: Vec<u32>,
    /// Full program: (conv_id, arg0_type, arg1_type) per step
    pub program: Vec<(i32, u8, u8)>,
    pub success: bool,
    pub execution_time_ms: u64,
    pub timestamp: std::time::Instant,
    /// Cached hidden states from inference — (hidden_state, base_opcode_logits) per decoder step.
    /// Pre-computed during normal inference so adaptation doesn't need to re-run ONNX.
    /// Empty if not captured (backward compat).
    pub cached_states: Vec<(Vec<f32>, Vec<f32>)>,
}

/// Ring buffer of recent experiences, used to drive LoRA adaptation.
pub struct ExperienceBuffer {
    buffer: Vec<Experience>,
    max_size: usize,
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

    /// Record a new experience. If the buffer is full, the oldest entry is evicted.
    pub fn record(&mut self, experience: Experience) {
        self.total_seen += 1;
        if self.buffer.len() >= self.max_size {
            // Remove oldest
            self.buffer.remove(0);
        }
        self.buffer.push(experience);
    }

    /// Get all successful experiences (for adaptation training data).
    pub fn successes(&self) -> Vec<&Experience> {
        self.buffer.iter().filter(|e| e.success).collect()
    }

    /// Get all failed experiences (for negative examples).
    pub fn failures(&self) -> Vec<&Experience> {
        self.buffer.iter().filter(|e| !e.success).collect()
    }

    /// Number of experiences currently in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Total number of experiences ever recorded (including evicted ones).
    pub fn total_seen(&self) -> u64 {
        self.total_seen
    }

    /// Count of successful experiences in the current buffer.
    pub fn success_count(&self) -> usize {
        self.buffer.iter().filter(|e| e.success).count()
    }

    /// Count of failed experiences in the current buffer.
    pub fn failure_count(&self) -> usize {
        self.buffer.iter().filter(|e| !e.success).count()
    }

    /// Clear all experiences.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Get the most recent N experiences.
    pub fn recent(&self, n: usize) -> &[Experience] {
        let start = if self.buffer.len() > n {
            self.buffer.len() - n
        } else {
            0
        };
        &self.buffer[start..]
    }
}
