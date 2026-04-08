use std::collections::HashMap;

use crate::errors::Result;
use crate::types::common::Precondition;
use crate::types::episode::Episode;
use crate::types::schema::{RollbackBias, Schema, SubgoalNode};

/// SchemaStore — storage and retrieval of reusable abstract control structures.
/// Schemas are induced from repeated episodes or provided by packs.
pub trait SchemaStore {
    /// Register a schema (from a pack or from induction).
    fn register(&mut self, schema: Schema) -> Result<()>;

    /// Find schemas whose trigger conditions match the given context.
    fn find_matching(&self, trigger_context: &serde_json::Value) -> Vec<&Schema>;

    /// Get a single schema by ID.
    fn get(&self, schema_id: &str) -> Option<&Schema>;

    /// Attempt to induce a new schema from a set of episodes that share
    /// a common goal fingerprint and successful outcome pattern.
    /// Returns None if there is insufficient evidence for induction.
    fn induce_from_episodes(&self, episodes: &[&Episode]) -> Option<Schema>;

    /// List all registered schemas.
    fn list_all(&self) -> Vec<&Schema>;
}

/// Default in-memory schema store backed by Vec<Schema>.
pub struct DefaultSchemaStore {
    schemas: Vec<Schema>,
}

impl DefaultSchemaStore {
    pub fn new() -> Self {
        Self {
            schemas: Vec::new(),
        }
    }
}

impl Default for DefaultSchemaStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a single Precondition is satisfied by the given JSON context.
/// Evaluates the precondition's expression against the context.
///
/// Matching rules:
/// - If expression is an object, every key/value in the expression must appear
///   in the context at the top level (shallow structural match).
/// - If expression is a string, it is treated as a required key in the context.
/// - Otherwise, the precondition is considered non-matchable (returns false).
fn precondition_matches(precondition: &Precondition, context: &serde_json::Value) -> bool {
    match &precondition.expression {
        serde_json::Value::Object(expr_map) => {
            let ctx_map = match context.as_object() {
                Some(m) => m,
                None => return false,
            };
            expr_map
                .iter()
                .all(|(k, v)| ctx_map.get(k).is_some_and(|cv| cv == v))
        }
        serde_json::Value::String(key) => {
            context
                .as_object()
                .is_some_and(|m| m.contains_key(key.as_str()))
        }
        _ => false,
    }
}

impl SchemaStore for DefaultSchemaStore {
    fn register(&mut self, schema: Schema) -> Result<()> {
        // Replace if schema_id already exists, otherwise append.
        if let Some(existing) = self.schemas.iter_mut().find(|s| s.schema_id == schema.schema_id) {
            *existing = schema;
        } else {
            self.schemas.push(schema);
        }
        Ok(())
    }

    fn find_matching(&self, trigger_context: &serde_json::Value) -> Vec<&Schema> {
        self.schemas
            .iter()
            .filter(|schema| {
                // A schema matches if ALL of its trigger_conditions are satisfied.
                // A schema with no trigger conditions matches nothing (must be explicit).
                !schema.trigger_conditions.is_empty()
                    && schema
                        .trigger_conditions
                        .iter()
                        .all(|pc| precondition_matches(pc, trigger_context))
            })
            .collect()
    }

    fn get(&self, schema_id: &str) -> Option<&Schema> {
        self.schemas.iter().find(|s| s.schema_id == schema_id)
    }

    fn list_all(&self) -> Vec<&Schema> {
        self.schemas.iter().collect()
    }

    fn induce_from_episodes(&self, episodes: &[&Episode]) -> Option<Schema> {
        // Require at least 3 successful episodes with the same goal fingerprint.
        if episodes.len() < 3 {
            return None;
        }

        let successful: Vec<&&Episode> = episodes.iter().filter(|ep| ep.success).collect();
        if successful.len() < 3 {
            return None;
        }

        // All successful episodes must share the same goal fingerprint.
        let fingerprint = &successful[0].goal_fingerprint;
        if !successful
            .iter()
            .all(|ep| ep.goal_fingerprint == *fingerprint)
        {
            return None;
        }

        // Extract the common skill sequence across successful episodes.
        // Find skills that appear in every episode's steps, preserving order.
        let skill_sequences: Vec<Vec<String>> = successful
            .iter()
            .map(|ep| ep.steps.iter().map(|s| s.selected_skill.clone()).collect())
            .collect();

        let candidate_ordering = extract_common_ordering(&skill_sequences);
        if candidate_ordering.is_empty() {
            return None;
        }

        // Build subgoal structure from the common ordering.
        let subgoals: Vec<SubgoalNode> = candidate_ordering
            .iter()
            .enumerate()
            .map(|(i, skill)| SubgoalNode {
                subgoal_id: format!("step_{i}"),
                description: format!("Execute {skill}"),
                skill_candidates: vec![skill.clone()],
                dependencies: if i > 0 {
                    vec![format!("step_{}", i - 1)]
                } else {
                    Vec::new()
                },
                optional: false,
            })
            .collect();

        // Collect all tags from the contributing episodes.
        let mut all_tags: Vec<String> = successful
            .iter()
            .flat_map(|ep| ep.tags.iter().cloned())
            .collect();
        all_tags.sort();
        all_tags.dedup();

        // Confidence scales with number of supporting episodes, capped at 0.95.
        let confidence = (successful.len() as f64 / 10.0).min(0.95);

        let schema = Schema {
            schema_id: format!("induced_{fingerprint}"),
            namespace: String::new(), // Induced schemas have no namespace
            pack: String::new(), // Induced schemas have no pack
            name: format!("Induced schema for {fingerprint}"),
            version: semver::Version::new(0, 1, 0),
            trigger_conditions: vec![Precondition {
                condition_type: "goal_fingerprint".to_string(),
                expression: serde_json::json!({ "goal_fingerprint": fingerprint }),
                description: format!("Goal matches {fingerprint}"),
            }],
            resource_requirements: Vec::new(),
            subgoal_structure: subgoals,
            candidate_skill_ordering: candidate_ordering,
            stop_conditions: Vec::new(),
            rollback_bias: RollbackBias::Cautious,
            confidence,
        };

        Some(schema)
    }
}

/// Extract the longest common subsequence of skills across multiple sequences.
/// This finds skills present in every sequence while preserving their relative order.
fn extract_common_ordering(sequences: &[Vec<String>]) -> Vec<String> {
    if sequences.is_empty() {
        return Vec::new();
    }

    if sequences.len() == 1 {
        return sequences[0].clone();
    }

    // Count how many sequences each skill appears in.
    let mut skill_counts: HashMap<&str, usize> = HashMap::new();
    for seq in sequences {
        // Deduplicate within each sequence for counting.
        let mut seen = std::collections::HashSet::new();
        for skill in seq {
            if seen.insert(skill.as_str()) {
                *skill_counts.entry(skill.as_str()).or_insert(0) += 1;
            }
        }
    }

    // Keep only skills that appear in every sequence.
    let universal: std::collections::HashSet<&str> = skill_counts
        .into_iter()
        .filter(|(_, count)| *count == sequences.len())
        .map(|(skill, _)| skill)
        .collect();

    // Take their order from the first sequence.
    sequences[0]
        .iter()
        .filter(|s| universal.contains(s.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::episode::{EpisodeOutcome, EpisodeStep};
    use crate::types::observation::Observation;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_schema(id: &str, trigger_key: &str, trigger_val: &str) -> Schema {
        Schema {
            schema_id: id.to_string(),
            namespace: "test".to_string(),
            pack: "test".to_string(),
            name: id.to_string(),
            version: semver::Version::new(1, 0, 0),
            trigger_conditions: vec![Precondition {
                condition_type: "key_match".to_string(),
                expression: serde_json::json!({ trigger_key: trigger_val }),
                description: format!("{trigger_key} = {trigger_val}"),
            }],
            resource_requirements: Vec::new(),
            subgoal_structure: Vec::new(),
            candidate_skill_ordering: vec!["skill_a".into()],
            stop_conditions: Vec::new(),
            rollback_bias: RollbackBias::Cautious,
            confidence: 0.8,
        }
    }

    fn make_episode_with_steps(fingerprint: &str, skills: &[&str]) -> Episode {
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
            goal_fingerprint: fingerprint.to_string(),
            initial_belief_summary: serde_json::json!({}),
            steps,
            observations: Vec::new(),
            outcome: EpisodeOutcome::Success,
            total_cost: 0.1,
            success: true,
            tags: vec!["test".into()],
            embedding: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut store = DefaultSchemaStore::new();
        let schema = make_schema("s1", "domain", "file");
        store.register(schema).unwrap();

        assert!(store.get("s1").is_some());
        assert!(store.get("s_nonexistent").is_none());
    }

    #[test]
    fn test_register_overwrites_existing() {
        let mut store = DefaultSchemaStore::new();
        let s1 = make_schema("s1", "domain", "file");
        let s1_updated = Schema {
            confidence: 0.99,
            ..make_schema("s1", "domain", "file")
        };

        store.register(s1).unwrap();
        assert!((store.get("s1").unwrap().confidence - 0.8).abs() < f64::EPSILON);

        store.register(s1_updated).unwrap();
        assert!((store.get("s1").unwrap().confidence - 0.99).abs() < f64::EPSILON);
    }

    #[test]
    fn test_find_matching() {
        let mut store = DefaultSchemaStore::new();
        store
            .register(make_schema("s_file", "domain", "file"))
            .unwrap();
        store
            .register(make_schema("s_net", "domain", "network"))
            .unwrap();

        let ctx = serde_json::json!({ "domain": "file" });
        let results = store.find_matching(&ctx);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].schema_id, "s_file");

        let ctx_none = serde_json::json!({ "domain": "crypto" });
        assert!(store.find_matching(&ctx_none).is_empty());
    }

    #[test]
    fn test_induce_requires_minimum_episodes() {
        let store = DefaultSchemaStore::new();

        // Two episodes: not enough
        let ep1 = make_episode_with_steps("fp", &["a", "b"]);
        let ep2 = make_episode_with_steps("fp", &["a", "b"]);
        let refs: Vec<&Episode> = vec![&ep1, &ep2];
        assert!(store.induce_from_episodes(&refs).is_none());
    }

    #[test]
    fn test_induce_from_matching_episodes() {
        let store = DefaultSchemaStore::new();
        let ep1 = make_episode_with_steps("file_read", &["open", "read", "close"]);
        let ep2 = make_episode_with_steps("file_read", &["open", "read", "close"]);
        let ep3 = make_episode_with_steps("file_read", &["open", "read", "close"]);

        let refs: Vec<&Episode> = vec![&ep1, &ep2, &ep3];
        let schema = store.induce_from_episodes(&refs);
        assert!(schema.is_some());

        let s = schema.unwrap();
        assert_eq!(s.candidate_skill_ordering, vec!["open", "read", "close"]);
        assert!(s.schema_id.contains("file_read"));
        assert_eq!(s.subgoal_structure.len(), 3);
    }

    #[test]
    fn test_induce_extracts_common_subsequence() {
        let store = DefaultSchemaStore::new();
        // Different episodes with varying extra steps but a common core.
        let ep1 = make_episode_with_steps("goal", &["init", "process", "cleanup"]);
        let ep2 = make_episode_with_steps("goal", &["init", "validate", "process", "cleanup"]);
        let ep3 = make_episode_with_steps("goal", &["init", "process", "log", "cleanup"]);

        let refs: Vec<&Episode> = vec![&ep1, &ep2, &ep3];
        let schema = store.induce_from_episodes(&refs).unwrap();

        // "init", "process", "cleanup" appear in all three
        assert_eq!(
            schema.candidate_skill_ordering,
            vec!["init", "process", "cleanup"]
        );
    }

    #[test]
    fn test_induce_rejects_mixed_fingerprints() {
        let store = DefaultSchemaStore::new();
        let ep1 = make_episode_with_steps("fp_a", &["x"]);
        let ep2 = make_episode_with_steps("fp_b", &["x"]);
        let ep3 = make_episode_with_steps("fp_a", &["x"]);

        let refs: Vec<&Episode> = vec![&ep1, &ep2, &ep3];
        assert!(store.induce_from_episodes(&refs).is_none());
    }

    #[test]
    fn test_precondition_string_key() {
        let pc = Precondition {
            condition_type: "key_present".into(),
            expression: serde_json::json!("domain"),
            description: "domain key required".into(),
        };
        let ctx = serde_json::json!({ "domain": "anything" });
        assert!(precondition_matches(&pc, &ctx));

        let ctx_missing = serde_json::json!({ "other": "val" });
        assert!(!precondition_matches(&pc, &ctx_missing));
    }
}
