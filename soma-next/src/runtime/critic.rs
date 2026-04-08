use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::Result;
use crate::types::common::{Budget, CriticDecision};

// --- Constants ---

/// Number of recent steps to consider for progress stall detection.
const PROGRESS_WINDOW: usize = 3;

/// Minimum average progress delta before declaring a stall.
const PROGRESS_STALL_THRESHOLD: f64 = 0.01;

/// Number of times a belief hash must repeat to trigger loop detection.
const BELIEF_LOOP_THRESHOLD: usize = 3;

/// Window of recent skill selections to check for repetition.
const SKILL_REPEAT_WINDOW: usize = 5;

/// Number of times a skill must repeat within the window to trigger loop detection.
const SKILL_REPEAT_THRESHOLD: usize = 3;

/// Number of consecutive failures before declaring a dead end.
const DEAD_END_THRESHOLD: u32 = 3;

/// Budget fraction below which a budget overrun is flagged.
const BUDGET_OVERRUN_FRACTION: f64 = 0.10;

// --- ObservationSummary ---

/// A condensed record of a single step's observation, used by the Critic
/// to detect patterns without needing the full observation payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationSummary {
    pub success: bool,
    pub skill_id: String,
    pub latency_ms: u64,
    pub cost: f64,
}

// --- CriticContext ---

/// Everything the Critic needs to make a decision. Assembled by the session
/// runtime from the current episode state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticContext {
    pub session_id: Uuid,
    pub step_count: u32,
    pub max_steps: u32,
    pub budget_remaining: Budget,
    pub initial_budget: Budget,
    pub recent_observations: Vec<ObservationSummary>,
    pub progress_history: Vec<f64>,
    pub selected_skills_history: Vec<String>,
    pub current_belief_hash: String,
    pub belief_hash_history: Vec<String>,
}

// --- CriticDetails ---

/// Detailed findings from the Critic's analysis. Every field is independently
/// computed; the decision logic then combines them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticDetails {
    pub progress_rate: f64,
    pub loop_detected: bool,
    pub dead_end_detected: bool,
    pub contradiction_detected: bool,
    pub budget_overrun: bool,
    pub consecutive_failures: u32,
}

// --- CriticEvaluation ---

/// The Critic's output: a decision, its confidence, a human-readable reason,
/// and the underlying details that led to the decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticEvaluation {
    pub decision: CriticDecision,
    pub confidence: f64,
    pub reason: String,
    pub details: CriticDetails,
}

// --- Critic trait ---

/// The Critic evaluates the current episode state and decides whether to
/// continue, revise the plan, backtrack, delegate, or stop.
pub trait Critic: Send + Sync {
    fn evaluate(&self, context: &CriticContext) -> Result<CriticEvaluation>;
}

// --- DefaultCritic ---

/// Rule-based Critic implementation. Applies deterministic heuristics to
/// detect loops, dead ends, contradictions, budget overruns, and progress
/// stalls, then maps findings to a CriticDecision.
pub struct DefaultCritic;

impl DefaultCritic {
    pub fn new() -> Self {
        Self
    }

    /// Detect loops via two signals:
    /// 1. The same belief hash appears 3+ times in the full history.
    /// 2. The same skill is selected 3+ times in the last 5 steps.
    fn detect_loop(context: &CriticContext) -> bool {
        // Signal 1: belief hash repetition
        let mut hash_counts: HashMap<&str, usize> = HashMap::new();
        for h in &context.belief_hash_history {
            *hash_counts.entry(h.as_str()).or_insert(0) += 1;
        }
        if hash_counts.values().any(|&c| c >= BELIEF_LOOP_THRESHOLD) {
            return true;
        }

        // Signal 2: skill repetition in recent window
        let skills = &context.selected_skills_history;
        let window_start = skills.len().saturating_sub(SKILL_REPEAT_WINDOW);
        let window = &skills[window_start..];
        let mut skill_counts: HashMap<&str, usize> = HashMap::new();
        for s in window {
            *skill_counts.entry(s.as_str()).or_insert(0) += 1;
        }
        if skill_counts.values().any(|&c| c >= SKILL_REPEAT_THRESHOLD) {
            return true;
        }

        false
    }

    /// Detect dead ends: 3+ consecutive failures in the most recent observations.
    fn detect_dead_end(context: &CriticContext) -> (bool, u32) {
        let mut consecutive = 0u32;
        for obs in context.recent_observations.iter().rev() {
            if !obs.success {
                consecutive += 1;
            } else {
                break;
            }
        }
        (consecutive >= DEAD_END_THRESHOLD, consecutive)
    }

    /// Detect budget overrun: any budget dimension remaining < 10% of initial.
    fn detect_budget_overrun(context: &CriticContext) -> bool {
        let remaining = &context.budget_remaining;
        let initial = &context.initial_budget;

        // Risk
        if initial.risk_remaining > 0.0
            && remaining.risk_remaining < initial.risk_remaining * BUDGET_OVERRUN_FRACTION
        {
            return true;
        }

        // Latency
        if initial.latency_remaining_ms > 0 {
            let threshold =
                (initial.latency_remaining_ms as f64 * BUDGET_OVERRUN_FRACTION) as u64;
            if remaining.latency_remaining_ms < threshold {
                return true;
            }
        }

        // Resource
        if initial.resource_remaining > 0.0
            && remaining.resource_remaining
                < initial.resource_remaining * BUDGET_OVERRUN_FRACTION
        {
            return true;
        }

        // Steps
        if initial.steps_remaining > 0 {
            let threshold =
                (initial.steps_remaining as f64 * BUDGET_OVERRUN_FRACTION).ceil() as u32;
            if remaining.steps_remaining < threshold {
                return true;
            }
        }

        false
    }

    /// Compute average progress delta over the last N steps.
    fn compute_progress_rate(context: &CriticContext) -> f64 {
        let history = &context.progress_history;
        if history.is_empty() {
            return 0.0;
        }
        let window_start = history.len().saturating_sub(PROGRESS_WINDOW);
        let window = &history[window_start..];
        let sum: f64 = window.iter().sum();
        sum / window.len() as f64
    }

    /// Detect contradiction: belief hash oscillating in an A->B->A pattern
    /// anywhere in the last 3 entries of the history.
    fn detect_contradiction(context: &CriticContext) -> bool {
        let history = &context.belief_hash_history;
        if history.len() < 3 {
            return false;
        }
        // Check the tail for A->B->A: history[i] == history[i+2] && history[i] != history[i+1]
        for window in history.windows(3) {
            if window[0] == window[2] && window[0] != window[1] {
                return true;
            }
        }
        false
    }
}

impl Default for DefaultCritic {
    fn default() -> Self {
        Self::new()
    }
}

impl Critic for DefaultCritic {
    fn evaluate(&self, context: &CriticContext) -> Result<CriticEvaluation> {
        let loop_detected = Self::detect_loop(context);
        let (dead_end_detected, consecutive_failures) = Self::detect_dead_end(context);
        let budget_overrun = Self::detect_budget_overrun(context);
        let progress_rate = Self::compute_progress_rate(context);
        let contradiction_detected = Self::detect_contradiction(context);

        let progress_stall = context.progress_history.len() >= PROGRESS_WINDOW
            && progress_rate < PROGRESS_STALL_THRESHOLD;

        let details = CriticDetails {
            progress_rate,
            loop_detected,
            dead_end_detected,
            contradiction_detected,
            budget_overrun,
            consecutive_failures,
        };

        // Decision logic (priority order):
        // 1. Budget overrun -> Stop (highest priority, cannot continue)
        // 2. Loop or dead end -> Backtrack (need to undo recent path)
        // 3. Progress stall or contradiction -> Revise (change approach)
        // 4. Otherwise -> Continue
        let (decision, confidence, reason) = if budget_overrun {
            (
                CriticDecision::Stop,
                0.95,
                "budget overrun: remaining budget < 10% of initial".to_string(),
            )
        } else if loop_detected && dead_end_detected {
            (
                CriticDecision::Backtrack,
                0.95,
                format!(
                    "loop and dead end detected: {} consecutive failures with repeated state",
                    consecutive_failures
                ),
            )
        } else if loop_detected {
            (
                CriticDecision::Backtrack,
                0.85,
                "loop detected: repeated belief state or skill selection".to_string(),
            )
        } else if dead_end_detected {
            (
                CriticDecision::Backtrack,
                0.85,
                format!(
                    "dead end: {} consecutive failures",
                    consecutive_failures
                ),
            )
        } else if contradiction_detected && progress_stall {
            (
                CriticDecision::Revise,
                0.85,
                "contradiction with progress stall: oscillating beliefs and no forward progress"
                    .to_string(),
            )
        } else if progress_stall {
            (
                CriticDecision::Revise,
                0.75,
                format!(
                    "progress stall: average delta {:.4} over last {} steps",
                    progress_rate, PROGRESS_WINDOW
                ),
            )
        } else if contradiction_detected {
            (
                CriticDecision::Revise,
                0.70,
                "contradiction detected: belief hash oscillation (A->B->A)".to_string(),
            )
        } else {
            (
                CriticDecision::Continue,
                0.80,
                "no anomalies detected".to_string(),
            )
        };

        Ok(CriticEvaluation {
            decision,
            confidence,
            reason,
            details,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_budget(risk: f64, latency_ms: u64, resource: f64, steps: u32) -> Budget {
        Budget {
            risk_remaining: risk,
            latency_remaining_ms: latency_ms,
            resource_remaining: resource,
            steps_remaining: steps,
        }
    }

    fn make_context() -> CriticContext {
        CriticContext {
            session_id: Uuid::new_v4(),
            step_count: 3,
            max_steps: 20,
            budget_remaining: make_budget(0.4, 25_000, 80.0, 17),
            initial_budget: make_budget(0.5, 30_000, 100.0, 20),
            recent_observations: vec![
                ObservationSummary {
                    success: true,
                    skill_id: "skill_a".to_string(),
                    latency_ms: 100,
                    cost: 1.0,
                },
                ObservationSummary {
                    success: true,
                    skill_id: "skill_b".to_string(),
                    latency_ms: 150,
                    cost: 1.5,
                },
                ObservationSummary {
                    success: true,
                    skill_id: "skill_c".to_string(),
                    latency_ms: 120,
                    cost: 1.2,
                },
            ],
            progress_history: vec![0.15, 0.12, 0.10],
            selected_skills_history: vec![
                "skill_a".to_string(),
                "skill_b".to_string(),
                "skill_c".to_string(),
            ],
            current_belief_hash: "hash_d".to_string(),
            belief_hash_history: vec![
                "hash_a".to_string(),
                "hash_b".to_string(),
                "hash_c".to_string(),
            ],
        }
    }

    // --- Basic healthy path ---

    #[test]
    fn healthy_context_returns_continue() {
        let critic = DefaultCritic::new();
        let ctx = make_context();
        let eval = critic.evaluate(&ctx).unwrap();
        assert_eq!(eval.decision, CriticDecision::Continue);
        assert!(!eval.details.loop_detected);
        assert!(!eval.details.dead_end_detected);
        assert!(!eval.details.contradiction_detected);
        assert!(!eval.details.budget_overrun);
        assert_eq!(eval.details.consecutive_failures, 0);
    }

    // --- Loop detection: belief hash ---

    #[test]
    fn loop_detected_via_belief_hash_repetition() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.belief_hash_history = vec![
            "hash_a".to_string(),
            "hash_b".to_string(),
            "hash_a".to_string(),
            "hash_c".to_string(),
            "hash_a".to_string(), // 3rd occurrence of hash_a
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.loop_detected);
        assert_eq!(eval.decision, CriticDecision::Backtrack);
    }

    #[test]
    fn no_loop_with_two_belief_hash_repeats() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.belief_hash_history = vec![
            "hash_a".to_string(),
            "hash_b".to_string(),
            "hash_a".to_string(), // only 2 occurrences
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        // May still detect contradiction (A->B->A), but not a loop via hash count
        assert!(!eval.details.loop_detected);
    }

    // --- Loop detection: skill repetition ---

    #[test]
    fn loop_detected_via_skill_repetition() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.selected_skills_history = vec![
            "skill_x".to_string(),
            "skill_a".to_string(),
            "skill_a".to_string(),
            "skill_a".to_string(), // 3 of last 5 are skill_a
            "skill_b".to_string(),
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.loop_detected);
        assert_eq!(eval.decision, CriticDecision::Backtrack);
    }

    #[test]
    fn no_loop_with_two_skill_repeats_in_window() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.selected_skills_history = vec![
            "skill_a".to_string(),
            "skill_a".to_string(), // only 2 in the window
            "skill_b".to_string(),
            "skill_c".to_string(),
            "skill_d".to_string(),
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(!eval.details.loop_detected);
    }

    #[test]
    fn skill_repetition_only_considers_last_five() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // skill_a appears 3 times but only once in the last 5
        ctx.selected_skills_history = vec![
            "skill_a".to_string(),
            "skill_a".to_string(),
            "skill_a".to_string(),
            "skill_b".to_string(),
            "skill_c".to_string(),
            "skill_d".to_string(),
            "skill_e".to_string(),
            "skill_f".to_string(),
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(!eval.details.loop_detected);
    }

    // --- Dead end detection ---

    #[test]
    fn dead_end_on_three_consecutive_failures() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.recent_observations = vec![
            ObservationSummary {
                success: true,
                skill_id: "s1".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s2".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s3".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s4".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.dead_end_detected);
        assert_eq!(eval.details.consecutive_failures, 3);
        assert_eq!(eval.decision, CriticDecision::Backtrack);
    }

    #[test]
    fn no_dead_end_with_two_consecutive_failures() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.recent_observations = vec![
            ObservationSummary {
                success: false,
                skill_id: "s1".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s2".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(!eval.details.dead_end_detected);
        assert_eq!(eval.details.consecutive_failures, 2);
    }

    #[test]
    fn dead_end_counts_from_tail() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // Failures are not at the tail — a success breaks the streak
        ctx.recent_observations = vec![
            ObservationSummary {
                success: false,
                skill_id: "s1".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s2".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s3".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: true,
                skill_id: "s4".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(!eval.details.dead_end_detected);
        assert_eq!(eval.details.consecutive_failures, 0);
    }

    // --- Budget overrun ---

    #[test]
    fn budget_overrun_on_low_risk() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.budget_remaining.risk_remaining = 0.04; // < 10% of 0.5
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.budget_overrun);
        assert_eq!(eval.decision, CriticDecision::Stop);
    }

    #[test]
    fn budget_overrun_on_low_latency() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.budget_remaining.latency_remaining_ms = 2_000; // < 10% of 30_000
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.budget_overrun);
        assert_eq!(eval.decision, CriticDecision::Stop);
    }

    #[test]
    fn budget_overrun_on_low_resource() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.budget_remaining.resource_remaining = 5.0; // < 10% of 100.0
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.budget_overrun);
        assert_eq!(eval.decision, CriticDecision::Stop);
    }

    #[test]
    fn budget_overrun_on_low_steps() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.budget_remaining.steps_remaining = 1; // < 10% of 20 (threshold = 2)
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.budget_overrun);
        assert_eq!(eval.decision, CriticDecision::Stop);
    }

    #[test]
    fn no_budget_overrun_at_boundary() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // Exactly at 10% — should NOT trigger (threshold is strictly less than)
        ctx.budget_remaining = make_budget(0.05, 3_000, 10.0, 2);
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(!eval.details.budget_overrun);
    }

    // --- Progress stall ---

    #[test]
    fn progress_stall_triggers_revise() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.progress_history = vec![0.005, 0.003, 0.002];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.progress_rate < PROGRESS_STALL_THRESHOLD);
        assert_eq!(eval.decision, CriticDecision::Revise);
    }

    #[test]
    fn no_stall_with_fewer_than_window_steps() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.progress_history = vec![0.001, 0.001]; // only 2 entries, window is 3
        let eval = critic.evaluate(&ctx).unwrap();
        // Even though rate is low, we don't flag stall with < PROGRESS_WINDOW entries
        assert_eq!(eval.decision, CriticDecision::Continue);
    }

    #[test]
    fn progress_rate_computed_over_window() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // 10 entries, but only last 3 matter for the rate
        ctx.progress_history = vec![
            0.5, 0.4, 0.3, 0.2, 0.1, 0.05, 0.03, // early
            0.002, 0.003, 0.001, // last 3: avg = 0.002
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.progress_rate < PROGRESS_STALL_THRESHOLD);
        assert_eq!(eval.decision, CriticDecision::Revise);
    }

    // --- Contradiction detection ---

    #[test]
    fn contradiction_detected_on_oscillation() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.belief_hash_history = vec![
            "hash_a".to_string(),
            "hash_b".to_string(),
            "hash_a".to_string(), // A->B->A
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.contradiction_detected);
        assert_eq!(eval.decision, CriticDecision::Revise);
    }

    #[test]
    fn no_contradiction_without_oscillation() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.belief_hash_history = vec![
            "hash_a".to_string(),
            "hash_b".to_string(),
            "hash_c".to_string(),
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(!eval.details.contradiction_detected);
    }

    #[test]
    fn no_contradiction_with_short_history() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.belief_hash_history = vec!["hash_a".to_string(), "hash_b".to_string()];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(!eval.details.contradiction_detected);
    }

    // --- Decision priority ---

    #[test]
    fn budget_overrun_takes_priority_over_loop() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // Trigger both budget overrun and loop
        ctx.budget_remaining.risk_remaining = 0.01;
        ctx.belief_hash_history = vec![
            "h".to_string(),
            "h".to_string(),
            "h".to_string(),
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.budget_overrun);
        assert!(eval.details.loop_detected);
        assert_eq!(eval.decision, CriticDecision::Stop);
    }

    #[test]
    fn loop_takes_priority_over_progress_stall() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // Trigger both loop and progress stall
        ctx.belief_hash_history = vec![
            "h".to_string(),
            "h".to_string(),
            "h".to_string(),
        ];
        ctx.progress_history = vec![0.001, 0.001, 0.001];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.loop_detected);
        assert_eq!(eval.decision, CriticDecision::Backtrack);
    }

    #[test]
    fn dead_end_takes_priority_over_contradiction() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // Trigger dead end
        ctx.recent_observations = vec![
            ObservationSummary {
                success: false,
                skill_id: "s1".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s2".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s3".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
        ];
        // Trigger contradiction
        ctx.belief_hash_history = vec![
            "a".to_string(),
            "b".to_string(),
            "a".to_string(),
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.dead_end_detected);
        assert!(eval.details.contradiction_detected);
        assert_eq!(eval.decision, CriticDecision::Backtrack);
    }

    // --- Combined loop + dead end ---

    #[test]
    fn loop_and_dead_end_combined_high_confidence() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        // Loop via belief hash
        ctx.belief_hash_history = vec![
            "x".to_string(),
            "x".to_string(),
            "x".to_string(),
        ];
        // Dead end via consecutive failures
        ctx.recent_observations = vec![
            ObservationSummary {
                success: false,
                skill_id: "s1".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s2".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s3".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.loop_detected);
        assert!(eval.details.dead_end_detected);
        assert_eq!(eval.decision, CriticDecision::Backtrack);
        assert!(eval.confidence >= 0.95);
    }

    // --- Contradiction + stall combined ---

    #[test]
    fn contradiction_and_stall_combined_higher_confidence() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.belief_hash_history = vec![
            "a".to_string(),
            "b".to_string(),
            "a".to_string(),
        ];
        ctx.progress_history = vec![0.001, 0.002, 0.001];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.details.contradiction_detected);
        assert_eq!(eval.decision, CriticDecision::Revise);
        assert!(eval.confidence >= 0.85);
    }

    // --- Edge cases ---

    #[test]
    fn empty_history_returns_continue() {
        let critic = DefaultCritic::new();
        let ctx = CriticContext {
            session_id: Uuid::new_v4(),
            step_count: 0,
            max_steps: 20,
            budget_remaining: make_budget(0.5, 30_000, 100.0, 20),
            initial_budget: make_budget(0.5, 30_000, 100.0, 20),
            recent_observations: vec![],
            progress_history: vec![],
            selected_skills_history: vec![],
            current_belief_hash: "initial".to_string(),
            belief_hash_history: vec![],
        };
        let eval = critic.evaluate(&ctx).unwrap();
        assert_eq!(eval.decision, CriticDecision::Continue);
        assert!(!eval.details.loop_detected);
        assert!(!eval.details.dead_end_detected);
        assert!(!eval.details.contradiction_detected);
        assert!(!eval.details.budget_overrun);
        assert_eq!(eval.details.consecutive_failures, 0);
        assert_eq!(eval.details.progress_rate, 0.0);
    }

    #[test]
    fn single_observation_no_anomalies() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.recent_observations = vec![ObservationSummary {
            success: true,
            skill_id: "only_one".to_string(),
            latency_ms: 100,
            cost: 1.0,
        }];
        ctx.progress_history = vec![0.5];
        ctx.belief_hash_history = vec!["h1".to_string()];
        ctx.selected_skills_history = vec!["only_one".to_string()];
        let eval = critic.evaluate(&ctx).unwrap();
        assert_eq!(eval.decision, CriticDecision::Continue);
    }

    #[test]
    fn default_critic_default_trait() {
        let critic = DefaultCritic;
        let ctx = make_context();
        let eval = critic.evaluate(&ctx).unwrap();
        assert_eq!(eval.decision, CriticDecision::Continue);
    }

    // --- Confidence ranges ---

    #[test]
    fn continue_confidence_is_reasonable() {
        let critic = DefaultCritic::new();
        let ctx = make_context();
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.confidence > 0.0 && eval.confidence <= 1.0);
    }

    #[test]
    fn stop_confidence_is_high() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.budget_remaining.risk_remaining = 0.01;
        let eval = critic.evaluate(&ctx).unwrap();
        assert_eq!(eval.decision, CriticDecision::Stop);
        assert!(eval.confidence >= 0.9);
    }

    // --- Reason strings ---

    #[test]
    fn reason_mentions_budget_on_overrun() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.budget_remaining.resource_remaining = 1.0;
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.reason.contains("budget"));
    }

    #[test]
    fn reason_mentions_loop_on_loop() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.belief_hash_history = vec![
            "x".to_string(),
            "x".to_string(),
            "x".to_string(),
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.reason.contains("loop"));
    }

    #[test]
    fn reason_mentions_dead_end_on_dead_end() {
        let critic = DefaultCritic::new();
        let mut ctx = make_context();
        ctx.recent_observations = vec![
            ObservationSummary {
                success: false,
                skill_id: "s".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
            ObservationSummary {
                success: false,
                skill_id: "s".to_string(),
                latency_ms: 50,
                cost: 0.5,
            },
        ];
        let eval = critic.evaluate(&ctx).unwrap();
        assert!(eval.reason.contains("dead end"));
    }
}
