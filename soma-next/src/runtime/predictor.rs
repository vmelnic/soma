use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// --- Constants ---

/// Default success probability for unknown skills (maximum uncertainty).
const DEFAULT_SUCCESS_PROBABILITY: f64 = 0.5;

/// Default cost estimate for unknown skills (dimensionless units).
const DEFAULT_COST: f64 = 1.0;

/// Default latency estimate for unknown skills (milliseconds).
const DEFAULT_LATENCY_MS: u64 = 100;

/// Exponential moving average smoothing factor.
/// 0.1 = slow adaptation, favors long-term history over recent observations.
const EMA_ALPHA: f64 = 0.1;

// --- PredictionContext ---

/// Input context for all prediction methods.
/// Carries the runtime snapshot that a predictor needs to estimate outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionContext {
    /// Compact summary of current beliefs (subset of belief state relevant to the decision).
    pub belief_summary: serde_json::Value,
    /// Compact summary of the active goal(s).
    pub goal_summary: serde_json::Value,
    /// Number of steps already executed in the current episode.
    pub step_count: u32,
    /// Remaining budget (dimensionless; maps to GoalSpec.resource_budget).
    pub budget_remaining: f64,
    /// Recent observations, most-recent last. Capped by the caller.
    pub recent_observations: Vec<serde_json::Value>,
}

// --- PredictionFeedback ---

/// Observation used to calibrate the predictor after a skill execution completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionFeedback {
    pub skill_id: String,
    pub predicted_success: f64,
    pub actual_success: bool,
    pub predicted_cost: f64,
    pub actual_cost: f64,
    pub predicted_latency_ms: u64,
    pub actual_latency_ms: u64,
}

// --- Predictor trait ---

/// Short-horizon consequence estimator.
///
/// The predictor is OPTIONAL as a learned module. When absent or disabled,
/// the DefaultPredictor provides heuristic estimates from running averages.
/// Correctness, observability, safety, and policy remain intact regardless;
/// only quality or speed may degrade.
pub trait Predictor: Send + Sync {
    /// Estimate the probability (0.0..=1.0) that `skill_id` will succeed
    /// given the current `context`.
    fn predict_success(&self, skill_id: &str, context: &PredictionContext) -> f64;

    /// Estimate the resource cost (dimensionless) of executing `skill_id`.
    fn predict_cost(&self, skill_id: &str, context: &PredictionContext) -> f64;

    /// Estimate the wall-clock latency in milliseconds.
    fn predict_latency_ms(&self, skill_id: &str, context: &PredictionContext) -> u64;

    /// Estimate the belief patch that would result from executing `skill_id`.
    /// Returns a JSON value representing the predicted belief delta.
    fn predict_next_belief(
        &self,
        skill_id: &str,
        context: &PredictionContext,
    ) -> serde_json::Value;

    /// Estimate the information gain from executing `skill_id`.
    /// Higher values indicate the skill is expected to reveal more novel information.
    fn predict_information_gain(&self, skill_id: &str, context: &PredictionContext) -> f64;

    /// Update internal estimates from an actual observation.
    fn calibrate(&mut self, observation: &PredictionFeedback);
}

// --- SkillStats ---

/// Per-skill running statistics maintained by DefaultPredictor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStats {
    /// Exponential moving average of success (1.0 = success, 0.0 = failure).
    pub success_rate: f64,
    /// Exponential moving average of observed cost.
    pub avg_cost: f64,
    /// Exponential moving average of observed latency in milliseconds.
    pub avg_latency_ms: f64,
    /// Total number of calibration observations received for this skill.
    pub times_used: u64,
}

impl SkillStats {
    /// Create stats with the given cost and latency priors.
    fn with_priors(cost_prior: f64, latency_prior_ms: f64) -> Self {
        Self {
            success_rate: DEFAULT_SUCCESS_PROBABILITY,
            avg_cost: cost_prior,
            avg_latency_ms: latency_prior_ms,
            times_used: 0,
        }
    }
}

impl Default for SkillStats {
    fn default() -> Self {
        Self {
            success_rate: DEFAULT_SUCCESS_PROBABILITY,
            avg_cost: DEFAULT_COST,
            avg_latency_ms: DEFAULT_LATENCY_MS as f64,
            times_used: 0,
        }
    }
}

// --- PredictionSummary ---

/// Convenience struct that bundles all predictions for a single skill invocation.
/// Not part of the trait, but useful for callers that need the full picture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionSummary {
    pub skill_id: String,
    pub success_probability: f64,
    pub estimated_cost: f64,
    pub estimated_latency_ms: u64,
    pub estimated_next_belief: serde_json::Value,
    pub information_gain: f64,
}

/// Produce a full PredictionSummary from any Predictor implementation.
pub fn summarize_prediction(
    predictor: &dyn Predictor,
    skill_id: &str,
    context: &PredictionContext,
) -> PredictionSummary {
    PredictionSummary {
        skill_id: skill_id.to_string(),
        success_probability: predictor.predict_success(skill_id, context),
        estimated_cost: predictor.predict_cost(skill_id, context),
        estimated_latency_ms: predictor.predict_latency_ms(skill_id, context),
        estimated_next_belief: predictor.predict_next_belief(skill_id, context),
        information_gain: predictor.predict_information_gain(skill_id, context),
    }
}

// --- DefaultPredictor ---

/// Heuristic (non-learned) predictor that maintains per-skill running averages.
///
/// When no learned model is loaded, this implementation provides safe,
/// conservative estimates derived from exponential moving averages of
/// actual observations. Unknown skills get neutral priors.
pub struct DefaultPredictor {
    stats: HashMap<String, SkillStats>,
}

impl DefaultPredictor {
    pub fn new() -> Self {
        Self {
            stats: HashMap::new(),
        }
    }

    /// Create a predictor pre-seeded with priors for known skills.
    /// `priors` maps skill_id to (cost_prior, latency_prior_ms).
    pub fn with_priors(priors: HashMap<String, (f64, f64)>) -> Self {
        let stats = priors
            .into_iter()
            .map(|(id, (cost, latency))| (id, SkillStats::with_priors(cost, latency)))
            .collect();
        Self { stats }
    }

    /// Read-only access to accumulated per-skill stats.
    pub fn stats(&self) -> &HashMap<String, SkillStats> {
        &self.stats
    }

    /// Look up stats for a skill, falling back to defaults for unknown skills.
    fn get_or_default(&self, skill_id: &str) -> SkillStats {
        self.stats
            .get(skill_id)
            .cloned()
            .unwrap_or_default()
    }
}

impl Default for DefaultPredictor {
    fn default() -> Self {
        Self::new()
    }
}

impl Predictor for DefaultPredictor {
    fn predict_success(&self, skill_id: &str, _context: &PredictionContext) -> f64 {
        self.get_or_default(skill_id).success_rate
    }

    fn predict_cost(&self, skill_id: &str, _context: &PredictionContext) -> f64 {
        self.get_or_default(skill_id).avg_cost
    }

    fn predict_latency_ms(&self, skill_id: &str, _context: &PredictionContext) -> u64 {
        self.get_or_default(skill_id).avg_latency_ms.round() as u64
    }

    fn predict_next_belief(
        &self,
        skill_id: &str,
        _context: &PredictionContext,
    ) -> serde_json::Value {
        // The heuristic predictor cannot predict belief deltas.
        // Return an empty object, which is a no-op patch.
        // A learned predictor would return a meaningful belief delta here.
        serde_json::json!({
            "predictor": "heuristic",
            "skill_id": skill_id,
            "patch": {}
        })
    }

    fn predict_information_gain(&self, skill_id: &str, _context: &PredictionContext) -> f64 {
        let stats = self.get_or_default(skill_id);
        // Novel skills (never or rarely used) have higher information gain.
        // Decays as 1/(1+n), so first use = 0.5, after 9 uses = 0.1, etc.
        1.0 / (1.0 + stats.times_used as f64)
    }

    fn calibrate(&mut self, observation: &PredictionFeedback) {
        let stats = self.stats
            .entry(observation.skill_id.clone())
            .or_default();

        let actual_success_f64 = if observation.actual_success { 1.0 } else { 0.0 };

        // Exponential moving average: new = alpha * observation + (1 - alpha) * old
        stats.success_rate =
            EMA_ALPHA * actual_success_f64 + (1.0 - EMA_ALPHA) * stats.success_rate;
        stats.avg_cost =
            EMA_ALPHA * observation.actual_cost + (1.0 - EMA_ALPHA) * stats.avg_cost;
        stats.avg_latency_ms =
            EMA_ALPHA * observation.actual_latency_ms as f64
                + (1.0 - EMA_ALPHA) * stats.avg_latency_ms;
        stats.times_used += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> PredictionContext {
        PredictionContext {
            belief_summary: serde_json::json!({"temperature": 22.5}),
            goal_summary: serde_json::json!({"objective": "test"}),
            step_count: 0,
            budget_remaining: 100.0,
            recent_observations: vec![],
        }
    }

    fn make_feedback(skill_id: &str, success: bool, cost: f64, latency_ms: u64) -> PredictionFeedback {
        PredictionFeedback {
            skill_id: skill_id.to_string(),
            predicted_success: 0.5,
            actual_success: success,
            predicted_cost: 1.0,
            actual_cost: cost,
            predicted_latency_ms: 100,
            actual_latency_ms: latency_ms,
        }
    }

    // --- DefaultPredictor construction ---

    #[test]
    fn new_predictor_is_empty() {
        let p = DefaultPredictor::new();
        assert!(p.stats().is_empty());
    }

    #[test]
    fn default_predictor_via_default_trait() {
        let p = DefaultPredictor::default();
        assert!(p.stats().is_empty());
    }

    #[test]
    fn predictor_with_priors() {
        let mut priors = HashMap::new();
        priors.insert("db.query".to_string(), (5.0, 50.0));
        priors.insert("http.get".to_string(), (2.0, 200.0));
        let p = DefaultPredictor::with_priors(priors);
        assert_eq!(p.stats().len(), 2);
        assert_eq!(p.stats()["db.query"].avg_cost, 5.0);
        assert_eq!(p.stats()["http.get"].avg_latency_ms, 200.0);
    }

    // --- Unknown skill defaults ---

    #[test]
    fn unknown_skill_success_is_default() {
        let p = DefaultPredictor::new();
        let ctx = make_context();
        assert_eq!(p.predict_success("unknown.skill", &ctx), DEFAULT_SUCCESS_PROBABILITY);
    }

    #[test]
    fn unknown_skill_cost_is_default() {
        let p = DefaultPredictor::new();
        let ctx = make_context();
        assert_eq!(p.predict_cost("unknown.skill", &ctx), DEFAULT_COST);
    }

    #[test]
    fn unknown_skill_latency_is_default() {
        let p = DefaultPredictor::new();
        let ctx = make_context();
        assert_eq!(p.predict_latency_ms("unknown.skill", &ctx), DEFAULT_LATENCY_MS);
    }

    #[test]
    fn unknown_skill_information_gain_is_one() {
        let p = DefaultPredictor::new();
        let ctx = make_context();
        // times_used = 0 => 1.0 / (1.0 + 0) = 1.0
        assert_eq!(p.predict_information_gain("unknown.skill", &ctx), 1.0);
    }

    #[test]
    fn unknown_skill_next_belief_is_empty_patch() {
        let p = DefaultPredictor::new();
        let ctx = make_context();
        let patch = p.predict_next_belief("unknown.skill", &ctx);
        assert_eq!(patch["predictor"], "heuristic");
        assert_eq!(patch["patch"], serde_json::json!({}));
    }

    // --- Calibration: single observation ---

    #[test]
    fn calibrate_single_success() {
        let mut p = DefaultPredictor::new();
        p.calibrate(&make_feedback("s1", true, 10.0, 500));
        let stats = &p.stats()["s1"];
        // EMA from default 0.5: 0.1 * 1.0 + 0.9 * 0.5 = 0.55
        assert!((stats.success_rate - 0.55).abs() < 1e-9);
        // EMA from default 1.0: 0.1 * 10.0 + 0.9 * 1.0 = 1.9
        assert!((stats.avg_cost - 1.9).abs() < 1e-9);
        // EMA from default 100.0: 0.1 * 500.0 + 0.9 * 100.0 = 140.0
        assert!((stats.avg_latency_ms - 140.0).abs() < 1e-9);
        assert_eq!(stats.times_used, 1);
    }

    #[test]
    fn calibrate_single_failure() {
        let mut p = DefaultPredictor::new();
        p.calibrate(&make_feedback("s1", false, 2.0, 50));
        let stats = &p.stats()["s1"];
        // EMA from default 0.5: 0.1 * 0.0 + 0.9 * 0.5 = 0.45
        assert!((stats.success_rate - 0.45).abs() < 1e-9);
        assert_eq!(stats.times_used, 1);
    }

    // --- Calibration: multiple observations converge ---

    #[test]
    fn calibrate_converges_toward_observations() {
        let mut p = DefaultPredictor::new();
        let ctx = make_context();

        // Feed 100 successes with cost=10, latency=200
        for _ in 0..100 {
            p.calibrate(&make_feedback("converge", true, 10.0, 200));
        }

        let success = p.predict_success("converge", &ctx);
        let cost = p.predict_cost("converge", &ctx);
        let latency = p.predict_latency_ms("converge", &ctx);

        // After many observations, EMA should be very close to the observed values
        assert!(success > 0.99, "success should converge near 1.0, got {success}");
        assert!((cost - 10.0).abs() < 0.1, "cost should converge near 10.0, got {cost}");
        assert!(
            (latency as f64 - 200.0).abs() < 1.0,
            "latency should converge near 200, got {latency}"
        );
    }

    #[test]
    fn calibrate_converges_toward_failure() {
        let mut p = DefaultPredictor::new();
        let ctx = make_context();

        for _ in 0..100 {
            p.calibrate(&make_feedback("fail_skill", false, 0.5, 10));
        }

        let success = p.predict_success("fail_skill", &ctx);
        assert!(success < 0.01, "success should converge near 0.0, got {success}");
    }

    // --- Information gain decay ---

    #[test]
    fn information_gain_decays_with_usage() {
        let mut p = DefaultPredictor::new();
        let ctx = make_context();

        // Before any use
        assert_eq!(p.predict_information_gain("decay_test", &ctx), 1.0);

        // After 1 use: 1/(1+1) = 0.5
        p.calibrate(&make_feedback("decay_test", true, 1.0, 100));
        assert!((p.predict_information_gain("decay_test", &ctx) - 0.5).abs() < 1e-9);

        // After 9 more uses (10 total): 1/(1+10) ≈ 0.0909
        for _ in 0..9 {
            p.calibrate(&make_feedback("decay_test", true, 1.0, 100));
        }
        let ig = p.predict_information_gain("decay_test", &ctx);
        assert!((ig - 1.0 / 11.0).abs() < 1e-9);
    }

    // --- Per-skill isolation ---

    #[test]
    fn different_skills_tracked_independently() {
        let mut p = DefaultPredictor::new();
        let ctx = make_context();

        // Skill A: always succeeds, high cost
        for _ in 0..50 {
            p.calibrate(&make_feedback("skill_a", true, 100.0, 1000));
        }
        // Skill B: always fails, low cost
        for _ in 0..50 {
            p.calibrate(&make_feedback("skill_b", false, 0.1, 5));
        }

        assert!(p.predict_success("skill_a", &ctx) > 0.95);
        assert!(p.predict_success("skill_b", &ctx) < 0.05);
        assert!(p.predict_cost("skill_a", &ctx) > 50.0);
        assert!(p.predict_cost("skill_b", &ctx) < 1.0);
    }

    // --- Priors are respected ---

    #[test]
    fn priors_used_before_calibration() {
        let mut priors = HashMap::new();
        priors.insert("slow_skill".to_string(), (50.0, 5000.0));
        let p = DefaultPredictor::with_priors(priors);
        let ctx = make_context();

        assert_eq!(p.predict_cost("slow_skill", &ctx), 50.0);
        assert_eq!(p.predict_latency_ms("slow_skill", &ctx), 5000);
        // Success rate still starts at default
        assert_eq!(p.predict_success("slow_skill", &ctx), DEFAULT_SUCCESS_PROBABILITY);
    }

    #[test]
    fn priors_shift_with_calibration() {
        let mut priors = HashMap::new();
        priors.insert("shift_test".to_string(), (50.0, 5000.0));
        let mut p = DefaultPredictor::with_priors(priors);
        let ctx = make_context();

        // Observe much lower cost/latency
        for _ in 0..100 {
            p.calibrate(&make_feedback("shift_test", true, 1.0, 10));
        }

        let cost = p.predict_cost("shift_test", &ctx);
        let latency = p.predict_latency_ms("shift_test", &ctx);
        assert!(cost < 2.0, "cost should have shifted toward 1.0, got {cost}");
        assert!(latency < 20, "latency should have shifted toward 10, got {latency}");
    }

    // --- PredictionSummary helper ---

    #[test]
    fn summarize_prediction_bundles_all_fields() {
        let p = DefaultPredictor::new();
        let ctx = make_context();
        let summary = summarize_prediction(&p, "bundle_test", &ctx);

        assert_eq!(summary.skill_id, "bundle_test");
        assert_eq!(summary.success_probability, DEFAULT_SUCCESS_PROBABILITY);
        assert_eq!(summary.estimated_cost, DEFAULT_COST);
        assert_eq!(summary.estimated_latency_ms, DEFAULT_LATENCY_MS);
        assert_eq!(summary.information_gain, 1.0);
        assert_eq!(summary.estimated_next_belief["predictor"], "heuristic");
    }

    // --- PredictionContext and PredictionFeedback serialization ---

    #[test]
    fn prediction_context_round_trips_json() {
        let ctx = make_context();
        let json = serde_json::to_string(&ctx).unwrap();
        let ctx2: PredictionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx.step_count, ctx2.step_count);
        assert_eq!(ctx.budget_remaining, ctx2.budget_remaining);
    }

    #[test]
    fn prediction_feedback_round_trips_json() {
        let fb = make_feedback("rt", true, std::f64::consts::PI, 42);
        let json = serde_json::to_string(&fb).unwrap();
        let fb2: PredictionFeedback = serde_json::from_str(&json).unwrap();
        assert_eq!(fb.skill_id, fb2.skill_id);
        assert_eq!(fb.actual_success, fb2.actual_success);
        assert!((fb.actual_cost - fb2.actual_cost).abs() < 1e-9);
        assert_eq!(fb.actual_latency_ms, fb2.actual_latency_ms);
    }

    #[test]
    fn skill_stats_round_trips_json() {
        let stats = SkillStats {
            success_rate: 0.85,
            avg_cost: 12.5,
            avg_latency_ms: 340.0,
            times_used: 77,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let stats2: SkillStats = serde_json::from_str(&json).unwrap();
        assert!((stats.success_rate - stats2.success_rate).abs() < 1e-9);
        assert_eq!(stats.times_used, stats2.times_used);
    }

    // --- Edge cases ---

    #[test]
    fn calibrate_zero_cost_and_latency() {
        let mut p = DefaultPredictor::new();
        p.calibrate(&make_feedback("zero", true, 0.0, 0));
        let stats = &p.stats()["zero"];
        // EMA from default 1.0: 0.1 * 0.0 + 0.9 * 1.0 = 0.9
        assert!((stats.avg_cost - 0.9).abs() < 1e-9);
        // EMA from default 100.0: 0.1 * 0.0 + 0.9 * 100.0 = 90.0
        assert!((stats.avg_latency_ms - 90.0).abs() < 1e-9);
    }

    #[test]
    fn context_with_observations() {
        let ctx = PredictionContext {
            belief_summary: serde_json::json!({}),
            goal_summary: serde_json::json!({}),
            step_count: 5,
            budget_remaining: 42.0,
            recent_observations: vec![
                serde_json::json!({"step": 1, "result": "ok"}),
                serde_json::json!({"step": 2, "result": "ok"}),
            ],
        };
        assert_eq!(ctx.recent_observations.len(), 2);
        assert_eq!(ctx.step_count, 5);
    }
}
