use crate::errors::{Result, SomaError};
use crate::types::common::{EffectDescriptor, Precondition};
use crate::types::episode::Episode;
use crate::types::routine::{Routine, RoutineOrigin};
use crate::types::schema::Schema;

/// Why a routine is being invalidated. Each variant triggers different
/// matching logic in `invalidate_by_condition`.
#[derive(Debug, Clone, PartialEq)]
pub enum InvalidationReason {
    /// A resource's schema changed — invalidate routines that reference the resource.
    ResourceSchemaChanged { resource_id: String },
    /// Runtime evidence shows preconditions no longer hold — invalidate all routines.
    PreconditionsNoLongerHold,
    /// Global or pack-level policy changed — conservative: invalidate all routines.
    PolicyChanged,
    /// A pack version bump removed skills — invalidate routines whose skill path
    /// contains any of the removed skill FQNs.
    PackVersionBreak { removed_skills: Vec<String> },
    /// Confidence dropped below a threshold — invalidate routines below that threshold.
    ConfidenceDropped { threshold: f64 },
}

/// RoutineStore — storage and retrieval of compiled habitual shortcuts.
/// Routines are high-confidence, bounded, deterministic under declared conditions.
/// They bypass deeper deliberation when match confidence and policy allow.
pub trait RoutineStore {
    /// Register a routine (from a pack, schema compilation, or peer transfer).
    fn register(&mut self, routine: Routine) -> Result<()>;

    /// Find routines whose match_conditions and guard_conditions are satisfied.
    fn find_matching(&self, context: &serde_json::Value) -> Vec<&Routine>;

    /// Get a single routine by ID.
    fn get(&self, routine_id: &str) -> Option<&Routine>;

    /// Attempt to compile a routine from a stable schema and supporting episodes.
    /// Returns None if the schema's confidence is too low or episodes are insufficient.
    fn compile_from_schema(&self, schema: &Schema, episodes: &[&Episode]) -> Option<Routine>;

    /// Invalidate a routine (e.g., after it produces unexpected results).
    fn invalidate(&mut self, routine_id: &str) -> Result<()>;

    /// Invalidate all routines matching the given condition. Returns the IDs
    /// of routines that were removed.
    fn invalidate_by_condition(&mut self, reason: &InvalidationReason) -> Vec<String>;

    /// List all registered routines.
    fn list_all(&self) -> Vec<&Routine>;

    /// Set the `autonomous` flag on a routine. Returns Ok(true) if the
    /// routine was found and updated, Ok(false) if not found.
    fn set_autonomous(&mut self, routine_id: &str, autonomous: bool) -> Result<bool>;
}

/// Default in-memory routine store backed by Vec<Routine>.
pub struct DefaultRoutineStore {
    routines: Vec<Routine>,
}

impl DefaultRoutineStore {
    pub fn new() -> Self {
        Self {
            routines: Vec::new(),
        }
    }
}

impl Default for DefaultRoutineStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a Precondition is satisfied by the given context.
/// Same matching logic as in schemas.rs for consistency.
///
/// For `world_state` conditions: if the expression value for a key is
/// `true` (boolean), the check passes when the key exists in the context
/// regardless of its actual value (existence-only match).
fn precondition_matches(precondition: &Precondition, context: &serde_json::Value) -> bool {
    match &precondition.expression {
        serde_json::Value::Object(expr_map) => {
            let ctx_map = match context.as_object() {
                Some(m) => m,
                None => return false,
            };
            expr_map.iter().all(|(k, v)| match ctx_map.get(k) {
                None => false,
                Some(cv) => {
                    // For world_state conditions: `true` means "key must
                    // exist" — any value satisfies.
                    if *v == serde_json::Value::Bool(true)
                        && precondition.condition_type == "world_state"
                    {
                        true
                    } else {
                        cv == v
                    }
                }
            })
        }
        serde_json::Value::String(key) => {
            context
                .as_object()
                .is_some_and(|m| m.contains_key(key.as_str()))
        }
        _ => false,
    }
}

impl RoutineStore for DefaultRoutineStore {
    fn register(&mut self, routine: Routine) -> Result<()> {
        // Replace if routine_id already exists, otherwise append.
        if let Some(existing) = self
            .routines
            .iter_mut()
            .find(|r| r.routine_id == routine.routine_id)
        {
            *existing = routine;
        } else {
            self.routines.push(routine);
        }
        Ok(())
    }

    fn find_matching(&self, context: &serde_json::Value) -> Vec<&Routine> {
        self.routines
            .iter()
            .filter(|routine| {
                if routine.match_conditions.is_empty() {
                    return false;
                }

                // Group conditions by type and match if ANY group fully
                // satisfies. This lets the same routine fire from either a
                // goal match OR a world state match.
                let goal_conds: Vec<_> = routine
                    .match_conditions
                    .iter()
                    .filter(|c| c.condition_type == "goal_fingerprint")
                    .collect();
                let ws_conds: Vec<_> = routine
                    .match_conditions
                    .iter()
                    .filter(|c| c.condition_type == "world_state")
                    .collect();
                let other_conds: Vec<_> = routine
                    .match_conditions
                    .iter()
                    .filter(|c| {
                        c.condition_type != "goal_fingerprint"
                            && c.condition_type != "world_state"
                    })
                    .collect();

                let goal_match = !goal_conds.is_empty()
                    && goal_conds
                        .iter()
                        .all(|c| precondition_matches(c, context));
                let ws_match = !ws_conds.is_empty()
                    && ws_conds
                        .iter()
                        .all(|c| precondition_matches(c, context));
                let other_match = !other_conds.is_empty()
                    && other_conds
                        .iter()
                        .all(|c| precondition_matches(c, context));

                let match_ok = goal_match || ws_match || other_match;

                // Guard conditions must ALL pass regardless of which group
                // triggered.
                let guard_ok = routine
                    .guard_conditions
                    .iter()
                    .all(|c| precondition_matches(c, context));

                match_ok && guard_ok
            })
            .collect()
    }

    fn get(&self, routine_id: &str) -> Option<&Routine> {
        self.routines.iter().find(|r| r.routine_id == routine_id)
    }

    fn compile_from_schema(&self, schema: &Schema, episodes: &[&Episode]) -> Option<Routine> {
        // Minimum confidence threshold for schema-to-routine compilation.
        const MIN_SCHEMA_CONFIDENCE: f64 = 0.7;
        // Minimum number of supporting successful episodes.
        const MIN_EPISODES: usize = 3;

        if schema.confidence < MIN_SCHEMA_CONFIDENCE {
            return None;
        }

        let successful: Vec<&&Episode> = episodes.iter().filter(|ep| ep.success).collect();
        if successful.len() < MIN_EPISODES {
            return None;
        }

        // The compiled skill path comes from the schema's candidate_skill_ordering.
        if schema.candidate_skill_ordering.is_empty() {
            return None;
        }

        // Verify that the schema's skill ordering actually appears in the episodes.
        // At least half the successful episodes must contain the full ordering as a subsequence.
        let required_matches = successful.len() / 2 + 1;
        let ordering = &schema.candidate_skill_ordering;

        let matching_count = successful
            .iter()
            .filter(|ep| {
                let skills: Vec<&str> = ep.steps.iter().map(|s| s.selected_skill.as_str()).collect();
                is_subsequence(ordering, &skills)
            })
            .count();

        if matching_count < required_matches {
            return None;
        }

        // Compute expected cost from the supporting episodes.
        let avg_cost =
            successful.iter().map(|ep| ep.total_cost).sum::<f64>() / successful.len() as f64;

        // Compute expected effects: collect unique effect patches from observations.
        let expected_effects: Vec<EffectDescriptor> = Vec::new();

        // Confidence is the schema confidence scaled by the proportion of matching episodes.
        let match_ratio = matching_count as f64 / successful.len() as f64;
        let confidence = (schema.confidence * match_ratio).min(0.95);

        // Convert schema trigger conditions to routine match/guard conditions.
        let match_conditions = schema.trigger_conditions.clone();
        let guard_conditions = schema.stop_conditions.clone();

        let routine = Routine {
            routine_id: format!("compiled_{}", schema.schema_id),
            namespace: schema.pack.clone(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions,
            compiled_skill_path: schema.candidate_skill_ordering.clone(),
            guard_conditions,
            expected_cost: avg_cost,
            expected_effect: expected_effects,
            confidence,
            autonomous: false,
        };

        Some(routine)
    }

    fn invalidate(&mut self, routine_id: &str) -> Result<()> {
        let len_before = self.routines.len();
        self.routines.retain(|r| r.routine_id != routine_id);
        if self.routines.len() == len_before {
            return Err(SomaError::Memory(format!(
                "routine not found: {routine_id}"
            )));
        }
        Ok(())
    }

    fn invalidate_by_condition(&mut self, reason: &InvalidationReason) -> Vec<String> {
        let mut invalidated = Vec::new();

        let should_remove: Box<dyn Fn(&Routine) -> bool> = match reason {
            InvalidationReason::PackVersionBreak { removed_skills } => {
                // Invalidate routines whose compiled_skill_path contains any removed skill.
                let removed = removed_skills.clone();
                Box::new(move |routine: &Routine| {
                    routine
                        .compiled_skill_path
                        .iter()
                        .any(|step| removed.contains(step))
                })
            }
            InvalidationReason::ConfidenceDropped { threshold } => {
                let thresh = *threshold;
                Box::new(move |routine: &Routine| routine.confidence < thresh)
            }
            InvalidationReason::PolicyChanged | InvalidationReason::PreconditionsNoLongerHold => {
                // Conservative: invalidate everything.
                Box::new(|_: &Routine| true)
            }
            InvalidationReason::ResourceSchemaChanged { resource_id } => {
                // Invalidate routines that reference the affected resource in their
                // expected_effect target_resource or compiled_skill_path.
                let rid = resource_id.clone();
                Box::new(move |routine: &Routine| {
                    routine
                        .expected_effect
                        .iter()
                        .any(|e| e.target_resource.as_deref() == Some(rid.as_str()))
                        || routine
                            .compiled_skill_path
                            .iter()
                            .any(|step| step.contains(rid.as_str()))
                })
            }
        };

        self.routines.retain(|routine| {
            if should_remove(routine) {
                invalidated.push(routine.routine_id.clone());
                false
            } else {
                true
            }
        });

        invalidated
    }

    fn list_all(&self) -> Vec<&Routine> {
        self.routines.iter().collect()
    }

    fn set_autonomous(&mut self, routine_id: &str, autonomous: bool) -> Result<bool> {
        if let Some(r) = self.routines.iter_mut().find(|r| r.routine_id == routine_id) {
            r.autonomous = autonomous;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Check if `needle` is a subsequence of `haystack`.
fn is_subsequence(needle: &[String], haystack: &[&str]) -> bool {
    let mut needle_idx = 0;
    for item in haystack {
        if needle_idx < needle.len() && *item == needle[needle_idx] {
            needle_idx += 1;
        }
    }
    needle_idx == needle.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::episode::{EpisodeOutcome, EpisodeStep};
    use crate::types::observation::Observation;
    use crate::types::schema::{RollbackBias, SubgoalNode};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_routine(id: &str, match_key: &str, match_val: &str) -> Routine {
        Routine {
            routine_id: id.to_string(),
            namespace: "test".to_string(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions: vec![Precondition {
                condition_type: "key_match".to_string(),
                expression: serde_json::json!({ match_key: match_val }),
                description: format!("{match_key} = {match_val}"),
            }],
            compiled_skill_path: vec!["a".into(), "b".into()],
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence: 0.9,
            autonomous: false,
        }
    }

    fn make_schema_for_compile(confidence: f64) -> Schema {
        Schema {
            schema_id: "test_schema".to_string(),
            namespace: "test".to_string(),
            pack: "test".to_string(),
            name: "Test Schema".to_string(),
            version: semver::Version::new(1, 0, 0),
            trigger_conditions: vec![Precondition {
                condition_type: "goal_fingerprint".into(),
                expression: serde_json::json!({ "goal_fingerprint": "file_read" }),
                description: "goal matches file_read".into(),
            }],
            resource_requirements: Vec::new(),
            subgoal_structure: vec![
                SubgoalNode {
                    subgoal_id: "step_0".into(),
                    description: "open".into(),
                    skill_candidates: vec!["open".into()],
                    dependencies: Vec::new(),
                    optional: false,
                },
                SubgoalNode {
                    subgoal_id: "step_1".into(),
                    description: "read".into(),
                    skill_candidates: vec!["read".into()],
                    dependencies: vec!["step_0".into()],
                    optional: false,
                },
            ],
            candidate_skill_ordering: vec!["open".into(), "read".into(), "close".into()],
            stop_conditions: Vec::new(),
            rollback_bias: RollbackBias::Cautious,
            confidence,
        }
    }

    fn make_episode_with_skills(skills: &[&str]) -> Episode {
        let steps: Vec<EpisodeStep> = skills
            .iter()
            .enumerate()
            .map(|(i, skill)| EpisodeStep {
                step_index: i as u32,
                belief_summary: serde_json::json!({}),
                candidates_considered: vec![skill.to_string()],
                predicted_scores: vec![0.9],
                selected_skill: skill.to_string(),
                observation: Observation {
                    observation_id: Uuid::new_v4(),
                    session_id: Uuid::new_v4(),
                    skill_id: Some(skill.to_string()),
                    port_calls: Vec::new(),
                    raw_result: serde_json::json!({}),
                    structured_result: serde_json::json!({}),
                    effect_patch: None,
                    success: true,
                    failure_class: None,
                    latency_ms: 10,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 0.9,
                    timestamp: Utc::now(),
                },
                belief_patch: serde_json::json!({}),
                progress_delta: 0.5,
                critic_decision: "continue".into(),
                timestamp: Utc::now(),
            })
            .collect();

        Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: "file_read".to_string(),
            initial_belief_summary: serde_json::json!({}),
            steps,
            observations: Vec::new(),
            outcome: EpisodeOutcome::Success,
            total_cost: 0.1,
            success: true,
            tags: Vec::new(),
            embedding: None,
            created_at: Utc::now(),
            salience: 1.0,
            world_state_context: serde_json::json!({}),
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut store = DefaultRoutineStore::new();
        let r = make_routine("r1", "domain", "file");
        store.register(r).unwrap();

        assert!(store.get("r1").is_some());
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_register_overwrites() {
        let mut store = DefaultRoutineStore::new();
        let r1 = Routine {
            confidence: 0.5,
            ..make_routine("r1", "domain", "file")
        };
        let r1_updated = Routine {
            confidence: 0.99,
            ..make_routine("r1", "domain", "file")
        };

        store.register(r1).unwrap();
        assert!((store.get("r1").unwrap().confidence - 0.5).abs() < f64::EPSILON);

        store.register(r1_updated).unwrap();
        assert!((store.get("r1").unwrap().confidence - 0.99).abs() < f64::EPSILON);
    }

    #[test]
    fn test_find_matching() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine("r_file", "domain", "file"))
            .unwrap();
        store
            .register(make_routine("r_net", "domain", "network"))
            .unwrap();

        let ctx = serde_json::json!({ "domain": "file" });
        let results = store.find_matching(&ctx);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].routine_id, "r_file");
    }

    #[test]
    fn test_find_matching_with_guard() {
        let mut store = DefaultRoutineStore::new();
        let r = Routine {
            guard_conditions: vec![Precondition {
                condition_type: "resource_check".into(),
                expression: serde_json::json!({ "disk_available": true }),
                description: "disk must be available".into(),
            }],
            ..make_routine("r_guarded", "domain", "file")
        };
        store.register(r).unwrap();

        // Without guard satisfied
        let ctx_no_guard = serde_json::json!({ "domain": "file" });
        assert!(store.find_matching(&ctx_no_guard).is_empty());

        // With guard satisfied
        let ctx_with_guard = serde_json::json!({ "domain": "file", "disk_available": true });
        assert_eq!(store.find_matching(&ctx_with_guard).len(), 1);
    }

    #[test]
    fn test_invalidate() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine("r1", "domain", "file"))
            .unwrap();
        assert!(store.get("r1").is_some());

        store.invalidate("r1").unwrap();
        assert!(store.get("r1").is_none());
    }

    #[test]
    fn test_invalidate_not_found() {
        let mut store = DefaultRoutineStore::new();
        let result = store.invalidate("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_compile_from_schema_success() {
        let store = DefaultRoutineStore::new();
        let schema = make_schema_for_compile(0.85);

        let ep1 = make_episode_with_skills(&["open", "read", "close"]);
        let ep2 = make_episode_with_skills(&["open", "read", "close"]);
        let ep3 = make_episode_with_skills(&["open", "read", "close"]);
        let refs: Vec<&Episode> = vec![&ep1, &ep2, &ep3];

        let routine = store.compile_from_schema(&schema, &refs);
        assert!(routine.is_some());
        let r = routine.unwrap();
        assert_eq!(r.compiled_skill_path, vec!["open", "read", "close"]);
        assert_eq!(r.origin, RoutineOrigin::SchemaCompiled);
        assert!(r.confidence > 0.0);
    }

    #[test]
    fn test_compile_from_schema_low_confidence() {
        let store = DefaultRoutineStore::new();
        let schema = make_schema_for_compile(0.3); // Below threshold

        let ep1 = make_episode_with_skills(&["open", "read", "close"]);
        let ep2 = make_episode_with_skills(&["open", "read", "close"]);
        let ep3 = make_episode_with_skills(&["open", "read", "close"]);
        let refs: Vec<&Episode> = vec![&ep1, &ep2, &ep3];

        assert!(store.compile_from_schema(&schema, &refs).is_none());
    }

    #[test]
    fn test_compile_from_schema_too_few_episodes() {
        let store = DefaultRoutineStore::new();
        let schema = make_schema_for_compile(0.85);

        let ep1 = make_episode_with_skills(&["open", "read", "close"]);
        let refs: Vec<&Episode> = vec![&ep1];

        assert!(store.compile_from_schema(&schema, &refs).is_none());
    }

    #[test]
    fn test_is_subsequence() {
        let needle: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        assert!(is_subsequence(&needle, &["a", "x", "b", "y", "c"]));
        assert!(is_subsequence(&needle, &["a", "b", "c"]));
        assert!(!is_subsequence(&needle, &["a", "c", "b"]));
        assert!(!is_subsequence(&needle, &["a", "b"]));
    }

    // --- InvalidationReason tests ---

    fn make_routine_with_skills(id: &str, skills: &[&str], confidence: f64) -> Routine {
        Routine {
            routine_id: id.to_string(),
            namespace: "test".to_string(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions: vec![Precondition {
                condition_type: "key_match".to_string(),
                expression: serde_json::json!({ "domain": "test" }),
                description: "domain = test".to_string(),
            }],
            compiled_skill_path: skills.iter().map(|s| s.to_string()).collect(),
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence,
            autonomous: false,
        }
    }

    #[test]
    fn test_invalidate_by_pack_version_break() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills(
                "r1",
                &["ns.open", "ns.read", "ns.close"],
                0.9,
            ))
            .unwrap();
        store
            .register(make_routine_with_skills(
                "r2",
                &["ns.list", "ns.filter"],
                0.85,
            ))
            .unwrap();
        store
            .register(make_routine_with_skills(
                "r3",
                &["other.ping"],
                0.8,
            ))
            .unwrap();

        let reason = InvalidationReason::PackVersionBreak {
            removed_skills: vec!["ns.read".to_string()],
        };
        let invalidated = store.invalidate_by_condition(&reason);

        // Only r1 should be invalidated (it references ns.read).
        assert_eq!(invalidated.len(), 1);
        assert!(invalidated.contains(&"r1".to_string()));
        assert!(store.get("r1").is_none());
        assert!(store.get("r2").is_some());
        assert!(store.get("r3").is_some());
    }

    #[test]
    fn test_invalidate_by_pack_version_break_multiple_removed() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills("r1", &["a", "b"], 0.9))
            .unwrap();
        store
            .register(make_routine_with_skills("r2", &["c", "d"], 0.85))
            .unwrap();
        store
            .register(make_routine_with_skills("r3", &["e"], 0.8))
            .unwrap();

        let reason = InvalidationReason::PackVersionBreak {
            removed_skills: vec!["b".to_string(), "d".to_string()],
        };
        let invalidated = store.invalidate_by_condition(&reason);

        assert_eq!(invalidated.len(), 2);
        assert!(invalidated.contains(&"r1".to_string()));
        assert!(invalidated.contains(&"r2".to_string()));
        assert!(store.get("r3").is_some());
    }

    #[test]
    fn test_invalidate_by_pack_version_break_no_match() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills("r1", &["a", "b"], 0.9))
            .unwrap();

        let reason = InvalidationReason::PackVersionBreak {
            removed_skills: vec!["z".to_string()],
        };
        let invalidated = store.invalidate_by_condition(&reason);

        assert!(invalidated.is_empty());
        assert!(store.get("r1").is_some());
    }

    #[test]
    fn test_invalidate_by_confidence_dropped() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills("high", &["a"], 0.95))
            .unwrap();
        store
            .register(make_routine_with_skills("mid", &["b"], 0.6))
            .unwrap();
        store
            .register(make_routine_with_skills("low", &["c"], 0.3))
            .unwrap();

        let reason = InvalidationReason::ConfidenceDropped { threshold: 0.7 };
        let invalidated = store.invalidate_by_condition(&reason);

        assert_eq!(invalidated.len(), 2);
        assert!(invalidated.contains(&"mid".to_string()));
        assert!(invalidated.contains(&"low".to_string()));
        assert!(store.get("high").is_some());
        assert!(store.get("mid").is_none());
        assert!(store.get("low").is_none());
    }

    #[test]
    fn test_invalidate_by_confidence_dropped_at_boundary() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills("exact", &["a"], 0.7))
            .unwrap();
        store
            .register(make_routine_with_skills("below", &["b"], 0.699))
            .unwrap();

        let reason = InvalidationReason::ConfidenceDropped { threshold: 0.7 };
        let invalidated = store.invalidate_by_condition(&reason);

        // Exact threshold value is NOT below threshold, so should survive.
        assert_eq!(invalidated.len(), 1);
        assert!(invalidated.contains(&"below".to_string()));
        assert!(store.get("exact").is_some());
    }

    #[test]
    fn test_invalidate_by_policy_changed() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills("r1", &["a"], 0.9))
            .unwrap();
        store
            .register(make_routine_with_skills("r2", &["b"], 0.8))
            .unwrap();

        let reason = InvalidationReason::PolicyChanged;
        let invalidated = store.invalidate_by_condition(&reason);

        // PolicyChanged is conservative: invalidates everything.
        assert_eq!(invalidated.len(), 2);
        assert!(store.get("r1").is_none());
        assert!(store.get("r2").is_none());
    }

    #[test]
    fn test_invalidate_by_preconditions_no_longer_hold() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills("r1", &["a"], 0.9))
            .unwrap();
        store
            .register(make_routine_with_skills("r2", &["b"], 0.8))
            .unwrap();

        let reason = InvalidationReason::PreconditionsNoLongerHold;
        let invalidated = store.invalidate_by_condition(&reason);

        // PreconditionsNoLongerHold is conservative: invalidates everything.
        assert_eq!(invalidated.len(), 2);
        assert!(store.get("r1").is_none());
        assert!(store.get("r2").is_none());
    }

    #[test]
    fn test_invalidate_by_resource_schema_changed_via_skill_path() {
        let mut store = DefaultRoutineStore::new();
        // Routine whose skill path contains the resource id as a substring.
        store
            .register(make_routine_with_skills(
                "r1",
                &["ns.read_users", "ns.write_users"],
                0.9,
            ))
            .unwrap();
        store
            .register(make_routine_with_skills(
                "r2",
                &["ns.list_orders"],
                0.85,
            ))
            .unwrap();

        let reason = InvalidationReason::ResourceSchemaChanged {
            resource_id: "users".to_string(),
        };
        let invalidated = store.invalidate_by_condition(&reason);

        // r1 references "users" in its skill path, r2 does not.
        assert_eq!(invalidated.len(), 1);
        assert!(invalidated.contains(&"r1".to_string()));
        assert!(store.get("r2").is_some());
    }

    #[test]
    fn test_invalidate_by_resource_schema_changed_via_expected_effect() {
        let mut store = DefaultRoutineStore::new();
        let mut r = make_routine_with_skills("r_effect", &["ns.ping"], 0.9);
        r.expected_effect = vec![EffectDescriptor {
            effect_type: crate::types::common::EffectType::Update,
            target_resource: Some("accounts".to_string()),
            description: "updates accounts resource".to_string(),
            patch: None,
        }];
        store.register(r).unwrap();
        store
            .register(make_routine_with_skills("r_other", &["ns.noop"], 0.85))
            .unwrap();

        let reason = InvalidationReason::ResourceSchemaChanged {
            resource_id: "accounts".to_string(),
        };
        let invalidated = store.invalidate_by_condition(&reason);

        assert_eq!(invalidated.len(), 1);
        assert!(invalidated.contains(&"r_effect".to_string()));
        assert!(store.get("r_other").is_some());
    }

    #[test]
    fn test_invalidate_empty_store() {
        let mut store = DefaultRoutineStore::new();

        let reason = InvalidationReason::PolicyChanged;
        let invalidated = store.invalidate_by_condition(&reason);

        assert!(invalidated.is_empty());
    }

    #[test]
    fn test_invalidate_by_condition_returns_ids() {
        let mut store = DefaultRoutineStore::new();
        store
            .register(make_routine_with_skills("r1", &["x"], 0.9))
            .unwrap();

        let reason = InvalidationReason::PolicyChanged;
        let invalidated = store.invalidate_by_condition(&reason);

        assert_eq!(invalidated, vec!["r1".to_string()]);
    }

    // --- OR-group matching and world_state tests ---

    #[test]
    fn test_find_matching_by_world_state() {
        let mut store = DefaultRoutineStore::new();
        let r = Routine {
            routine_id: "r_ws".to_string(),
            namespace: "test".to_string(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions: vec![Precondition {
                condition_type: "world_state".to_string(),
                expression: serde_json::json!({ "webhook.crm": "active" }),
                description: "CRM webhook active".to_string(),
            }],
            compiled_skill_path: vec!["handle_crm".into()],
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence: 0.9,
            autonomous: false,
        };
        store.register(r).unwrap();

        // Matches: world state has the right key+value.
        let ctx = serde_json::json!({ "webhook.crm": "active" });
        assert_eq!(store.find_matching(&ctx).len(), 1);

        // Does not match: wrong value.
        let ctx_wrong = serde_json::json!({ "webhook.crm": "inactive" });
        assert!(store.find_matching(&ctx_wrong).is_empty());

        // Does not match: key absent.
        let ctx_missing = serde_json::json!({ "other_key": "active" });
        assert!(store.find_matching(&ctx_missing).is_empty());
    }

    #[test]
    fn test_find_matching_or_groups() {
        let mut store = DefaultRoutineStore::new();
        // Routine with BOTH goal_fingerprint and world_state conditions.
        let r = Routine {
            routine_id: "r_or".to_string(),
            namespace: "test".to_string(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions: vec![
                Precondition {
                    condition_type: "goal_fingerprint".to_string(),
                    expression: serde_json::json!({ "goal_fingerprint": "handle_lead" }),
                    description: "Goal matches handle_lead".to_string(),
                },
                Precondition {
                    condition_type: "world_state".to_string(),
                    expression: serde_json::json!({ "webhook.crm": "new_lead" }),
                    description: "CRM new_lead event".to_string(),
                },
            ],
            compiled_skill_path: vec!["process_lead".into()],
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence: 0.9,
            autonomous: false,
        };
        store.register(r).unwrap();

        // Matches via goal_fingerprint only (world state absent).
        let ctx_goal = serde_json::json!({ "goal_fingerprint": "handle_lead" });
        assert_eq!(store.find_matching(&ctx_goal).len(), 1);

        // Matches via world_state only (goal absent).
        let ctx_ws = serde_json::json!({ "webhook.crm": "new_lead" });
        assert_eq!(store.find_matching(&ctx_ws).len(), 1);

        // Matches when both are present.
        let ctx_both =
            serde_json::json!({ "goal_fingerprint": "handle_lead", "webhook.crm": "new_lead" });
        assert_eq!(store.find_matching(&ctx_both).len(), 1);

        // Does not match: neither group satisfied.
        let ctx_none = serde_json::json!({ "unrelated": "data" });
        assert!(store.find_matching(&ctx_none).is_empty());
    }

    #[test]
    fn test_world_state_existence_match() {
        let mut store = DefaultRoutineStore::new();
        // world_state condition with `true` sentinel = existence check.
        let r = Routine {
            routine_id: "r_exist".to_string(),
            namespace: "test".to_string(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions: vec![Precondition {
                condition_type: "world_state".to_string(),
                expression: serde_json::json!({ "webhook.crm": true }),
                description: "CRM webhook present".to_string(),
            }],
            compiled_skill_path: vec!["handle_crm".into()],
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence: 0.9,
            autonomous: false,
        };
        store.register(r).unwrap();

        // Matches: key exists with an object value (existence check).
        let ctx_obj =
            serde_json::json!({ "webhook.crm": { "event": "new_lead", "source": "hubspot" } });
        assert_eq!(store.find_matching(&ctx_obj).len(), 1);

        // Matches: key exists with a string value.
        let ctx_str = serde_json::json!({ "webhook.crm": "anything" });
        assert_eq!(store.find_matching(&ctx_str).len(), 1);

        // Does not match: key absent.
        let ctx_missing = serde_json::json!({ "other": "data" });
        assert!(store.find_matching(&ctx_missing).is_empty());
    }
}
