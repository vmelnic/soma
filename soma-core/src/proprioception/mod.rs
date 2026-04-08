//! Proprioception — self-knowledge and runtime statistics.
//!
//! The SOMA's self-model: what it knows about itself, its capabilities,
//! its current state, and its recent performance.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The SOMA instance's self-model: cumulative statistics, loaded capabilities,
/// and resource usage. Queried by the MCP health endpoint and the REPL status command.
///
/// Unlike [`SomaMetrics`](crate::metrics::SomaMetrics) (lock-free atomics for
/// Prometheus scraping), this struct uses plain `u64`/`f32` fields and requires
/// `&mut self` for updates -- it is owned by the main orchestration loop.
pub struct Proprioception {
    pub start_time: Instant,
    /// Unix timestamp (seconds) captured at startup, for serialization to JSON.
    pub start_timestamp: u64,
    pub total_inferences: u64,
    pub successful_inferences: u64,
    pub failed_inferences: u64,
    pub total_adaptations: u64,
    /// Total experience records (successes + failures) seen during this session.
    pub experience_count: u64,
    pub checkpoints_saved: u64,
    pub consolidations: u64,
    #[allow(dead_code)]
    pub active_connections: u64,
    #[allow(dead_code)]
    pub total_signals_processed: u64,
    pub total_decisions_recorded: u64,
    pub loaded_plugins: Vec<String>,
    pub lora_magnitude: f32,
    /// CPU tracking requires platform-specific implementation; currently always 0.0.
    pub cpu_usage_percent: f32,
}

impl Proprioception {
    pub fn new() -> Self {
        let start_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            start_time: Instant::now(),
            start_timestamp,
            total_inferences: 0,
            successful_inferences: 0,
            failed_inferences: 0,
            total_adaptations: 0,
            experience_count: 0,
            checkpoints_saved: 0,
            consolidations: 0,
            active_connections: 0,
            total_signals_processed: 0,
            total_decisions_recorded: 0,
            loaded_plugins: Vec::new(),
            lora_magnitude: 0.0,
            cpu_usage_percent: 0.0,
        }
    }

    /// Record a successful inference + execution cycle (increments experience count).
    pub const fn record_success(&mut self) {
        self.total_inferences += 1;
        self.successful_inferences += 1;
        self.experience_count += 1;
    }

    /// Record a failed inference or execution cycle (increments experience count).
    pub const fn record_failure(&mut self) {
        self.total_inferences += 1;
        self.failed_inferences += 1;
        self.experience_count += 1;
    }

    /// Record that a `LoRA` adaptation pass was performed.
    pub const fn record_adaptation(&mut self) {
        self.total_adaptations += 1;
    }

    /// Record that a checkpoint was saved to disk.
    pub const fn record_checkpoint(&mut self) {
        self.checkpoints_saved += 1;
    }

    /// Record that a `LoRA` consolidation (merge into base weights) occurred.
    #[allow(dead_code)]
    pub const fn record_consolidation(&mut self) {
        self.consolidations += 1;
    }

    /// Record that a decision was written to the decision log.
    pub const fn record_decision(&mut self) {
        self.total_decisions_recorded += 1;
    }

    /// How long this SOMA instance has been running.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Format uptime as a human-readable string.
    fn format_uptime(&self) -> String {
        let d = self.uptime();
        let secs = d.as_secs();
        if secs < 60 {
            format!("{secs}s")
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
        }
    }

    /// Success rate as a percentage (0.0 - 100.0).
    #[allow(clippy::cast_precision_loss)] // Acceptable: precision loss negligible for percentages
    pub fn success_rate(&self) -> f64 {
        if self.total_inferences == 0 {
            return 0.0;
        }
        (self.successful_inferences as f64 / self.total_inferences as f64) * 100.0
    }

    /// Generate a human-readable status report.
    pub fn report(&self) -> String {
        let plugins_str = if self.loaded_plugins.is_empty() {
            "none".to_string()
        } else {
            self.loaded_plugins.join(", ")
        };
        format!(
            "Uptime: {}\n\
             Inferences: {} total ({} ok, {} err, {:.1}% success)\n\
             Adaptations: {}\n\
             Experiences: {}\n\
             Checkpoints: {}\n\
             Decisions: {}\n\
             Plugins: {}\n\
             LoRA magnitude: {:.6}\n\
             CPU usage: {:.1}%",
            self.format_uptime(),
            self.total_inferences,
            self.successful_inferences,
            self.failed_inferences,
            self.success_rate(),
            self.total_adaptations,
            self.experience_count,
            self.checkpoints_saved,
            self.total_decisions_recorded,
            plugins_str,
            self.lora_magnitude,
            self.cpu_usage_percent,
        )
    }

    /// Return peak (high-water-mark) RSS in bytes via `getrusage(2)`.
    ///
    /// This is **peak** RSS, not current. Current RSS would require
    /// `mach_task_basic_info` (macOS) or `/proc/self/statm` (Linux).
    pub fn peak_rss_bytes() -> u64 {
        // SAFETY: zeroed rusage is valid, and RUSAGE_SELF is always a valid argument.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) };
        if rc == 0 {
            // macOS: ru_maxrss is in bytes. Linux: in KB.
            // ru_maxrss is always non-negative, sign loss is safe
            #[cfg(target_os = "macos")]
            { usage.ru_maxrss.cast_unsigned() }
            #[cfg(not(target_os = "macos"))]
            { usage.ru_maxrss.cast_unsigned() * 1024 }
        } else {
            0
        }
    }

    /// Set the list of currently loaded plugin names.
    pub fn set_plugins(&mut self, names: Vec<String>) {
        self.loaded_plugins = names;
    }

    /// Set the current `LoRA` adapter magnitude.
    #[allow(dead_code)]
    pub const fn set_lora_magnitude(&mut self, mag: f32) {
        self.lora_magnitude = mag;
    }

    /// Refresh CPU usage estimate. Currently a no-op (returns 0.0) pending
    /// platform-specific implementation (`proc_pidinfo` on macOS, `/proc/self/stat` on Linux).
    #[allow(dead_code)]
    pub const fn update_cpu(&mut self) {
        self.cpu_usage_percent = 0.0;
    }

    /// Serialize for MCP health endpoint.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "uptime_secs": self.uptime().as_secs(),
            "start_timestamp": self.start_timestamp,
            "inferences": {
                "total": self.total_inferences,
                "successful": self.successful_inferences,
                "failed": self.failed_inferences,
                "success_rate": self.success_rate(),
            },
            "adaptations": self.total_adaptations,
            "experience_count": self.experience_count,
            "checkpoints_saved": self.checkpoints_saved,
            "consolidations": self.consolidations,
            "decisions_recorded": self.total_decisions_recorded,
            "memory_peak_rss_bytes": Self::peak_rss_bytes(),
            "loaded_plugins": self.loaded_plugins,
            "lora_magnitude": self.lora_magnitude,
            "cpu_usage_percent": self.cpu_usage_percent,
        })
    }
}
