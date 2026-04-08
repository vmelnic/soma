use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::errors::{Result, SomaError};
use crate::types::common::Budget;

// --- Delegation policy filter ---

/// Trait that the Selector uses to check whether a delegated candidate is
/// allowed by policy. Callers provide an implementation that wraps the full
/// PolicyRuntime + PolicyContext for the current session. The selector stays
/// decoupled from policy internals — it only asks "is this delegation allowed?"
///
/// If no DelegationPolicyFilter is configured, all delegated candidates are
/// kept (permissive default).
pub trait DelegationPolicyFilter: Send + Sync {
    /// Return true if delegating `action` to `peer_id` is allowed.
    /// When `peer_id` is None (delegation target unknown at selection time),
    /// implementations should return true to defer the check to execution time.
    fn is_delegation_allowed(&self, action: &str, peer_id: Option<&str>) -> bool;
}

// --- Scoring weights ---

/// Default weights for the multi-objective scoring function.
/// Sum to 1.0. Tunable per-deployment via ScoringWeights.
const DEFAULT_SUCCESS_WEIGHT: f64 = 0.40;
const DEFAULT_COST_WEIGHT: f64 = 0.20;
const DEFAULT_LATENCY_WEIGHT: f64 = 0.15;
const DEFAULT_RISK_WEIGHT: f64 = 0.15;
const DEFAULT_INFO_GAIN_WEIGHT: f64 = 0.10;

/// Default confidence threshold for routine matching.
/// Routines with confidence below this are not promoted to the routine tier.
const DEFAULT_ROUTINE_CONFIDENCE_THRESHOLD: f64 = 0.8;

/// Hierarchy tier bonus: routines are preferred over schemas, schemas over composites, etc.
/// Applied additively after the weighted score, so a routine with a decent score
/// beats a primitive with a perfect score.
const ROUTINE_TIER_BONUS: f64 = 0.30;
const SCHEMA_TIER_BONUS: f64 = 0.20;
const COMPOSITE_TIER_BONUS: f64 = 0.10;
const PRIMITIVE_TIER_BONUS: f64 = 0.00;

// --- CandidateType ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateType {
    Routine,
    Schema,
    CompositeSkill,
    PrimitiveSkill,
}

impl CandidateType {
    /// Tier bonus for hierarchical preference.
    fn tier_bonus(self) -> f64 {
        match self {
            CandidateType::Routine => ROUTINE_TIER_BONUS,
            CandidateType::Schema => SCHEMA_TIER_BONUS,
            CandidateType::CompositeSkill => COMPOSITE_TIER_BONUS,
            CandidateType::PrimitiveSkill => PRIMITIVE_TIER_BONUS,
        }
    }
}

// --- Candidate ---

/// A selection candidate produced by the Selector.
/// Carries predicted outcomes and a composite score used for ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub candidate_id: String,
    pub candidate_type: CandidateType,
    pub skill_id: String,
    pub score: f64,
    pub predicted_success: f64,
    pub predicted_cost: f64,
    pub predicted_latency_ms: u64,
    pub information_gain: f64,
    pub risk: f64,
    /// True when this candidate represents a delegated skill (execution routed
    /// to a remote peer). Delegated candidates are subject to policy filtering
    /// during candidate generation.
    #[serde(default)]
    pub is_delegated: bool,
    /// The target peer for delegated skills. Used by the policy filter to
    /// evaluate whether the delegation is allowed.
    #[serde(default)]
    pub peer_id: Option<String>,
    /// Whether this candidate's skill supports reversal (rollback).
    /// Used as a tiebreaker during ranking: when scores are equal, reversible
    /// candidates are preferred over irreversible ones.
    #[serde(default)]
    pub reversible: bool,
}

// --- SelectionContext ---

/// Everything the Selector needs to generate and rank candidates.
/// The goal is carried as serde_json::Value so the selector stays decoupled
/// from GoalSpec parsing (the Goal Runtime owns that).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionContext {
    pub goal: serde_json::Value,
    pub belief_summary: serde_json::Value,
    pub available_skills: Vec<String>,
    pub available_routines: Vec<String>,
    pub available_schemas: Vec<String>,
    pub budget_remaining: Budget,
    pub step_count: u32,
    /// Per-routine confidence thresholds from SkillSpec.confidence_threshold.
    /// When a routine's per-skill threshold is higher than the selector-wide
    /// routine_confidence_threshold, the per-skill value takes precedence.
    #[serde(default)]
    pub routine_confidence_thresholds: HashMap<String, f64>,
    /// Maps skill_id -> peer_id for delegated skills. Skills in this map are
    /// flagged as delegated and subjected to policy filtering during candidate
    /// generation.
    #[serde(default)]
    pub delegated_skills: HashMap<String, String>,
    /// Maps skill_id -> whether the skill supports reversal (rollback).
    /// Skills with rollback_support != Irreversible are mapped to true.
    /// Used by the selector as a tiebreaker when candidates have equal scores.
    #[serde(default)]
    pub skill_reversibility: HashMap<String, bool>,
}

// --- ScoringWeights ---

/// Tunable weights for the multi-objective scoring function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub success: f64,
    pub cost: f64,
    pub latency: f64,
    pub risk: f64,
    pub info_gain: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            success: DEFAULT_SUCCESS_WEIGHT,
            cost: DEFAULT_COST_WEIGHT,
            latency: DEFAULT_LATENCY_WEIGHT,
            risk: DEFAULT_RISK_WEIGHT,
            info_gain: DEFAULT_INFO_GAIN_WEIGHT,
        }
    }
}

// --- Selector trait ---

/// The Selector: candidate generation, ranking, and selection.
///
/// Selection is NOT a flat global action classifier. It proceeds hierarchically:
///   1. Routines  — compiled habitual shortcuts (highest confidence, lowest deliberation)
///   2. Schemas   — reusable abstract control structures
///   3. Composite skills — subgoal decomposition
///   4. Primitive skills — direct port invocations (fallback)
///
/// Each tier is checked in order. Within a tier, candidates are ranked by a
/// weighted combination of predicted_success, cost, latency, risk, and information_gain.
pub trait Selector: Send + Sync {
    /// Generate all viable candidates from the available routines, schemas, and skills.
    /// Returns candidates with predicted outcomes but without final scores
    /// (scores are assigned during ranking).
    fn generate_candidates(&self, context: &SelectionContext) -> Result<Vec<Candidate>>;

    /// Rank candidates in place by computing composite scores.
    /// After ranking, candidates are sorted descending by score.
    fn rank_candidates(&self, candidates: &mut Vec<Candidate>, context: &SelectionContext);

    /// Pick the top candidate. Returns NoCandidates if the list is empty.
    fn select(&self, candidates: &[Candidate]) -> Result<Candidate>;
}

// --- DefaultSelector ---

/// Default hierarchical selector.
///
/// Generates candidates per tier, scores them with a weighted objective function
/// plus a tier bonus, then returns the highest-scoring candidate.
///
/// When a `DelegationPolicyFilter` is configured, delegated candidates that
/// would be denied by policy are removed during candidate generation. Without
/// a filter, all delegated candidates are kept (permissive default) and
/// delegation policy is deferred to execution time.
pub struct DefaultSelector {
    pub weights: ScoringWeights,
    pub routine_confidence_threshold: f64,
    delegation_policy: Option<Arc<dyn DelegationPolicyFilter>>,
}

impl DefaultSelector {
    pub fn new() -> Self {
        Self {
            weights: ScoringWeights::default(),
            routine_confidence_threshold: DEFAULT_ROUTINE_CONFIDENCE_THRESHOLD,
            delegation_policy: None,
        }
    }

    pub fn with_weights(weights: ScoringWeights) -> Self {
        Self {
            weights,
            routine_confidence_threshold: DEFAULT_ROUTINE_CONFIDENCE_THRESHOLD,
            delegation_policy: None,
        }
    }

    /// Attach a delegation policy filter. When set, delegated candidates that
    /// would be denied are removed during candidate generation rather than
    /// waiting for execution-time rejection.
    pub fn with_delegation_policy(mut self, filter: Arc<dyn DelegationPolicyFilter>) -> Self {
        self.delegation_policy = Some(filter);
        self
    }

    /// Set the delegation policy filter after construction.
    pub fn set_delegation_policy(&mut self, filter: Arc<dyn DelegationPolicyFilter>) {
        self.delegation_policy = Some(filter);
    }

    /// Filter a list of candidates through the delegation policy. Delegated
    /// candidates that would be denied are removed; non-delegated candidates
    /// pass through unchanged.
    ///
    /// This method is useful when the caller manages policy separately from
    /// the selector and wants to apply filtering as a post-processing step
    /// after `generate_candidates`.
    pub fn filter_by_policy(
        candidates: Vec<Candidate>,
        policy: &dyn DelegationPolicyFilter,
    ) -> Vec<Candidate> {
        candidates
            .into_iter()
            .filter(|c| {
                if !c.is_delegated {
                    return true;
                }
                policy.is_delegation_allowed(&c.skill_id, c.peer_id.as_deref())
            })
            .collect()
    }

    /// Compute the weighted score for a candidate (before tier bonus).
    /// All input values are expected in [0, 1] except predicted_latency_ms which is
    /// normalized against the budget's latency_remaining_ms.
    fn compute_base_score(&self, candidate: &Candidate, context: &SelectionContext) -> f64 {
        let w = &self.weights;

        // Normalize latency: 1.0 = instant, 0.0 = uses entire remaining budget
        let latency_norm = if context.budget_remaining.latency_remaining_ms > 0 {
            1.0 - (candidate.predicted_latency_ms as f64
                / context.budget_remaining.latency_remaining_ms as f64)
                .clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Normalize cost: 1.0 = free, 0.0 = uses entire remaining resource budget
        let cost_norm = if context.budget_remaining.resource_remaining > 0.0 {
            1.0 - (candidate.predicted_cost / context.budget_remaining.resource_remaining)
                .clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Risk: 1.0 = no risk, 0.0 = maximal risk
        let risk_norm = 1.0 - candidate.risk.clamp(0.0, 1.0);

        w.success * candidate.predicted_success.clamp(0.0, 1.0)
            + w.cost * cost_norm
            + w.latency * latency_norm
            + w.risk * risk_norm
            + w.info_gain * candidate.information_gain.clamp(0.0, 1.0)
    }

    /// Check whether a candidate fits within the remaining budget.
    fn fits_budget(candidate: &Candidate, budget: &Budget) -> bool {
        if candidate.predicted_cost > budget.resource_remaining {
            return false;
        }
        if candidate.predicted_latency_ms > budget.latency_remaining_ms {
            return false;
        }
        if candidate.risk > budget.risk_remaining {
            return false;
        }
        true
    }
}

impl Default for DefaultSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DefaultSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultSelector")
            .field("weights", &self.weights)
            .field("routine_confidence_threshold", &self.routine_confidence_threshold)
            .field("has_delegation_policy", &self.delegation_policy.is_some())
            .finish()
    }
}

impl Selector for DefaultSelector {
    fn generate_candidates(&self, context: &SelectionContext) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();
        let mut id_counter: u64 = 0;

        // Tier 1: Routines — only include if predicted_success meets the
        // confidence threshold. Use the per-skill threshold from SkillSpec when
        // it is higher than the selector-wide routine_confidence_threshold.
        for routine_id in &context.available_routines {
            id_counter += 1;
            let delegation_info = context.delegated_skills.get(routine_id);
            let reversible = context
                .skill_reversibility
                .get(routine_id)
                .copied()
                .unwrap_or(false);
            let candidate = Candidate {
                candidate_id: format!("cand-{id_counter}"),
                candidate_type: CandidateType::Routine,
                skill_id: routine_id.clone(),
                score: 0.0, // assigned during ranking
                predicted_success: 0.0,
                predicted_cost: 0.0,
                predicted_latency_ms: 0,
                information_gain: 0.0,
                risk: 0.0,
                is_delegated: delegation_info.is_some(),
                peer_id: delegation_info.cloned(),
                reversible,
            };

            let per_skill = context
                .routine_confidence_thresholds
                .get(routine_id)
                .copied()
                .unwrap_or(0.0);
            let effective_threshold = self.routine_confidence_threshold.max(per_skill);

            if candidate.predicted_success >= effective_threshold {
                candidates.push(candidate);
            }
        }

        // Tier 2: Schemas
        for schema_id in &context.available_schemas {
            id_counter += 1;
            let delegation_info = context.delegated_skills.get(schema_id);
            let reversible = context
                .skill_reversibility
                .get(schema_id)
                .copied()
                .unwrap_or(false);
            let candidate = Candidate {
                candidate_id: format!("cand-{id_counter}"),
                candidate_type: CandidateType::Schema,
                skill_id: schema_id.clone(),
                score: 0.0,
                predicted_success: 0.0,
                predicted_cost: 0.0,
                predicted_latency_ms: 0,
                information_gain: 0.0,
                risk: 0.0,
                is_delegated: delegation_info.is_some(),
                peer_id: delegation_info.cloned(),
                reversible,
            };
            candidates.push(candidate);
        }

        // Tier 3 & 4: Skills (composite vs primitive determined by skill_id convention)
        // In the default implementation, skills whose id contains "composite" or "::"
        // are classified as CompositeSkill; everything else is PrimitiveSkill.
        // A real implementation would look up SkillSpec.kind from the skill registry.
        for skill_id in &context.available_skills {
            id_counter += 1;
            let kind = if skill_id.contains("composite") || skill_id.contains("::") {
                CandidateType::CompositeSkill
            } else {
                CandidateType::PrimitiveSkill
            };
            let delegation_info = context.delegated_skills.get(skill_id);
            let reversible = context
                .skill_reversibility
                .get(skill_id)
                .copied()
                .unwrap_or(false);
            let candidate = Candidate {
                candidate_id: format!("cand-{id_counter}"),
                candidate_type: kind,
                skill_id: skill_id.clone(),
                score: 0.0,
                predicted_success: 0.0,
                predicted_cost: 0.0,
                predicted_latency_ms: 0,
                information_gain: 0.0,
                risk: 0.0,
                is_delegated: delegation_info.is_some(),
                peer_id: delegation_info.cloned(),
                reversible,
            };
            candidates.push(candidate);
        }

        // Apply delegation policy filtering: remove delegated candidates that
        // would be denied. If no policy filter is configured, all candidates
        // pass through (permissive default — policy enforcement deferred to
        // execution time).
        if let Some(ref policy) = self.delegation_policy {
            candidates = Self::filter_by_policy(candidates, policy.as_ref());
        }

        Ok(candidates)
    }

    fn rank_candidates(&self, candidates: &mut Vec<Candidate>, context: &SelectionContext) {
        // Remove candidates that exceed the remaining budget
        candidates.retain(|c| Self::fits_budget(c, &context.budget_remaining));

        // Compute score = base_score + tier_bonus
        for candidate in candidates.iter_mut() {
            let base = self.compute_base_score(candidate, context);
            candidate.score = base + candidate.candidate_type.tier_bonus();
        }

        // Sort descending by score. When scores are equal, prefer reversible
        // candidates (skills that support rollback) over irreversible ones.
        candidates.sort_by(|a, b| {
            let score_cmp = b
                .score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal);
            if score_cmp != std::cmp::Ordering::Equal {
                return score_cmp;
            }
            // Tiebreaker: reversible (true) before irreversible (false).
            // Reverse comparison so true sorts before false.
            b.reversible.cmp(&a.reversible)
        });
    }

    fn select(&self, candidates: &[Candidate]) -> Result<Candidate> {
        candidates
            .first()
            .cloned()
            .ok_or(SomaError::NoCandidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_budget() -> Budget {
        Budget {
            risk_remaining: 0.5,
            latency_remaining_ms: 10_000,
            resource_remaining: 100.0,
            steps_remaining: 10,
        }
    }

    fn make_context() -> SelectionContext {
        SelectionContext {
            goal: serde_json::json!({"description": "test goal"}),
            belief_summary: serde_json::json!({}),
            available_skills: vec![
                "fs.list_dir".to_string(),
                "fs::composite::backup".to_string(),
            ],
            available_routines: vec!["routine.list_tmp".to_string()],
            available_schemas: vec!["schema.file_ops".to_string()],
            budget_remaining: make_budget(),
            step_count: 0,
            routine_confidence_thresholds: HashMap::new(),
            delegated_skills: HashMap::new(),
            skill_reversibility: HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_candidate(
        id: &str,
        ctype: CandidateType,
        skill: &str,
        success: f64,
        cost: f64,
        latency_ms: u64,
        risk: f64,
        info_gain: f64,
    ) -> Candidate {
        Candidate {
            candidate_id: id.to_string(),
            candidate_type: ctype,
            skill_id: skill.to_string(),
            score: 0.0,
            predicted_success: success,
            predicted_cost: cost,
            predicted_latency_ms: latency_ms,
            information_gain: info_gain,
            risk,
            is_delegated: false,
            peer_id: None,
            reversible: false,
        }
    }

    // --- generate_candidates ---

    #[test]
    fn generate_candidates_produces_all_tiers() {
        let mut selector = DefaultSelector::new();
        // Set threshold to 0.0 so placeholder routines (predicted_success=0.0) pass
        selector.routine_confidence_threshold = 0.0;
        let ctx = make_context();
        let candidates = selector.generate_candidates(&ctx).unwrap();

        // 1 routine + 1 schema + 2 skills = 4
        assert_eq!(candidates.len(), 4);

        let types: Vec<CandidateType> = candidates.iter().map(|c| c.candidate_type).collect();
        assert!(types.contains(&CandidateType::Routine));
        assert!(types.contains(&CandidateType::Schema));
        assert!(types.contains(&CandidateType::PrimitiveSkill));
        assert!(types.contains(&CandidateType::CompositeSkill));
    }

    #[test]
    fn generate_candidates_empty_context() {
        let selector = DefaultSelector::new();
        let ctx = SelectionContext {
            goal: serde_json::json!({}),
            belief_summary: serde_json::json!({}),
            available_skills: vec![],
            available_routines: vec![],
            available_schemas: vec![],
            budget_remaining: make_budget(),
            step_count: 0,
            routine_confidence_thresholds: HashMap::new(),
            delegated_skills: HashMap::new(),
            skill_reversibility: HashMap::new(),
        };
        let candidates = selector.generate_candidates(&ctx).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn generate_candidates_unique_ids() {
        let mut selector = DefaultSelector::new();
        selector.routine_confidence_threshold = 0.0;
        let ctx = make_context();
        let candidates = selector.generate_candidates(&ctx).unwrap();
        let ids: Vec<&str> = candidates.iter().map(|c| c.candidate_id.as_str()).collect();
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len());
    }

    #[test]
    fn generate_candidates_classifies_composite_by_double_colon() {
        let selector = DefaultSelector::new();
        let ctx = SelectionContext {
            goal: serde_json::json!({}),
            belief_summary: serde_json::json!({}),
            available_skills: vec!["net::http::get".to_string()],
            available_routines: vec![],
            available_schemas: vec![],
            budget_remaining: make_budget(),
            step_count: 0,
            routine_confidence_thresholds: HashMap::new(),
            delegated_skills: HashMap::new(),
            skill_reversibility: HashMap::new(),
        };
        let candidates = selector.generate_candidates(&ctx).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].candidate_type, CandidateType::CompositeSkill);
    }

    // --- routine confidence threshold ---

    #[test]
    fn generate_candidates_filters_routines_below_threshold() {
        let selector = DefaultSelector::new(); // threshold = 0.8
        let ctx = make_context(); // routine has predicted_success=0.0 (placeholder)
        let candidates = selector.generate_candidates(&ctx).unwrap();

        // Routine should be filtered out because 0.0 < 0.8
        let routine_count = candidates
            .iter()
            .filter(|c| c.candidate_type == CandidateType::Routine)
            .count();
        assert_eq!(routine_count, 0);

        // Schemas and skills should still be present
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn generate_candidates_zero_threshold_passes_all_routines() {
        let mut selector = DefaultSelector::new();
        selector.routine_confidence_threshold = 0.0;
        let ctx = make_context();
        let candidates = selector.generate_candidates(&ctx).unwrap();

        let routine_count = candidates
            .iter()
            .filter(|c| c.candidate_type == CandidateType::Routine)
            .count();
        assert_eq!(routine_count, 1);
    }

    #[test]
    fn generate_candidates_per_skill_threshold_overrides_when_higher() {
        let mut selector = DefaultSelector::new();
        // Selector-wide threshold is low, but the per-skill threshold is high
        selector.routine_confidence_threshold = 0.3;
        let mut thresholds = HashMap::new();
        thresholds.insert("routine.strict".to_string(), 0.95);
        thresholds.insert("routine.lenient".to_string(), 0.1);

        let ctx = SelectionContext {
            goal: serde_json::json!({}),
            belief_summary: serde_json::json!({}),
            available_skills: vec![],
            available_routines: vec![
                "routine.strict".to_string(),
                "routine.lenient".to_string(),
            ],
            available_schemas: vec![],
            budget_remaining: make_budget(),
            step_count: 0,
            routine_confidence_thresholds: thresholds,
            delegated_skills: HashMap::new(),
            skill_reversibility: HashMap::new(),
        };
        let candidates = selector.generate_candidates(&ctx).unwrap();

        // Both routines have predicted_success=0.0:
        // - "routine.strict": effective threshold = max(0.3, 0.95) = 0.95 -> filtered
        // - "routine.lenient": effective threshold = max(0.3, 0.1) = 0.3 -> filtered
        // Neither passes because predicted_success is 0.0
        assert_eq!(candidates.len(), 0);
    }

    #[test]
    fn generate_candidates_per_skill_threshold_does_not_lower_selector_threshold() {
        let mut selector = DefaultSelector::new();
        selector.routine_confidence_threshold = 0.5;
        let mut thresholds = HashMap::new();
        // Per-skill threshold is lower than selector-wide, selector-wide wins
        thresholds.insert("routine.easy".to_string(), 0.1);

        let ctx = SelectionContext {
            goal: serde_json::json!({}),
            belief_summary: serde_json::json!({}),
            available_skills: vec![],
            available_routines: vec!["routine.easy".to_string()],
            available_schemas: vec![],
            budget_remaining: make_budget(),
            step_count: 0,
            routine_confidence_thresholds: thresholds,
            delegated_skills: HashMap::new(),
            skill_reversibility: HashMap::new(),
        };
        let candidates = selector.generate_candidates(&ctx).unwrap();

        // predicted_success=0.0 < max(0.5, 0.1)=0.5, so filtered out
        assert_eq!(candidates.len(), 0);
    }

    // --- rank_candidates ---

    #[test]
    fn rank_candidates_routine_beats_primitive_at_equal_predictions() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let mut candidates = vec![
            make_candidate("prim", CandidateType::PrimitiveSkill, "fs.list_dir", 0.9, 10.0, 100, 0.1, 0.5),
            make_candidate("rout", CandidateType::Routine, "routine.list_tmp", 0.9, 10.0, 100, 0.1, 0.5),
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        assert_eq!(candidates[0].candidate_id, "rout");
        assert_eq!(candidates[1].candidate_id, "prim");
        assert!(candidates[0].score > candidates[1].score);
    }

    #[test]
    fn rank_candidates_schema_beats_composite_at_equal_predictions() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let mut candidates = vec![
            make_candidate("comp", CandidateType::CompositeSkill, "composite.backup", 0.8, 20.0, 500, 0.2, 0.3),
            make_candidate("sch", CandidateType::Schema, "schema.file_ops", 0.8, 20.0, 500, 0.2, 0.3),
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        assert_eq!(candidates[0].candidate_id, "sch");
        assert_eq!(candidates[1].candidate_id, "comp");
    }

    #[test]
    fn rank_candidates_filters_over_budget() {
        let selector = DefaultSelector::new();
        let ctx = SelectionContext {
            budget_remaining: Budget {
                risk_remaining: 0.1,
                latency_remaining_ms: 500,
                resource_remaining: 10.0,
                steps_remaining: 5,
            },
            ..make_context()
        };
        let mut candidates = vec![
            make_candidate("cheap", CandidateType::PrimitiveSkill, "fs.stat", 0.9, 5.0, 100, 0.05, 0.1),
            make_candidate("expensive", CandidateType::PrimitiveSkill, "fs.backup", 0.9, 50.0, 200, 0.05, 0.1),
            make_candidate("slow", CandidateType::PrimitiveSkill, "net.download", 0.9, 5.0, 9000, 0.05, 0.1),
            make_candidate("risky", CandidateType::PrimitiveSkill, "fs.delete_all", 0.9, 5.0, 100, 0.5, 0.1),
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        // Only "cheap" should survive
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].candidate_id, "cheap");
    }

    #[test]
    fn rank_candidates_higher_success_wins_within_same_tier() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let mut candidates = vec![
            make_candidate("low", CandidateType::PrimitiveSkill, "a", 0.3, 10.0, 100, 0.1, 0.1),
            make_candidate("high", CandidateType::PrimitiveSkill, "b", 0.95, 10.0, 100, 0.1, 0.1),
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        assert_eq!(candidates[0].candidate_id, "high");
    }

    #[test]
    fn rank_candidates_lower_cost_wins_within_same_tier() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let mut candidates = vec![
            make_candidate("expensive", CandidateType::PrimitiveSkill, "a", 0.8, 90.0, 100, 0.1, 0.1),
            make_candidate("cheap", CandidateType::PrimitiveSkill, "b", 0.8, 5.0, 100, 0.1, 0.1),
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        assert_eq!(candidates[0].candidate_id, "cheap");
    }

    #[test]
    fn rank_candidates_lower_risk_wins_within_same_tier() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let mut candidates = vec![
            make_candidate("risky", CandidateType::PrimitiveSkill, "a", 0.8, 10.0, 100, 0.45, 0.1),
            make_candidate("safe", CandidateType::PrimitiveSkill, "b", 0.8, 10.0, 100, 0.05, 0.1),
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        assert_eq!(candidates[0].candidate_id, "safe");
    }

    // --- select ---

    #[test]
    fn select_returns_first_candidate() {
        let selector = DefaultSelector::new();
        let candidates = vec![
            make_candidate("best", CandidateType::Routine, "r1", 0.95, 5.0, 50, 0.01, 0.5),
            make_candidate("second", CandidateType::PrimitiveSkill, "s1", 0.5, 10.0, 200, 0.1, 0.1),
        ];
        let selected = selector.select(&candidates).unwrap();
        assert_eq!(selected.candidate_id, "best");
    }

    #[test]
    fn select_empty_returns_no_candidates_error() {
        let selector = DefaultSelector::new();
        let result = selector.select(&[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SomaError::NoCandidates));
    }

    // --- scoring ---

    #[test]
    fn compute_base_score_perfect_candidate() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let candidate = make_candidate(
            "perfect",
            CandidateType::PrimitiveSkill,
            "ideal",
            1.0,   // perfect success
            0.0,   // zero cost
            0,     // zero latency
            0.0,   // zero risk
            1.0,   // max info gain
        );
        let score = selector.compute_base_score(&candidate, &ctx);
        // 0.4*1 + 0.2*1 + 0.15*1 + 0.15*1 + 0.1*1 = 1.0
        assert!((score - 1.0).abs() < 1e-10);
    }

    #[test]
    fn compute_base_score_worst_candidate() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let candidate = make_candidate(
            "worst",
            CandidateType::PrimitiveSkill,
            "terrible",
            0.0,       // zero success
            100.0,     // uses entire resource budget
            10_000,    // uses entire latency budget
            1.0,       // max risk
            0.0,       // zero info gain
        );
        let score = selector.compute_base_score(&candidate, &ctx);
        assert!((score - 0.0).abs() < 1e-10);
    }

    #[test]
    fn compute_base_score_mid_candidate() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let candidate = make_candidate(
            "mid",
            CandidateType::PrimitiveSkill,
            "average",
            0.5,   // half success
            50.0,  // half cost
            5000,  // half latency
            0.5,   // half risk
            0.5,   // half info gain
        );
        let score = selector.compute_base_score(&candidate, &ctx);
        // 0.4*0.5 + 0.2*0.5 + 0.15*0.5 + 0.15*0.5 + 0.1*0.5 = 0.5
        assert!((score - 0.5).abs() < 1e-10);
    }

    // --- tier bonuses ---

    #[test]
    fn tier_bonus_ordering() {
        assert!(CandidateType::Routine.tier_bonus() > CandidateType::Schema.tier_bonus());
        assert!(CandidateType::Schema.tier_bonus() > CandidateType::CompositeSkill.tier_bonus());
        assert!(CandidateType::CompositeSkill.tier_bonus() > CandidateType::PrimitiveSkill.tier_bonus());
        assert_eq!(CandidateType::PrimitiveSkill.tier_bonus(), 0.0);
    }

    // --- custom weights ---

    #[test]
    fn custom_weights_change_ranking() {
        // Create weights that heavily favor information gain
        let weights = ScoringWeights {
            success: 0.1,
            cost: 0.1,
            latency: 0.1,
            risk: 0.1,
            info_gain: 0.6,
        };
        let selector = DefaultSelector::with_weights(weights);
        let ctx = make_context();

        let mut candidates = vec![
            make_candidate("high_success", CandidateType::PrimitiveSkill, "a", 0.95, 10.0, 100, 0.1, 0.1),
            make_candidate("high_info", CandidateType::PrimitiveSkill, "b", 0.5, 10.0, 100, 0.1, 0.95),
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        // With info_gain weight=0.6, the high_info candidate should win
        assert_eq!(candidates[0].candidate_id, "high_info");
    }

    // --- fits_budget ---

    #[test]
    fn fits_budget_within_limits() {
        let budget = make_budget();
        let candidate = make_candidate("ok", CandidateType::PrimitiveSkill, "a", 0.5, 50.0, 5000, 0.3, 0.1);
        assert!(DefaultSelector::fits_budget(&candidate, &budget));
    }

    #[test]
    fn fits_budget_exceeds_cost() {
        let budget = make_budget();
        let candidate = make_candidate("expensive", CandidateType::PrimitiveSkill, "a", 0.5, 200.0, 100, 0.1, 0.1);
        assert!(!DefaultSelector::fits_budget(&candidate, &budget));
    }

    #[test]
    fn fits_budget_exceeds_latency() {
        let budget = make_budget();
        let candidate = make_candidate("slow", CandidateType::PrimitiveSkill, "a", 0.5, 10.0, 20_000, 0.1, 0.1);
        assert!(!DefaultSelector::fits_budget(&candidate, &budget));
    }

    #[test]
    fn fits_budget_exceeds_risk() {
        let budget = make_budget();
        let candidate = make_candidate("risky", CandidateType::PrimitiveSkill, "a", 0.5, 10.0, 100, 0.9, 0.1);
        assert!(!DefaultSelector::fits_budget(&candidate, &budget));
    }

    // --- end-to-end ---

    #[test]
    fn full_lifecycle_generate_rank_select() {
        let selector = DefaultSelector::new();
        let ctx = make_context();

        // Set up predictions via pre-built candidates instead of relying on defaults
        let mut candidates = vec![
            make_candidate("r1", CandidateType::Routine, "routine.list_tmp", 0.9, 5.0, 50, 0.05, 0.3),
            make_candidate("s1", CandidateType::Schema, "schema.file_ops", 0.7, 15.0, 200, 0.1, 0.4),
            make_candidate("c1", CandidateType::CompositeSkill, "fs::composite::backup", 0.6, 30.0, 500, 0.2, 0.2),
            make_candidate("p1", CandidateType::PrimitiveSkill, "fs.list_dir", 0.85, 2.0, 30, 0.02, 0.1),
        ];

        selector.rank_candidates(&mut candidates, &ctx);
        let selected = selector.select(&candidates).unwrap();

        // Routine should win thanks to tier bonus, despite primitive having lower cost
        assert_eq!(selected.candidate_type, CandidateType::Routine);
        assert_eq!(selected.candidate_id, "r1");
    }

    #[test]
    fn full_lifecycle_all_over_budget_returns_error() {
        let selector = DefaultSelector::new();
        let ctx = SelectionContext {
            budget_remaining: Budget {
                risk_remaining: 0.01,
                latency_remaining_ms: 10,
                resource_remaining: 0.5,
                steps_remaining: 1,
            },
            ..make_context()
        };

        let mut candidates = vec![
            make_candidate("a", CandidateType::PrimitiveSkill, "a", 0.9, 10.0, 100, 0.1, 0.1),
            make_candidate("b", CandidateType::Routine, "b", 0.9, 10.0, 100, 0.1, 0.1),
        ];
        selector.rank_candidates(&mut candidates, &ctx);
        let result = selector.select(&candidates);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SomaError::NoCandidates));
    }

    #[test]
    fn default_selector_implements_default_trait() {
        let selector = DefaultSelector::default();
        assert!((selector.weights.success - DEFAULT_SUCCESS_WEIGHT).abs() < 1e-10);
        assert!((selector.routine_confidence_threshold - DEFAULT_ROUTINE_CONFIDENCE_THRESHOLD).abs() < 1e-10);
    }

    #[test]
    fn scoring_weights_default() {
        let w = ScoringWeights::default();
        let sum = w.success + w.cost + w.latency + w.risk + w.info_gain;
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn zero_budget_latency_scores_zero_latency() {
        let selector = DefaultSelector::new();
        let ctx = SelectionContext {
            budget_remaining: Budget {
                risk_remaining: 0.5,
                latency_remaining_ms: 0,
                resource_remaining: 100.0,
                steps_remaining: 10,
            },
            ..make_context()
        };
        let candidate = make_candidate("a", CandidateType::PrimitiveSkill, "a", 1.0, 0.0, 100, 0.0, 1.0);
        let score = selector.compute_base_score(&candidate, &ctx);
        // Latency component should be 0 when budget is 0
        // 0.4*1.0 + 0.2*1.0 + 0.15*0.0 + 0.15*1.0 + 0.1*1.0 = 0.85
        assert!((score - 0.85).abs() < 1e-10);
    }

    #[test]
    fn zero_budget_resource_scores_zero_cost() {
        let selector = DefaultSelector::new();
        let ctx = SelectionContext {
            budget_remaining: Budget {
                risk_remaining: 0.5,
                latency_remaining_ms: 10_000,
                resource_remaining: 0.0,
                steps_remaining: 10,
            },
            ..make_context()
        };
        let candidate = make_candidate("a", CandidateType::PrimitiveSkill, "a", 1.0, 10.0, 0, 0.0, 1.0);
        let score = selector.compute_base_score(&candidate, &ctx);
        // Cost component should be 0 when budget is 0
        // 0.4*1.0 + 0.2*0.0 + 0.15*1.0 + 0.15*1.0 + 0.1*1.0 = 0.80
        assert!((score - 0.80).abs() < 1e-10);
    }

    #[test]
    fn candidate_type_serde_roundtrip() {
        let types = vec![
            CandidateType::Routine,
            CandidateType::Schema,
            CandidateType::CompositeSkill,
            CandidateType::PrimitiveSkill,
        ];
        for ct in types {
            let json = serde_json::to_string(&ct).unwrap();
            let back: CandidateType = serde_json::from_str(&json).unwrap();
            assert_eq!(ct, back);
        }
    }

    #[test]
    fn candidate_serde_roundtrip() {
        let candidate = make_candidate("test", CandidateType::Schema, "s1", 0.7, 15.0, 200, 0.1, 0.4);
        let json = serde_json::to_string(&candidate).unwrap();
        let back: Candidate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.candidate_id, "test");
        assert_eq!(back.candidate_type, CandidateType::Schema);
        assert_eq!(back.skill_id, "s1");
    }

    #[test]
    fn selection_context_serde_roundtrip() {
        let ctx = make_context();
        let json = serde_json::to_string(&ctx).unwrap();
        let back: SelectionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back.available_skills.len(), ctx.available_skills.len());
        assert_eq!(back.step_count, ctx.step_count);
    }

    // --- delegation policy filtering ---

    /// A policy filter that denies all delegations.
    struct DenyAllDelegations;
    impl DelegationPolicyFilter for DenyAllDelegations {
        fn is_delegation_allowed(&self, _action: &str, _peer_id: Option<&str>) -> bool {
            false
        }
    }

    /// A policy filter that allows all delegations.
    struct AllowAllDelegations;
    impl DelegationPolicyFilter for AllowAllDelegations {
        fn is_delegation_allowed(&self, _action: &str, _peer_id: Option<&str>) -> bool {
            true
        }
    }

    /// A policy filter that only allows delegations to a specific peer.
    struct AllowPeerOnly(String);
    impl DelegationPolicyFilter for AllowPeerOnly {
        fn is_delegation_allowed(&self, _action: &str, peer_id: Option<&str>) -> bool {
            peer_id == Some(self.0.as_str())
        }
    }

    #[test]
    fn generate_candidates_marks_delegated_skills() {
        let selector = DefaultSelector::new();
        let mut ctx = make_context();
        ctx.delegated_skills
            .insert("fs.list_dir".to_string(), "peer-1".to_string());

        let candidates = selector.generate_candidates(&ctx).unwrap();
        let delegated: Vec<&Candidate> = candidates.iter().filter(|c| c.is_delegated).collect();
        assert_eq!(delegated.len(), 1);
        assert_eq!(delegated[0].skill_id, "fs.list_dir");
        assert_eq!(delegated[0].peer_id.as_deref(), Some("peer-1"));

        // Non-delegated candidates should have is_delegated=false
        let non_delegated: Vec<&Candidate> = candidates.iter().filter(|c| !c.is_delegated).collect();
        assert!(non_delegated.iter().all(|c| c.peer_id.is_none()));
    }

    #[test]
    fn filter_by_policy_removes_denied_delegated_candidates() {
        let policy = DenyAllDelegations;
        let candidates = vec![
            {
                let mut c = make_candidate("local", CandidateType::PrimitiveSkill, "fs.read", 0.9, 5.0, 50, 0.1, 0.1);
                c.is_delegated = false;
                c
            },
            {
                let mut c = make_candidate("remote", CandidateType::PrimitiveSkill, "net.fetch", 0.8, 10.0, 200, 0.2, 0.3);
                c.is_delegated = true;
                c.peer_id = Some("peer-1".to_string());
                c
            },
        ];

        let filtered = DefaultSelector::filter_by_policy(candidates, &policy);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].candidate_id, "local");
    }

    #[test]
    fn filter_by_policy_keeps_allowed_delegated_candidates() {
        let policy = AllowAllDelegations;
        let candidates = vec![
            {
                let mut c = make_candidate("remote", CandidateType::PrimitiveSkill, "net.fetch", 0.8, 10.0, 200, 0.2, 0.3);
                c.is_delegated = true;
                c.peer_id = Some("peer-1".to_string());
                c
            },
        ];

        let filtered = DefaultSelector::filter_by_policy(candidates, &policy);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].candidate_id, "remote");
    }

    #[test]
    fn filter_by_policy_selective_peer() {
        let policy = AllowPeerOnly("trusted-peer".to_string());
        let candidates = vec![
            {
                let mut c = make_candidate("good", CandidateType::PrimitiveSkill, "a", 0.9, 5.0, 50, 0.1, 0.1);
                c.is_delegated = true;
                c.peer_id = Some("trusted-peer".to_string());
                c
            },
            {
                let mut c = make_candidate("bad", CandidateType::PrimitiveSkill, "b", 0.9, 5.0, 50, 0.1, 0.1);
                c.is_delegated = true;
                c.peer_id = Some("untrusted-peer".to_string());
                c
            },
            make_candidate("local", CandidateType::PrimitiveSkill, "c", 0.9, 5.0, 50, 0.1, 0.1),
        ];

        let filtered = DefaultSelector::filter_by_policy(candidates, &policy);
        assert_eq!(filtered.len(), 2);
        let ids: Vec<&str> = filtered.iter().map(|c| c.candidate_id.as_str()).collect();
        assert!(ids.contains(&"good"));
        assert!(ids.contains(&"local"));
        assert!(!ids.contains(&"bad"));
    }

    #[test]
    fn generate_candidates_with_deny_policy_filters_delegated() {
        let selector = DefaultSelector::new()
            .with_delegation_policy(Arc::new(DenyAllDelegations));

        let mut ctx = make_context();
        ctx.delegated_skills
            .insert("fs.list_dir".to_string(), "peer-1".to_string());

        let candidates = selector.generate_candidates(&ctx).unwrap();

        // The delegated skill should be removed; non-delegated should remain
        assert!(candidates.iter().all(|c| !c.is_delegated));
        assert!(!candidates.iter().any(|c| c.skill_id == "fs.list_dir"));
    }

    #[test]
    fn generate_candidates_without_policy_keeps_all_delegated() {
        let selector = DefaultSelector::new();

        let mut ctx = make_context();
        ctx.delegated_skills
            .insert("fs.list_dir".to_string(), "peer-1".to_string());

        let candidates = selector.generate_candidates(&ctx).unwrap();

        // Without a policy filter, delegated candidates are kept (permissive default)
        assert!(candidates.iter().any(|c| c.skill_id == "fs.list_dir" && c.is_delegated));
    }

    #[test]
    fn set_delegation_policy_after_construction() {
        let mut selector = DefaultSelector::new();
        selector.set_delegation_policy(Arc::new(DenyAllDelegations));

        let mut ctx = make_context();
        ctx.delegated_skills
            .insert("fs.list_dir".to_string(), "peer-1".to_string());

        let candidates = selector.generate_candidates(&ctx).unwrap();
        assert!(!candidates.iter().any(|c| c.skill_id == "fs.list_dir"));
    }

    #[test]
    fn filter_by_policy_with_no_peer_id_defers_check() {
        // When peer_id is None, delegation target is unknown at selection time.
        // AllowPeerOnly requires a specific peer, so None should not match.
        let policy = AllowPeerOnly("specific-peer".to_string());
        let candidates = vec![{
            let mut c = make_candidate("unknown_peer", CandidateType::PrimitiveSkill, "a", 0.9, 5.0, 50, 0.1, 0.1);
            c.is_delegated = true;
            c.peer_id = None;
            c
        }];

        let filtered = DefaultSelector::filter_by_policy(candidates, &policy);
        // peer_id is None, so is_delegation_allowed("a", None) is called.
        // AllowPeerOnly returns false for None, so it's filtered out.
        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn candidate_delegated_fields_serde_roundtrip() {
        let mut candidate = make_candidate("del", CandidateType::PrimitiveSkill, "net.call", 0.7, 10.0, 200, 0.1, 0.3);
        candidate.is_delegated = true;
        candidate.peer_id = Some("peer-42".to_string());

        let json = serde_json::to_string(&candidate).unwrap();
        let back: Candidate = serde_json::from_str(&json).unwrap();
        assert!(back.is_delegated);
        assert_eq!(back.peer_id.as_deref(), Some("peer-42"));
    }

    #[test]
    fn candidate_delegated_defaults_on_deserialize() {
        // Legacy JSON without is_delegated/peer_id should default gracefully
        let json = r#"{
            "candidate_id": "old",
            "candidate_type": "primitive_skill",
            "skill_id": "fs.read",
            "score": 0.5,
            "predicted_success": 0.9,
            "predicted_cost": 5.0,
            "predicted_latency_ms": 100,
            "information_gain": 0.1,
            "risk": 0.05
        }"#;
        let candidate: Candidate = serde_json::from_str(json).unwrap();
        assert!(!candidate.is_delegated);
        assert!(candidate.peer_id.is_none());
        assert!(!candidate.reversible);
    }

    // --- reversibility preference ---

    #[test]
    fn rank_candidates_reversible_preferred_at_equal_score() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let mut candidates = vec![
            {
                let mut c = make_candidate(
                    "irreversible",
                    CandidateType::PrimitiveSkill,
                    "fs.delete",
                    0.8, 10.0, 100, 0.1, 0.5,
                );
                c.reversible = false;
                c
            },
            {
                let mut c = make_candidate(
                    "reversible",
                    CandidateType::PrimitiveSkill,
                    "fs.move",
                    0.8, 10.0, 100, 0.1, 0.5,
                );
                c.reversible = true;
                c
            },
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        // Both have equal scores; reversible should rank first
        assert_eq!(candidates[0].candidate_id, "reversible");
        assert_eq!(candidates[1].candidate_id, "irreversible");
        assert!((candidates[0].score - candidates[1].score).abs() < 1e-10);
    }

    #[test]
    fn rank_candidates_higher_score_beats_reversibility() {
        let selector = DefaultSelector::new();
        let ctx = make_context();
        let mut candidates = vec![
            {
                let mut c = make_candidate(
                    "reversible_weak",
                    CandidateType::PrimitiveSkill,
                    "fs.move",
                    0.5, 10.0, 100, 0.1, 0.1,
                );
                c.reversible = true;
                c
            },
            {
                let mut c = make_candidate(
                    "irreversible_strong",
                    CandidateType::PrimitiveSkill,
                    "fs.delete",
                    0.95, 10.0, 100, 0.1, 0.1,
                );
                c.reversible = false;
                c
            },
        ];
        selector.rank_candidates(&mut candidates, &ctx);

        // Higher score wins despite being irreversible
        assert_eq!(candidates[0].candidate_id, "irreversible_strong");
    }

    #[test]
    fn generate_candidates_sets_reversible_from_context() {
        let mut selector = DefaultSelector::new();
        selector.routine_confidence_threshold = 0.0;

        let mut ctx = make_context();
        ctx.skill_reversibility
            .insert("fs.list_dir".to_string(), true);
        ctx.skill_reversibility
            .insert("routine.list_tmp".to_string(), false);

        let candidates = selector.generate_candidates(&ctx).unwrap();

        let fs_list = candidates.iter().find(|c| c.skill_id == "fs.list_dir").unwrap();
        assert!(fs_list.reversible);

        let routine = candidates.iter().find(|c| c.skill_id == "routine.list_tmp").unwrap();
        assert!(!routine.reversible);

        // Skills not in the map default to false
        let composite = candidates
            .iter()
            .find(|c| c.skill_id == "fs::composite::backup")
            .unwrap();
        assert!(!composite.reversible);
    }

    #[test]
    fn candidate_reversible_serde_roundtrip() {
        let mut candidate = make_candidate("rev", CandidateType::PrimitiveSkill, "fs.move", 0.8, 10.0, 100, 0.1, 0.3);
        candidate.reversible = true;

        let json = serde_json::to_string(&candidate).unwrap();
        let back: Candidate = serde_json::from_str(&json).unwrap();
        assert!(back.reversible);
    }
}
