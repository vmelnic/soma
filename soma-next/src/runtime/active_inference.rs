use crate::types::belief::BeliefState;
use crate::types::common::Budget;
use crate::types::episode::Episode;
use crate::types::goal::GoalSpec;
use crate::types::session::CandidateScore;
use crate::types::skill::SkillSpec;

use super::session::CandidatePredictor;

const DEFAULT_STEEPNESS: f64 = 6.0;

pub struct ActiveInferencePredictor<'a> {
    inner: &'a dyn CandidatePredictor,
    budget_consumed_ratio: f64,
    steepness: f64,
    pragmatic_weight: f64,
    epistemic_weight: f64,
}

impl<'a> ActiveInferencePredictor<'a> {
    pub fn new(
        inner: &'a dyn CandidatePredictor,
        pragmatic_weight: f64,
        epistemic_weight: f64,
        goal: &GoalSpec,
        budget: &Budget,
    ) -> Self {
        Self {
            inner,
            budget_consumed_ratio: Self::budget_consumed_ratio(goal, budget),
            steepness: DEFAULT_STEEPNESS,
            pragmatic_weight,
            epistemic_weight,
        }
    }

    pub fn compute_precision(ratio: f64, steepness: f64) -> f64 {
        let raw = 1.0 / (1.0 + (-steepness * (ratio - 0.5)).exp());
        raw.clamp(0.05, 0.95)
    }

    pub fn budget_consumed_ratio(goal: &GoalSpec, budget: &Budget) -> f64 {
        let mut dimensions = 0;
        let mut total = 0.0;

        if goal.resource_budget > 0.0 {
            total += 1.0 - (budget.resource_remaining / goal.resource_budget).clamp(0.0, 1.0);
            dimensions += 1;
        }
        if goal.latency_budget_ms > 0 {
            total += 1.0
                - (budget.latency_remaining_ms as f64 / goal.latency_budget_ms as f64)
                    .clamp(0.0, 1.0);
            dimensions += 1;
        }
        if goal.risk_budget > 0.0 {
            total += 1.0 - (budget.risk_remaining / goal.risk_budget).clamp(0.0, 1.0);
            dimensions += 1;
        }

        if dimensions == 0 {
            0.0
        } else {
            total / dimensions as f64
        }
    }

    pub fn goal_proximity(goal: &GoalSpec, belief: &BeliefState) -> f64 {
        if goal.success_conditions.is_empty() {
            return 0.5;
        }
        let satisfied = goal
            .success_conditions
            .iter()
            .filter(|cond| {
                belief.facts.iter().any(|f| {
                    if let Some(obj) = cond.expression.as_object() {
                        obj.iter().all(|(k, v)| {
                            (k == "subject" && serde_json::Value::String(f.subject.clone()) == *v)
                                || (k == "predicate"
                                    && serde_json::Value::String(f.predicate.clone()) == *v)
                                || (k == &f.predicate && f.value == *v)
                        })
                    } else {
                        false
                    }
                })
            })
            .count();
        satisfied as f64 / goal.success_conditions.len() as f64
    }

    pub fn score(
        &self,
        candidates: &[SkillSpec],
        goal: &GoalSpec,
        belief: &BeliefState,
        episodes: &[Episode],
    ) -> Vec<CandidateScore> {
        let inner_scores = self.inner.score(candidates, goal, belief, episodes);
        let precision = Self::compute_precision(self.budget_consumed_ratio, self.steepness);
        let proximity = Self::goal_proximity(goal, belief);

        inner_scores
            .into_iter()
            .map(|cs| {
                let pragmatic = cs.predicted_success * proximity;
                let epistemic = cs.information_gain;
                let score = self.pragmatic_weight * pragmatic * precision
                    + self.epistemic_weight * epistemic * (1.0 - precision);
                CandidateScore { score, ..cs }
            })
            .collect()
    }

    pub fn predict_top(&self, scored: &[CandidateScore], limit: usize) -> Vec<CandidateScore> {
        let mut sorted = scored.to_vec();
        sorted.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(limit);
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::goal::{ExplorationStrategy, GoalSource, GoalSourceType, Objective, Priority};
    use chrono::Utc;
    use uuid::Uuid;

    struct MockPredictor {
        scores: Vec<CandidateScore>,
    }

    impl CandidatePredictor for MockPredictor {
        fn score(
            &self,
            _candidates: &[SkillSpec],
            _goal: &GoalSpec,
            _belief: &BeliefState,
            _episodes: &[Episode],
        ) -> Vec<CandidateScore> {
            self.scores.clone()
        }

        fn predict_top(&self, scored: &[CandidateScore], limit: usize) -> Vec<CandidateScore> {
            let mut s = scored.to_vec();
            s.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            s.truncate(limit);
            s
        }
    }

    fn make_goal() -> GoalSpec {
        GoalSpec {
            goal_id: Uuid::new_v4(),
            source: GoalSource {
                source_type: GoalSourceType::User,
                identity: None,
                session_id: None,
                peer_id: None,
            },
            objective: Objective {
                description: "test".to_string(),
                structured: None,
            },
            constraints: vec![],
            success_conditions: vec![],
            risk_budget: 1.0,
            latency_budget_ms: 10000,
            resource_budget: 1.0,
            deadline: None,
            permissions_scope: vec![],
            priority: Priority::Normal,
            max_steps: None,
            exploration: ExplorationStrategy::Greedy,
        }
    }

    fn make_belief() -> BeliefState {
        BeliefState {
            belief_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            resources: vec![],
            facts: vec![],
            uncertainties: vec![],
            provenance: vec![],
            active_bindings: vec![],
            world_hash: String::new(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_precision_zero_budget() {
        let p = ActiveInferencePredictor::compute_precision(0.0, DEFAULT_STEEPNESS);
        assert!(p < 0.10, "expected low precision at start, got {p}");
    }

    #[test]
    fn test_precision_full_budget() {
        let p = ActiveInferencePredictor::compute_precision(1.0, DEFAULT_STEEPNESS);
        assert!(p > 0.90, "expected high precision at end, got {p}");
    }

    #[test]
    fn test_precision_half() {
        let p = ActiveInferencePredictor::compute_precision(0.5, DEFAULT_STEEPNESS);
        assert!(
            (p - 0.5).abs() < 0.01,
            "expected ~0.5 at midpoint, got {p}"
        );
    }

    #[test]
    fn test_explore_early_exploit_late() {
        let mock = MockPredictor {
            scores: vec![
                CandidateScore {
                    skill_id: "exploiter".into(),
                    score: 0.0,
                    predicted_success: 0.9,
                    predicted_cost: 0.1,
                    predicted_latency_ms: 10,
                    information_gain: 0.1,
                },
                CandidateScore {
                    skill_id: "explorer".into(),
                    score: 0.0,
                    predicted_success: 0.2,
                    predicted_cost: 0.1,
                    predicted_latency_ms: 10,
                    information_gain: 0.9,
                },
            ],
        };

        let goal = make_goal();
        let belief = make_belief();

        // Early: budget barely consumed → epistemic dominates
        let early_budget = Budget {
            risk_remaining: 0.95,
            latency_remaining_ms: 9500,
            resource_remaining: 0.95,
            steps_remaining: 100,
        };
        let aip = ActiveInferencePredictor::new(&mock, 1.0, 1.0, &goal, &early_budget);
        let scores = aip.score(&[], &goal, &belief, &[]);
        let top = aip.predict_top(&scores, 2);
        assert_eq!(
            top[0].skill_id, "explorer",
            "early: explorer should rank first"
        );

        // Late: budget mostly consumed → pragmatic dominates
        let late_budget = Budget {
            risk_remaining: 0.05,
            latency_remaining_ms: 500,
            resource_remaining: 0.05,
            steps_remaining: 2,
        };
        let aip = ActiveInferencePredictor::new(&mock, 1.0, 1.0, &goal, &late_budget);
        let scores = aip.score(&[], &goal, &belief, &[]);
        let top = aip.predict_top(&scores, 2);
        assert_eq!(
            top[0].skill_id, "exploiter",
            "late: exploiter should rank first"
        );
    }

    #[test]
    fn test_strategy_serde_roundtrip() {
        let strategy = ExplorationStrategy::ActiveInference {
            pragmatic_weight: 1.5,
            epistemic_weight: 0.8,
        };
        let json = serde_json::to_string(&strategy).unwrap();
        let back: ExplorationStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(strategy, back);
    }

    #[test]
    fn test_strategy_default_weights() {
        let json = r#"{"kind": "active_inference"}"#;
        let strategy: ExplorationStrategy = serde_json::from_str(json).unwrap();
        match strategy {
            ExplorationStrategy::ActiveInference {
                pragmatic_weight,
                epistemic_weight,
            } => {
                assert!((pragmatic_weight - 1.0).abs() < f64::EPSILON);
                assert!((epistemic_weight - 1.0).abs() < f64::EPSILON);
            }
            _ => panic!("expected ActiveInference"),
        }
    }

    #[test]
    fn test_budget_consumed_ratio() {
        let goal = make_goal();
        let budget = Budget {
            risk_remaining: 0.5,
            latency_remaining_ms: 5000,
            resource_remaining: 0.5,
            steps_remaining: 50,
        };
        let ratio = ActiveInferencePredictor::budget_consumed_ratio(&goal, &budget);
        assert!(
            (ratio - 0.5).abs() < 0.01,
            "expected ~0.5, got {ratio}"
        );
    }
}
