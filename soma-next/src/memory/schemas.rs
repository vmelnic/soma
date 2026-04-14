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

    /// Induce schemas from episodes using embedding-based clustering.
    /// Groups episodes by embedding similarity rather than requiring
    /// identical goal fingerprints, then uses PrefixSpan to find
    /// common skill subsequences within each cluster.
    fn induce_from_episodes_with_embedder(
        &self,
        episodes: &[&Episode],
        embedder: &dyn crate::memory::embedder::GoalEmbedder,
    ) -> Vec<Schema> {
        let _ = embedder;
        self.induce_from_episodes(episodes).into_iter().collect()
    }

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
/// - For `world_state` conditions: if the expression value for a key is `true`
///   (boolean), the check passes when the key exists regardless of its actual
///   value (existence-only match).
/// - If expression is a string, it is treated as a required key in the context.
/// - Otherwise, the precondition is considered non-matchable (returns false).
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
                if schema.trigger_conditions.is_empty() {
                    return false;
                }

                // Group conditions by type and match if ANY group fully
                // satisfies. A schema with both goal_fingerprint and
                // world_state conditions can fire from either trigger.
                let goal_conds: Vec<_> = schema
                    .trigger_conditions
                    .iter()
                    .filter(|c| c.condition_type == "goal_fingerprint")
                    .collect();
                let ws_conds: Vec<_> = schema
                    .trigger_conditions
                    .iter()
                    .filter(|c| c.condition_type == "world_state")
                    .collect();
                let other_conds: Vec<_> = schema
                    .trigger_conditions
                    .iter()
                    .filter(|c| {
                        c.condition_type != "goal_fingerprint"
                            && c.condition_type != "world_state"
                    })
                    .collect();

                let goal_match = !goal_conds.is_empty()
                    && goal_conds
                        .iter()
                        .all(|c| precondition_matches(c, trigger_context));
                let ws_match = !ws_conds.is_empty()
                    && ws_conds
                        .iter()
                        .all(|c| precondition_matches(c, trigger_context));
                let other_match = !other_conds.is_empty()
                    && other_conds
                        .iter()
                        .all(|c| precondition_matches(c, trigger_context));

                goal_match || ws_match || other_match
            })
            .collect()
    }

    fn get(&self, schema_id: &str) -> Option<&Schema> {
        self.schemas.iter().find(|s| s.schema_id == schema_id)
    }

    fn list_all(&self) -> Vec<&Schema> {
        self.schemas.iter().collect()
    }

    fn induce_from_episodes_with_embedder(
        &self,
        episodes: &[&Episode],
        embedder: &dyn crate::memory::embedder::GoalEmbedder,
    ) -> Vec<Schema> {
        if episodes.is_empty() {
            return Vec::new();
        }

        // Cluster episodes by embedding similarity (greedy, cosine threshold 0.8).
        let embedding_threshold = 0.8;
        let mut clusters: Vec<(Vec<f32>, Vec<usize>)> = Vec::new(); // (centroid, episode_indices)
        let mut fingerprint_groups: HashMap<String, Vec<usize>> = HashMap::new();

        for (i, ep) in episodes.iter().enumerate() {
            if let Some(ref emb) = ep.embedding {
                let mut assigned = false;
                for (centroid, members) in &mut clusters {
                    let sim = embedder.similarity(emb, centroid);
                    if sim >= embedding_threshold {
                        members.push(i);
                        assigned = true;
                        break;
                    }
                }
                if !assigned {
                    clusters.push((emb.clone(), vec![i]));
                }
            } else {
                // No embedding: group by exact goal_fingerprint.
                fingerprint_groups
                    .entry(ep.goal_fingerprint.clone())
                    .or_default()
                    .push(i);
            }
        }

        // Merge fingerprint groups into the cluster list.
        for (_fp, indices) in fingerprint_groups {
            clusters.push((Vec::new(), indices));
        }

        let mut schemas = Vec::new();

        for (_centroid, members) in &clusters {
            // Need at least 3 successful episodes.
            let successful_indices: Vec<usize> = members
                .iter()
                .copied()
                .filter(|&i| episodes[i].success)
                .collect();

            if successful_indices.len() < 3 {
                continue;
            }

            // Extract skill sequences and salience weights from successful episodes.
            let skill_sequences: Vec<Vec<String>> = successful_indices
                .iter()
                .map(|&i| {
                    episodes[i]
                        .steps
                        .iter()
                        .map(|s| s.selected_skill.clone())
                        .collect()
                })
                .collect();

            let weights: Vec<f64> = successful_indices
                .iter()
                .map(|&i| episodes[i].salience)
                .collect();

            // Use PrefixSpan to find the longest frequent subsequence,
            // weighted by episode salience.
            if let Some(freq) =
                crate::memory::sequence_mining::longest_frequent_subsequence(
                    &skill_sequences,
                    0.7,
                    Some(&weights),
                )
            {
                let candidate_ordering = freq.pattern;
                if candidate_ordering.is_empty() {
                    continue;
                }

                // Use the first successful episode's fingerprint for the schema ID.
                let fingerprint = &episodes[successful_indices[0]].goal_fingerprint;
                let confidence = (freq.support_ratio).min(0.95);

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
                let mut all_tags: Vec<String> = successful_indices
                    .iter()
                    .flat_map(|&i| episodes[i].tags.iter().cloned())
                    .collect();
                all_tags.sort();
                all_tags.dedup();

                // Extract common world state facts across all successful
                // episodes in this cluster. These become the "stimulus" that
                // can trigger the routine reactively via world state.
                let ws_precondition = {
                    let contexts: Vec<&serde_json::Map<String, serde_json::Value>> =
                        successful_indices
                            .iter()
                            .filter_map(|&i| episodes[i].world_state_context.as_object())
                            .collect();

                    if contexts.len() >= 2 {
                        // Find keys present in ALL contexts within this cluster.
                        let first_keys: std::collections::HashSet<&String> =
                            contexts[0].keys().collect();
                        let common_keys: Vec<&String> = first_keys
                            .iter()
                            .filter(|k| contexts[1..].iter().all(|ctx| ctx.contains_key(**k)))
                            .copied()
                            .collect();

                        // Filter out ubiquitous keys (present in >80% of ALL
                        // episodes, not distinctive for this cluster).
                        let total_episodes = episodes.len();
                        let distinctive_keys: Vec<&String> = common_keys
                            .into_iter()
                            .filter(|k| {
                                let global_count = episodes
                                    .iter()
                                    .filter(|ep| {
                                        ep.world_state_context
                                            .as_object()
                                            .is_some_and(|m| m.contains_key(*k))
                                    })
                                    .count();
                                // Keep if present in <80% of all episodes.
                                global_count * 5 < total_episodes * 4
                            })
                            .collect();

                        if !distinctive_keys.is_empty() {
                            let mut expr = serde_json::Map::new();
                            for key in &distinctive_keys {
                                let first_val = &contexts[0][*key];
                                let all_same = contexts
                                    .iter()
                                    .all(|ctx| ctx.get(*key) == Some(first_val));
                                if all_same {
                                    expr.insert((*key).clone(), first_val.clone());
                                } else {
                                    // Values differ across episodes — check key
                                    // existence only (sentinel `true`).
                                    expr.insert((*key).clone(), serde_json::json!(true));
                                }
                            }
                            Some(Precondition {
                                condition_type: "world_state".to_string(),
                                expression: serde_json::Value::Object(expr),
                                description: format!(
                                    "World state contains: {}",
                                    distinctive_keys
                                        .iter()
                                        .map(|k| k.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                let mut trigger_conditions = vec![Precondition {
                    condition_type: "goal_fingerprint".to_string(),
                    expression: serde_json::json!({ "goal_fingerprint": fingerprint }),
                    description: format!("Goal matches {fingerprint}"),
                }];
                if let Some(ws_cond) = ws_precondition {
                    trigger_conditions.push(ws_cond);
                }

                let schema = Schema {
                    schema_id: format!("induced_{fingerprint}"),
                    namespace: String::new(),
                    pack: String::new(),
                    name: format!("Induced schema for {fingerprint}"),
                    version: semver::Version::new(0, 1, 0),
                    trigger_conditions,
                    resource_requirements: Vec::new(),
                    subgoal_structure: subgoals,
                    candidate_skill_ordering: candidate_ordering,
                    stop_conditions: Vec::new(),
                    rollback_bias: RollbackBias::Cautious,
                    confidence,
                };

                schemas.push(schema);
            }
        }

        schemas
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

/// Run a full consolidation cycle over all stored episodes.
///
/// This is the "sleep" equivalent: replays episodes, induces schemas,
/// compiles routines, and reports what was created. Called periodically
/// by the background consolidation thread or on-demand via MCP.
pub fn run_consolidation_cycle(
    episode_store: &std::sync::Arc<std::sync::Mutex<dyn crate::memory::episodes::EpisodeStore + Send>>,
    schema_store: &std::sync::Arc<std::sync::Mutex<dyn SchemaStore + Send>>,
    routine_store: &std::sync::Arc<std::sync::Mutex<dyn crate::memory::routines::RoutineStore + Send>>,
    embedder: &dyn crate::memory::embedder::GoalEmbedder,
) -> (usize, usize) {
    // 1. Lock episode store, list all episodes, release lock.
    let all_episodes: Vec<crate::types::episode::Episode> = {
        let es = match episode_store.lock() {
            Ok(es) => es,
            Err(_) => return (0, 0),
        };
        let count = es.count();
        es.list(count, 0).into_iter().cloned().collect()
    };

    // 2. Need at least 3 episodes to attempt any induction.
    if all_episodes.len() < 3 {
        return (0, 0);
    }

    // 3. Induce schemas from all episodes using the embedding-aware path.
    let episode_refs: Vec<&crate::types::episode::Episode> = all_episodes.iter().collect();
    let induced_schemas = {
        let ss = match schema_store.lock() {
            Ok(ss) => ss,
            Err(_) => return (0, 0),
        };
        ss.induce_from_episodes_with_embedder(&episode_refs, embedder)
    };

    let mut schemas_count = 0usize;
    let mut routines_count = 0usize;

    for schema in &induced_schemas {
        // 4. Register each induced schema (skip duplicates — register replaces).
        {
            let mut ss = match schema_store.lock() {
                Ok(ss) => ss,
                Err(_) => continue,
            };
            if let Err(e) = ss.register(schema.clone()) {
                eprintln!("[consolidation] failed to register schema: {e}");
                continue;
            }
        }
        schemas_count += 1;

        // 5. Only attempt routine compilation for high-confidence schemas.
        if schema.confidence < 0.7 {
            continue;
        }

        let compiled_routine = {
            let rs = match routine_store.lock() {
                Ok(rs) => rs,
                Err(_) => continue,
            };
            rs.compile_from_schema(schema, &episode_refs)
        };

        // 6. Register each compiled routine.
        if let Some(routine) = compiled_routine {
            let mut rs = match routine_store.lock() {
                Ok(rs) => rs,
                Err(_) => continue,
            };
            if let Err(e) = rs.register(routine) {
                eprintln!("[consolidation] failed to register routine: {e}");
            } else {
                routines_count += 1;
            }
        }
    }

    // 7. Log summary.
    eprintln!(
        "[consolidation] induced {} schemas, compiled {} routines",
        schemas_count, routines_count
    );

    (schemas_count, routines_count)
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
            salience: 1.0,
            world_state_context: serde_json::json!({}),
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

    // --- induce_from_episodes_with_embedder tests ---

    fn make_episode_with_embedding(fingerprint: &str, skills: &[&str], embedding: Vec<f32>) -> Episode {
        let mut ep = make_episode_with_steps(fingerprint, skills);
        ep.embedding = Some(embedding);
        ep
    }

    #[test]
    fn test_induce_with_embedder_clusters_by_embedding() {
        use crate::memory::embedder::HashEmbedder;
        let store = DefaultSchemaStore::new();
        let embedder = HashEmbedder::new();

        // Three episodes with the same embedding cluster and shared skill pattern.
        let ep1 = make_episode_with_embedding("list files", &["open", "read", "close"], vec![1.0, 0.0, 0.0]);
        let ep2 = make_episode_with_embedding("list files", &["open", "read", "close"], vec![0.99, 0.1, 0.0]);
        let ep3 = make_episode_with_embedding("list files", &["open", "read", "close"], vec![0.98, 0.15, 0.0]);

        let refs: Vec<&Episode> = vec![&ep1, &ep2, &ep3];
        let schemas = store.induce_from_episodes_with_embedder(&refs, &embedder);

        assert!(!schemas.is_empty(), "should induce at least one schema");
        assert_eq!(schemas[0].candidate_skill_ordering, vec!["open", "read", "close"]);
    }

    #[test]
    fn test_induce_with_embedder_separate_clusters() {
        use crate::memory::embedder::HashEmbedder;
        let store = DefaultSchemaStore::new();
        let embedder = HashEmbedder::new();

        // Cluster A: 3 episodes with similar embeddings.
        let a1 = make_episode_with_embedding("read file", &["open", "read", "close"], vec![1.0, 0.0, 0.0]);
        let a2 = make_episode_with_embedding("read file", &["open", "read", "close"], vec![0.99, 0.05, 0.0]);
        let a3 = make_episode_with_embedding("read file", &["open", "read", "close"], vec![0.98, 0.1, 0.0]);

        // Cluster B: 3 episodes with different embeddings.
        let b1 = make_episode_with_embedding("send email", &["connect", "auth", "send"], vec![0.0, 0.0, 1.0]);
        let b2 = make_episode_with_embedding("send email", &["connect", "auth", "send"], vec![0.0, 0.05, 0.99]);
        let b3 = make_episode_with_embedding("send email", &["connect", "auth", "send"], vec![0.0, 0.1, 0.98]);

        let refs: Vec<&Episode> = vec![&a1, &a2, &a3, &b1, &b2, &b3];
        let schemas = store.induce_from_episodes_with_embedder(&refs, &embedder);

        assert_eq!(schemas.len(), 2, "should produce two schemas from two clusters");
    }

    #[test]
    fn test_induce_with_embedder_too_few_episodes() {
        use crate::memory::embedder::HashEmbedder;
        let store = DefaultSchemaStore::new();
        let embedder = HashEmbedder::new();

        // Only 2 episodes in one cluster — not enough.
        let ep1 = make_episode_with_embedding("fp", &["a", "b"], vec![1.0, 0.0]);
        let ep2 = make_episode_with_embedding("fp", &["a", "b"], vec![0.99, 0.1]);

        let refs: Vec<&Episode> = vec![&ep1, &ep2];
        let schemas = store.induce_from_episodes_with_embedder(&refs, &embedder);

        assert!(schemas.is_empty(), "two episodes should not be enough for induction");
    }

    #[test]
    fn test_induce_with_embedder_fallback_no_embeddings() {
        use crate::memory::embedder::HashEmbedder;
        let store = DefaultSchemaStore::new();
        let embedder = HashEmbedder::new();

        // Episodes without embeddings: should fall back to fingerprint grouping.
        let ep1 = make_episode_with_steps("file_read", &["open", "read", "close"]);
        let ep2 = make_episode_with_steps("file_read", &["open", "read", "close"]);
        let ep3 = make_episode_with_steps("file_read", &["open", "read", "close"]);

        let refs: Vec<&Episode> = vec![&ep1, &ep2, &ep3];
        let schemas = store.induce_from_episodes_with_embedder(&refs, &embedder);

        assert!(!schemas.is_empty());
        assert_eq!(schemas[0].candidate_skill_ordering, vec!["open", "read", "close"]);
    }

    #[test]
    fn test_induce_with_embedder_confidence_capped() {
        use crate::memory::embedder::HashEmbedder;
        let store = DefaultSchemaStore::new();
        let embedder = HashEmbedder::new();

        // All episodes are in one cluster, all have the same pattern.
        let eps: Vec<Episode> = (0..5)
            .map(|_| make_episode_with_embedding("goal", &["a", "b"], vec![1.0, 0.0, 0.0]))
            .collect();

        let refs: Vec<&Episode> = eps.iter().collect();
        let schemas = store.induce_from_episodes_with_embedder(&refs, &embedder);

        assert!(!schemas.is_empty());
        assert!(schemas[0].confidence <= 0.95, "confidence should be capped at 0.95");
    }

    #[test]
    fn test_induce_with_embedder_empty_input() {
        use crate::memory::embedder::HashEmbedder;
        let store = DefaultSchemaStore::new();
        let embedder = HashEmbedder::new();

        let schemas = store.induce_from_episodes_with_embedder(&[], &embedder);
        assert!(schemas.is_empty());
    }
}
