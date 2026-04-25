//! Port health monitoring — analyzes per-port latency samples from RuntimeMetrics
//! and emits world state facts when ports degrade, spike, or recover.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
#[cfg(feature = "native")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "native")]
use std::time::Duration;

use crate::runtime::metrics::RuntimeMetrics;
use crate::runtime::world_state::WorldStateStore;

/// Per-port health baseline maintained across ticks.
#[derive(Debug, Clone)]
pub struct PortBaseline {
    /// EMA of latency (ms), updated each tick from the latest samples.
    pub ema_latency: f64,
    /// EMA of latency variance, for detecting abnormal spread.
    pub ema_variance: f64,
    /// Total samples seen so far (used for warm-up gating).
    pub samples_seen: u64,
    /// Current health status.
    pub status: PortHealthStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortHealthStatus {
    Healthy,
    Degraded,
    Unresponsive,
}

impl std::fmt::Display for PortHealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Unresponsive => write!(f, "unresponsive"),
        }
    }
}

const EMA_ALPHA: f64 = 0.1;
const SPIKE_THRESHOLD: f64 = 3.0;
const MIN_SAMPLES_FOR_BASELINE: u64 = 10;

impl PortBaseline {
    fn new() -> Self {
        Self {
            ema_latency: 0.0,
            ema_variance: 0.0,
            samples_seen: 0,
            status: PortHealthStatus::Healthy,
        }
    }

    fn is_calibrated(&self) -> bool {
        self.samples_seen >= MIN_SAMPLES_FOR_BASELINE
    }
}

/// Analyze latency samples for a port and return the updated baseline + new status.
///
/// Returns `None` if the port has no new samples since the last tick.
pub fn analyze_port(
    baseline: &mut PortBaseline,
    samples: &[u64],
    prev_sample_count: u64,
) -> Option<PortHealthStatus> {
    let total = samples.len() as u64;
    if total == 0 {
        return None;
    }

    let new_count = total.saturating_sub(prev_sample_count);

    // Update EMA from all current samples (idempotent baseline convergence).
    let mean = samples.iter().sum::<u64>() as f64 / samples.len() as f64;
    let variance = if samples.len() > 1 {
        samples
            .iter()
            .map(|&s| {
                let d = s as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / (samples.len() - 1) as f64
    } else {
        0.0
    };

    if baseline.samples_seen == 0 {
        baseline.ema_latency = mean;
        baseline.ema_variance = variance;
    } else {
        baseline.ema_latency =
            EMA_ALPHA * mean + (1.0 - EMA_ALPHA) * baseline.ema_latency;
        baseline.ema_variance =
            EMA_ALPHA * variance + (1.0 - EMA_ALPHA) * baseline.ema_variance;
    }
    baseline.samples_seen = total;

    if !baseline.is_calibrated() {
        return Some(baseline.status);
    }

    // No new samples since last tick — port may be unresponsive.
    if new_count == 0 {
        return None;
    }

    // Compute recent window stats (last N new samples).
    let window_start = samples.len().saturating_sub(new_count as usize);
    let window = &samples[window_start..];
    let window_mean = window.iter().sum::<u64>() as f64 / window.len() as f64;

    // Spike detection: recent mean exceeds baseline by SPIKE_THRESHOLD standard deviations.
    let stddev = baseline.ema_variance.sqrt().max(1.0);
    let z_score = (window_mean - baseline.ema_latency) / stddev;

    let new_status = if z_score > SPIKE_THRESHOLD {
        PortHealthStatus::Degraded
    } else {
        PortHealthStatus::Healthy
    };

    baseline.status = new_status;
    Some(new_status)
}

/// Start a background thread that periodically analyzes port latency and
/// emits health facts into the world state store.
#[cfg(feature = "native")]
pub fn start_port_health_monitor(
    metrics: Arc<RuntimeMetrics>,
    world_state: Arc<Mutex<dyn WorldStateStore + Send>>,
    interval_secs: u64,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("soma-port-health".to_string())
        .spawn(move || {
            let mut baselines: HashMap<String, PortBaseline> = HashMap::new();
            let mut prev_counts: HashMap<String, u64> = HashMap::new();

            loop {
                thread::sleep(Duration::from_secs(interval_secs));

                let snap = metrics.snapshot();

                // Track which ports we saw this tick to detect disappeared ports.
                let mut seen_ports: Vec<String> = Vec::new();

                for (port_id, samples) in &snap.port_latencies {
                    seen_ports.push(port_id.clone());

                    let baseline = baselines
                        .entry(port_id.clone())
                        .or_insert_with(PortBaseline::new);
                    let prev = prev_counts.get(port_id).copied().unwrap_or(0);

                    if let Some(new_status) = analyze_port(baseline, samples, prev) {
                        prev_counts.insert(port_id.clone(), samples.len() as u64);

                        if !baseline.is_calibrated() {
                            continue;
                        }

                        let old_fact_id = format!("port_health_{port_id}");

                        // Always emit the current health fact so the brain
                        // and reactive monitor have up-to-date port status.
                        let fact = crate::types::belief::Fact {
                            fact_id: old_fact_id,
                            subject: "port".to_string(),
                            predicate: format!("{port_id}.health"),
                            value: serde_json::json!({
                                "status": new_status.to_string(),
                                "ema_latency_ms": (baseline.ema_latency * 10.0).round() / 10.0,
                                "ema_stddev_ms": (baseline.ema_variance.sqrt() * 10.0).round() / 10.0,
                                "samples": baseline.samples_seen,
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                            }),
                            confidence: 1.0,
                            provenance: crate::types::common::FactProvenance::Observed,
                            timestamp: chrono::Utc::now(),
                            ttl_ms: None, prior_confidence: None, prediction_error: None,
                        };
                        if let Ok(mut ws) = world_state.lock() {
                            let _ = ws.add_fact(fact);
                        }
                    }
                }
            }
        })
        .expect("failed to spawn port health monitor thread")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_baseline_is_uncalibrated() {
        let b = PortBaseline::new();
        assert!(!b.is_calibrated());
        assert_eq!(b.status, PortHealthStatus::Healthy);
    }

    #[test]
    fn test_analyze_empty_samples_returns_none() {
        let mut b = PortBaseline::new();
        assert!(analyze_port(&mut b, &[], 0).is_none());
    }

    #[test]
    fn test_analyze_warmup_phase() {
        let mut b = PortBaseline::new();
        let samples: Vec<u64> = (1..=5).collect();
        let result = analyze_port(&mut b, &samples, 0);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), PortHealthStatus::Healthy);
        assert!(!b.is_calibrated());
        assert_eq!(b.samples_seen, 5);
    }

    #[test]
    fn test_analyze_calibrated_healthy() {
        let mut b = PortBaseline::new();
        // Feed 20 samples of consistent latency (~50ms).
        let samples: Vec<u64> = (0..20).map(|_| 50).collect();
        analyze_port(&mut b, &samples, 0);
        assert!(b.is_calibrated());
        assert_eq!(b.status, PortHealthStatus::Healthy);
    }

    #[test]
    fn test_analyze_detects_spike() {
        let mut b = PortBaseline::new();
        // Establish baseline with 20 samples at ~10ms.
        let mut samples: Vec<u64> = vec![10; 20];
        analyze_port(&mut b, &samples, 0);
        assert!(b.is_calibrated());
        assert_eq!(b.status, PortHealthStatus::Healthy);

        // Add spike samples (~500ms) and analyze.
        for _ in 0..5 {
            samples.push(500);
        }
        let result = analyze_port(&mut b, &samples, 20);
        assert_eq!(result.unwrap(), PortHealthStatus::Degraded);
    }

    #[test]
    fn test_analyze_recovers_from_degraded() {
        let mut b = PortBaseline::new();
        let mut samples: Vec<u64> = vec![10; 20];
        analyze_port(&mut b, &samples, 0);

        // Spike.
        for _ in 0..5 {
            samples.push(500);
        }
        analyze_port(&mut b, &samples, 20);
        assert_eq!(b.status, PortHealthStatus::Degraded);

        // Recovery: more normal samples.
        for _ in 0..10 {
            samples.push(12);
        }
        let result = analyze_port(&mut b, &samples, 25);
        assert_eq!(result.unwrap(), PortHealthStatus::Healthy);
    }

    #[test]
    fn test_analyze_no_new_samples_returns_none() {
        let mut b = PortBaseline::new();
        let samples: Vec<u64> = vec![10; 20];
        analyze_port(&mut b, &samples, 0);

        // Same count, no new samples.
        let result = analyze_port(&mut b, &samples, 20);
        assert!(result.is_none());
    }

    #[test]
    fn test_status_display() {
        assert_eq!(PortHealthStatus::Healthy.to_string(), "healthy");
        assert_eq!(PortHealthStatus::Degraded.to_string(), "degraded");
        assert_eq!(PortHealthStatus::Unresponsive.to_string(), "unresponsive");
    }

    #[test]
    fn test_ema_converges_to_mean() {
        let mut b = PortBaseline::new();
        let samples: Vec<u64> = vec![100; 50];
        analyze_port(&mut b, &samples, 0);
        assert!((b.ema_latency - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_gradual_increase_stays_healthy() {
        let mut b = PortBaseline::new();
        // Baseline with natural variance (8-12ms range).
        let mut samples: Vec<u64> = (0..20).map(|i| 8 + (i % 5)).collect();
        analyze_port(&mut b, &samples, 0);

        // Gradual increase to ~13ms — within normal variance.
        for _ in 0..5 {
            samples.push(13);
        }
        let result = analyze_port(&mut b, &samples, 20);
        assert_eq!(result.unwrap(), PortHealthStatus::Healthy);
    }

    #[test]
    fn test_single_sample_no_panic() {
        let mut b = PortBaseline::new();
        let result = analyze_port(&mut b, &[42], 0);
        assert!(result.is_some());
        assert_eq!(b.samples_seen, 1);
    }
}
