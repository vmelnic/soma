//! Per-skill observed-cost statistics.
//!
//! The body keeps an exponentially-weighted moving average of latency,
//! resource cost, and success rate per skill. Updated on every episode by
//! `update_from_episode`, read by the predictor (overlaid on the static
//! `cost_prior` so scoring uses observed data once `n_observed >= 5`).
//!
//! Persisted to `<state_dir>/skill_stats.json` so calibration survives a
//! runtime restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::errors::{Result, SomaError};
use crate::types::episode::Episode;

const EMA_ALPHA: f64 = 0.1;
const MIN_OBSERVED_FOR_OVERLAY: u64 = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillStats {
    pub skill_id: String,
    pub n_observed: u64,
    pub ema_latency_ms: f64,
    pub ema_resource_cost: f64,
    pub ema_success_rate: f64,
}

impl SkillStats {
    fn update(&mut self, latency_ms: u64, resource_cost: f64, success: bool) {
        self.n_observed += 1;
        if self.n_observed == 1 {
            self.ema_latency_ms = latency_ms as f64;
            self.ema_resource_cost = resource_cost;
            self.ema_success_rate = if success { 1.0 } else { 0.0 };
            return;
        }
        self.ema_latency_ms =
            EMA_ALPHA * latency_ms as f64 + (1.0 - EMA_ALPHA) * self.ema_latency_ms;
        self.ema_resource_cost =
            EMA_ALPHA * resource_cost + (1.0 - EMA_ALPHA) * self.ema_resource_cost;
        let s = if success { 1.0 } else { 0.0 };
        self.ema_success_rate = EMA_ALPHA * s + (1.0 - EMA_ALPHA) * self.ema_success_rate;
    }

    /// True once we've seen enough samples to trust the EMA over the prior.
    pub fn is_calibrated(&self) -> bool {
        self.n_observed >= MIN_OBSERVED_FOR_OVERLAY
    }
}

/// Map skill_id → SkillStats. Mutated under a single lock; persisted as JSON.
pub struct SkillStatsStore {
    inner: Mutex<HashMap<String, SkillStats>>,
    path: Option<PathBuf>,
}

impl SkillStatsStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            path: None,
        }
    }

    /// Open a store backed by a JSON file. Creates the file on first save.
    /// Missing or corrupt files start with empty stats.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let p = path.as_ref().to_path_buf();
        let map = std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, SkillStats>>(&s).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(map),
            path: Some(p),
        }
    }

    pub fn get(&self, skill_id: &str) -> Option<SkillStats> {
        self.inner.lock().ok()?.get(skill_id).cloned()
    }

    pub fn snapshot(&self) -> HashMap<String, SkillStats> {
        self.inner
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Update stats from one episode by walking the trace's port_calls.
    /// Each step's chosen skill gets one update with the step's aggregate
    /// latency, derived resource cost, and success flag.
    pub fn update_from_episode(&self, episode: &Episode) -> Result<()> {
        let mut map = self
            .inner
            .lock()
            .map_err(|e| SomaError::Memory(format!("skill stats lock poisoned: {e}")))?;
        for step in &episode.steps {
            let entry = map
                .entry(step.selected_skill.clone())
                .or_insert_with(|| SkillStats {
                    skill_id: step.selected_skill.clone(),
                    ..SkillStats::default()
                });
            let success = step.observation.success
                && step.observation.port_calls.iter().all(|p| p.success);
            let latency_ms = step.observation.latency_ms;
            // Map the cost profile back to a scalar via the same weights the
            // session controller uses for budget deduction.
            let r = &step.observation.resource_cost;
            let cv = |c: crate::types::common::CostClass| match c {
                crate::types::common::CostClass::Negligible => 0.0,
                crate::types::common::CostClass::Low => 0.25,
                crate::types::common::CostClass::Medium => 0.5,
                crate::types::common::CostClass::High => 0.75,
                crate::types::common::CostClass::Extreme => 1.0,
            };
            let scalar = cv(r.cpu_cost_class) * 0.3
                + cv(r.memory_cost_class) * 0.2
                + cv(r.io_cost_class) * 0.2
                + cv(r.network_cost_class) * 0.2
                + cv(r.energy_cost_class) * 0.1;
            entry.update(latency_ms, scalar, success);
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let Some(ref p) = self.path else { return Ok(()) };
        let map = self
            .inner
            .lock()
            .map_err(|e| SomaError::Memory(format!("skill stats lock poisoned: {e}")))?;
        let json = serde_json::to_string_pretty(&*map)
            .map_err(|e| SomaError::Memory(format!("serialize skill stats: {e}")))?;
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(p, json)
            .map_err(|e| SomaError::Memory(format!("write skill stats: {e}")))?;
        Ok(())
    }
}

impl Default for SkillStatsStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared handle for the runtime.
pub type SharedSkillStats = Arc<SkillStatsStore>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::episode::*;
    use crate::types::observation::Observation;
    use chrono::Utc;
    use uuid::Uuid;

    fn ep_with_step(skill_id: &str, latency_ms: u64, success: bool) -> Episode {
        Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: "fp".into(),
            initial_belief_summary: serde_json::json!({}),
            steps: vec![EpisodeStep {
                step_index: 0,
                belief_summary: serde_json::json!({}),
                candidates_considered: vec![skill_id.into()],
                predicted_scores: vec![1.0],
                selected_skill: skill_id.into(),
                observation: Observation {
                    observation_id: Uuid::new_v4(),
                    session_id: Uuid::new_v4(),
                    skill_id: Some(skill_id.into()),
                    port_calls: vec![],
                    raw_result: serde_json::Value::Null,
                    structured_result: serde_json::Value::Null,
                    effect_patch: None,
                    success,
                    failure_class: None,
                    failure_detail: None,
                    latency_ms,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 1.0,
                    timestamp: Utc::now(),
                },
                belief_patch: serde_json::Value::Null,
                progress_delta: 0.1,
                critic_decision: "Continue".into(),
                timestamp: Utc::now(),
            }],
            observations: vec![],
            outcome: EpisodeOutcome::Success,
            total_cost: 0.0,
            success: true,
            tags: vec![],
            embedding: None,
            created_at: Utc::now(),
            salience: 1.0,
            world_state_context: serde_json::json!({}),
        }
    }

    #[test]
    fn ema_converges_toward_observed_latency() {
        let s = SkillStatsStore::new();
        for _ in 0..50 {
            s.update_from_episode(&ep_with_step("foo", 200, true)).unwrap();
        }
        let st = s.get("foo").unwrap();
        assert_eq!(st.n_observed, 50);
        // EMA should be close to 200 after 50 samples.
        assert!((st.ema_latency_ms - 200.0).abs() < 5.0);
        assert!((st.ema_success_rate - 1.0).abs() < 0.01);
        assert!(st.is_calibrated());
    }

    #[test]
    fn success_rate_drops_with_failures() {
        let s = SkillStatsStore::new();
        for _ in 0..20 {
            s.update_from_episode(&ep_with_step("bar", 50, true)).unwrap();
        }
        for _ in 0..20 {
            s.update_from_episode(&ep_with_step("bar", 50, false)).unwrap();
        }
        let st = s.get("bar").unwrap();
        // Recent failures pull EMA down.
        assert!(st.ema_success_rate < 0.5, "got {}", st.ema_success_rate);
    }

    #[test]
    fn open_creates_empty_when_file_missing() {
        let dir = std::env::temp_dir().join(format!("soma_stats_{}", Uuid::new_v4()));
        let path = dir.join("stats.json");
        let s = SkillStatsStore::open(&path);
        assert!(s.snapshot().is_empty());
        s.update_from_episode(&ep_with_step("baz", 10, true)).unwrap();
        s.save().unwrap();
        // Reopen and verify persistence.
        let s2 = SkillStatsStore::open(&path);
        assert!(s2.get("baz").is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
