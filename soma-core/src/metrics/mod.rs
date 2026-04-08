//! Prometheus-compatible metrics.
//!
//! 20+ metrics covering inference, plugin execution, protocol, memory, and adaptation.
//! All counters use atomic operations for lock-free concurrent updates.
//! Per-plugin metrics are tracked via `PluginMetrics` in a concurrent `DashMap`.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use dashmap::DashMap;

/// Atomic counters for a single plugin, keyed by name in [`SomaMetrics::per_plugin`].
pub struct PluginMetrics {
    pub calls: AtomicU64,
    pub errors: AtomicU64,
    pub duration_sum_ms: AtomicU64,
}

/// Lock-free runtime metrics exposed as both Prometheus text and JSON.
///
/// All counters use `AtomicU64` with `Relaxed` ordering -- eventual consistency
/// is acceptable for observability data and avoids contention on hot paths.
pub struct SomaMetrics {
    pub inferences_total: AtomicU64,
    pub inferences_success: AtomicU64,
    pub inferences_failed: AtomicU64,
    pub inference_duration_sum_ms: AtomicU64,
    /// Running sum of confidence scores scaled by 1e6 for integer storage.
    /// Divide by `inferences_total` and 1e6 to recover average confidence.
    pub inference_confidence_sum: AtomicU64,

    pub programs_executed: AtomicU64,
    pub program_steps_executed: AtomicU64,

    pub plugin_calls_total: AtomicU64,
    pub plugin_calls_failed: AtomicU64,
    pub plugin_retries: AtomicU64,
    pub plugin_duration_sum_ms: AtomicU64,

    pub experience_buffer_size: AtomicU64,
    pub adaptations_total: AtomicU64,
    pub checkpoints_saved: AtomicU64,
    pub memory_rss_bytes: AtomicU64,

    /// `LoRA` adapter magnitude stored as raw `f32::to_bits` inside a `u64`.
    pub lora_magnitude: AtomicU64,

    pub reflex_hits_total: AtomicU64,
    pub reflex_misses_total: AtomicU64,
    pub reflex_entries: AtomicU64,

    pub protocol_connections_active: AtomicU64,
    pub protocol_signals_sent: AtomicU64,
    pub protocol_signals_received: AtomicU64,
    pub protocol_bytes_transferred: AtomicU64,

    /// Per-plugin counters, lazily populated on first call to each plugin.
    pub per_plugin: DashMap<String, PluginMetrics>,

    pub start_time: Instant,
}

impl SomaMetrics {
    pub fn new() -> Self {
        Self {
            inferences_total: AtomicU64::new(0),
            inferences_success: AtomicU64::new(0),
            inferences_failed: AtomicU64::new(0),
            inference_duration_sum_ms: AtomicU64::new(0),
            inference_confidence_sum: AtomicU64::new(0),
            programs_executed: AtomicU64::new(0),
            program_steps_executed: AtomicU64::new(0),
            plugin_calls_total: AtomicU64::new(0),
            plugin_calls_failed: AtomicU64::new(0),
            plugin_retries: AtomicU64::new(0),
            plugin_duration_sum_ms: AtomicU64::new(0),
            experience_buffer_size: AtomicU64::new(0),
            adaptations_total: AtomicU64::new(0),
            checkpoints_saved: AtomicU64::new(0),
            memory_rss_bytes: AtomicU64::new(0),
            lora_magnitude: AtomicU64::new(0),
            reflex_hits_total: AtomicU64::new(0),
            reflex_misses_total: AtomicU64::new(0),
            reflex_entries: AtomicU64::new(0),
            protocol_connections_active: AtomicU64::new(0),
            protocol_signals_sent: AtomicU64::new(0),
            protocol_signals_received: AtomicU64::new(0),
            protocol_bytes_transferred: AtomicU64::new(0),
            per_plugin: DashMap::new(),
            start_time: Instant::now(),
        }
    }

    /// Record a completed inference attempt (success or failure) and its wall-clock duration.
    pub fn record_inference(&self, success: bool, duration_ms: u64) {
        self.inferences_total.fetch_add(1, Ordering::Relaxed);
        self.inference_duration_sum_ms.fetch_add(duration_ms, Ordering::Relaxed);
        if success {
            self.inferences_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inferences_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a completed program execution and its step count.
    pub fn record_program(&self, steps: u64) {
        self.programs_executed.fetch_add(1, Ordering::Relaxed);
        self.program_steps_executed.fetch_add(steps, Ordering::Relaxed);
    }

    /// Record a plugin call (global counters only, no per-plugin tracking).
    #[allow(dead_code)]
    pub fn record_plugin_call(&self, success: bool) {
        self.plugin_calls_total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.plugin_calls_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a plugin call with per-plugin metric tracking.
    /// Updates both global counters and per-plugin counters.
    pub fn record_plugin_call_named(&self, plugin_name: &str, duration_ms: u64, success: bool) {
        self.plugin_calls_total.fetch_add(1, Ordering::Relaxed);
        self.plugin_duration_sum_ms.fetch_add(duration_ms, Ordering::Relaxed);
        if !success {
            self.plugin_calls_failed.fetch_add(1, Ordering::Relaxed);
        }

        // Per-plugin tracking
        let entry = self.per_plugin.entry(plugin_name.to_string()).or_insert_with(|| PluginMetrics {
            calls: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            duration_sum_ms: AtomicU64::new(0),
        });
        entry.calls.fetch_add(1, Ordering::Relaxed);
        entry.duration_sum_ms.fetch_add(duration_ms, Ordering::Relaxed);
        if !success {
            entry.errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a reflex layer cache hit and update entry gauge.
    pub fn record_reflex_hit(&self, entries: u64) {
        self.reflex_hits_total.fetch_add(1, Ordering::Relaxed);
        self.reflex_entries.store(entries, Ordering::Relaxed);
    }

    /// Record a reflex layer cache miss and update entry gauge.
    pub fn record_reflex_miss(&self, entries: u64) {
        self.reflex_misses_total.fetch_add(1, Ordering::Relaxed);
        self.reflex_entries.store(entries, Ordering::Relaxed);
    }

    /// Record a plugin retry.
    pub fn record_plugin_retry(&self) {
        self.plugin_retries.fetch_add(1, Ordering::Relaxed);
    }

    /// Accumulate an inference confidence score into the running sum.
    /// Stored as `(confidence * 1e6)` to preserve ~6 decimal digits in integer form.
    #[allow(dead_code)]
    pub fn record_confidence(&self, confidence: f32) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (f64::from(confidence) * 1_000_000.0) as u64;
        self.inference_confidence_sum.fetch_add(scaled, Ordering::Relaxed);
    }

    /// Accumulate a plugin call duration (global counter only, no per-plugin tracking).
    #[allow(dead_code)]
    pub fn record_plugin_duration(&self, duration_ms: u64) {
        self.plugin_duration_sum_ms.fetch_add(duration_ms, Ordering::Relaxed);
    }

    /// Set the current `LoRA` adapter magnitude.
    /// Stored as raw f32 bits inside an `AtomicU64`.
    #[allow(dead_code)]
    pub fn set_lora_magnitude(&self, magnitude: f32) {
        self.lora_magnitude.store(u64::from(magnitude.to_bits()), Ordering::Relaxed);
    }

    /// Set the current process RSS in bytes.
    #[allow(dead_code)]
    pub fn set_memory_rss(&self, bytes: u64) {
        self.memory_rss_bytes.store(bytes, Ordering::Relaxed);
    }

    /// Average inference confidence (0.0..1.0).
    #[allow(clippy::cast_precision_loss)] // Acceptable: metrics counters don't need exact precision
    pub fn avg_confidence(&self) -> f64 {
        let total = self.inferences_total.load(Ordering::Relaxed);
        if total == 0 { return 0.0; }
        let sum = self.inference_confidence_sum.load(Ordering::Relaxed) as f64;
        sum / 1_000_000.0 / total as f64
    }

    /// Read back the `LoRA` magnitude as f32.
    pub fn get_lora_magnitude(&self) -> f32 {
        // Only the lower 32 bits are used (stored via f32::to_bits)
        #[allow(clippy::cast_possible_truncation)]
        f32::from_bits(self.lora_magnitude.load(Ordering::Relaxed) as u32)
    }

    /// Average inference duration in ms.
    #[allow(clippy::cast_precision_loss)] // Acceptable: metrics counters don't need exact precision
    pub fn avg_inference_ms(&self) -> f64 {
        let total = self.inferences_total.load(Ordering::Relaxed);
        if total == 0 { return 0.0; }
        self.inference_duration_sum_ms.load(Ordering::Relaxed) as f64 / total as f64
    }

    /// Collect per-plugin counters into a JSON object keyed by plugin name.
    fn per_plugin_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for entry in &self.per_plugin {
            let name = entry.key().clone();
            let pm = entry.value();
            map.insert(name, serde_json::json!({
                "calls": pm.calls.load(Ordering::Relaxed),
                "errors": pm.errors.load(Ordering::Relaxed),
                "duration_sum_ms": pm.duration_sum_ms.load(Ordering::Relaxed),
            }));
        }
        serde_json::Value::Object(map)
    }

    /// Serialize to Prometheus text exposition format.
    pub fn to_prometheus(&self) -> String {
        let mut out = String::with_capacity(2048);

        prom(&mut out, "soma_inferences_total", "counter",
            "Total inference requests", self.inferences_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_inferences_success", "counter",
            "Successful inferences", self.inferences_success.load(Ordering::Relaxed));
        prom(&mut out, "soma_inferences_failed", "counter",
            "Failed inferences", self.inferences_failed.load(Ordering::Relaxed));
        prom(&mut out, "soma_inference_duration_sum_ms", "counter",
            "Sum of inference durations in ms", self.inference_duration_sum_ms.load(Ordering::Relaxed));
        prom_f64(&mut out, "soma_inference_confidence_avg", "gauge",
            "Average inference confidence", self.avg_confidence());
        prom(&mut out, "soma_reflex_hits_total", "counter",
            "Reflex layer cache hits", self.reflex_hits_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_reflex_misses_total", "counter",
            "Reflex layer cache misses", self.reflex_misses_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_reflex_entries", "gauge",
            "Current reflex layer entry count", self.reflex_entries.load(Ordering::Relaxed));
        prom(&mut out, "soma_programs_executed", "counter",
            "Total programs executed", self.programs_executed.load(Ordering::Relaxed));
        prom(&mut out, "soma_program_steps_executed", "counter",
            "Total program steps executed", self.program_steps_executed.load(Ordering::Relaxed));
        prom(&mut out, "soma_plugin_calls_total", "counter",
            "Total plugin convention calls", self.plugin_calls_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_plugin_errors_total", "counter",
            "Failed plugin calls", self.plugin_calls_failed.load(Ordering::Relaxed));
        prom(&mut out, "soma_plugin_retries", "counter",
            "Plugin call retries", self.plugin_retries.load(Ordering::Relaxed));
        prom(&mut out, "soma_plugin_duration_sum_ms", "counter",
            "Sum of plugin call durations in ms", self.plugin_duration_sum_ms.load(Ordering::Relaxed));

        // Per-plugin metrics
        if !self.per_plugin.is_empty() {
            out.push_str("# HELP soma_plugin_calls_total_per Per-plugin total calls\n");
            out.push_str("# TYPE soma_plugin_calls_total_per counter\n");
            for entry in &self.per_plugin {
                let name = entry.key();
                let pm = entry.value();
                let _ = writeln!(out,
                    "soma_plugin_calls_total{{plugin=\"{name}\"}} {}",
                    pm.calls.load(Ordering::Relaxed),
                );
            }
            out.push_str("# HELP soma_plugin_errors_total_per Per-plugin error count\n");
            out.push_str("# TYPE soma_plugin_errors_total_per counter\n");
            for entry in &self.per_plugin {
                let name = entry.key();
                let pm = entry.value();
                let _ = writeln!(out,
                    "soma_plugin_errors_total{{plugin=\"{name}\"}} {}",
                    pm.errors.load(Ordering::Relaxed),
                );
            }
            out.push_str("# HELP soma_plugin_duration_sum_ms_per Per-plugin duration sum in ms\n");
            out.push_str("# TYPE soma_plugin_duration_sum_ms_per counter\n");
            for entry in &self.per_plugin {
                let name = entry.key();
                let pm = entry.value();
                let _ = writeln!(out,
                    "soma_plugin_duration_sum_ms{{plugin=\"{name}\"}} {}",
                    pm.duration_sum_ms.load(Ordering::Relaxed),
                );
            }
        }
        prom(&mut out, "soma_experience_buffer_size", "gauge",
            "Current experience buffer size", self.experience_buffer_size.load(Ordering::Relaxed));
        prom(&mut out, "soma_adaptations_total", "counter",
            "Total LoRA adaptations", self.adaptations_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_checkpoints_saved", "counter",
            "Total checkpoints saved", self.checkpoints_saved.load(Ordering::Relaxed));
        prom(&mut out, "soma_memory_rss_bytes", "gauge",
            "Current resident set size in bytes", self.memory_rss_bytes.load(Ordering::Relaxed));
        prom_f64(&mut out, "soma_lora_magnitude", "gauge",
            "Current LoRA adapter magnitude", f64::from(self.get_lora_magnitude()));
        prom(&mut out, "soma_protocol_connections_active", "gauge",
            "Active protocol connections", self.protocol_connections_active.load(Ordering::Relaxed));
        prom(&mut out, "soma_protocol_signals_sent", "counter",
            "Total signals sent", self.protocol_signals_sent.load(Ordering::Relaxed));
        prom(&mut out, "soma_protocol_signals_received", "counter",
            "Total signals received", self.protocol_signals_received.load(Ordering::Relaxed));
        prom(&mut out, "soma_protocol_bytes_transferred", "counter",
            "Total bytes transferred", self.protocol_bytes_transferred.load(Ordering::Relaxed));

        // Uptime gauge
        prom_f64(&mut out, "soma_uptime_seconds", "gauge",
            "Seconds since SOMA instance started", self.start_time.elapsed().as_secs_f64());

        out
    }

    /// Set the active connection count gauge.
    #[allow(dead_code)]
    pub fn set_active_connections(&self, count: u64) {
        self.protocol_connections_active.store(count, Ordering::Relaxed);
    }

    /// Record an outbound protocol signal and its byte size.
    pub fn record_signal_sent(&self, bytes: u64) {
        self.protocol_signals_sent.fetch_add(1, Ordering::Relaxed);
        self.protocol_bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record an inbound protocol signal and its byte size.
    pub fn record_signal_received(&self, bytes: u64) {
        self.protocol_signals_received.fetch_add(1, Ordering::Relaxed);
        self.protocol_bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Serialize to JSON for MCP health/metrics endpoint.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "inference": {
                "total": self.inferences_total.load(Ordering::Relaxed),
                "success": self.inferences_success.load(Ordering::Relaxed),
                "failed": self.inferences_failed.load(Ordering::Relaxed),
                "avg_duration_ms": self.avg_inference_ms(),
                "avg_confidence": self.avg_confidence(),
            },
            "programs": {
                "executed": self.programs_executed.load(Ordering::Relaxed),
                "steps_executed": self.program_steps_executed.load(Ordering::Relaxed),
            },
            "plugins": {
                "calls_total": self.plugin_calls_total.load(Ordering::Relaxed),
                "calls_failed": self.plugin_calls_failed.load(Ordering::Relaxed),
                "retries": self.plugin_retries.load(Ordering::Relaxed),
                "duration_sum_ms": self.plugin_duration_sum_ms.load(Ordering::Relaxed),
            },
            "memory": {
                "experience_buffer_size": self.experience_buffer_size.load(Ordering::Relaxed),
                "adaptations_total": self.adaptations_total.load(Ordering::Relaxed),
                "checkpoints_saved": self.checkpoints_saved.load(Ordering::Relaxed),
                "rss_bytes": self.memory_rss_bytes.load(Ordering::Relaxed),
            },
            "reflex": {
                "hits_total": self.reflex_hits_total.load(Ordering::Relaxed),
                "misses_total": self.reflex_misses_total.load(Ordering::Relaxed),
                "entries": self.reflex_entries.load(Ordering::Relaxed),
            },
            "adaptation": {
                "lora_magnitude": self.get_lora_magnitude(),
            },
            "protocol": {
                "connections_active": self.protocol_connections_active.load(Ordering::Relaxed),
                "signals_sent": self.protocol_signals_sent.load(Ordering::Relaxed),
                "signals_received": self.protocol_signals_received.load(Ordering::Relaxed),
                "bytes_transferred": self.protocol_bytes_transferred.load(Ordering::Relaxed),
            },
            "uptime_seconds": self.start_time.elapsed().as_secs_f64(),
            "per_plugin": self.per_plugin_json(),
        })
    }
}

/// Write a single Prometheus metric (HELP + TYPE + value) for a u64 counter/gauge.
fn prom(out: &mut String, name: &str, ptype: &str, help: &str, value: u64) {
    let _ = write!(out, "# HELP {name} {help}\n# TYPE {name} {ptype}\n{name} {value}\n");
}

/// Write a single Prometheus metric (HELP + TYPE + value) for an f64 gauge.
fn prom_f64(out: &mut String, name: &str, ptype: &str, help: &str, value: f64) {
    let _ = write!(out, "# HELP {name} {help}\n# TYPE {name} {ptype}\n{name} {value}\n");
}
