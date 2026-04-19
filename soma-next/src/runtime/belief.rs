use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::belief::{BeliefPatch, BeliefState, Fact, ResourceEntry};

/// BeliefRuntime — manages the runtime's current model of the world.
///
/// Responsibilities:
/// - Create empty belief states for new sessions
/// - Merge observation results via patches (add, update, remove)
/// - Query resources and facts
/// - Compute deterministic world hashes (SHA-256)
/// - Checkpoint/restore for persistence
pub trait BeliefRuntime {
    /// Create an empty belief state for a session.
    fn create_belief(&self, session_id: Uuid) -> Result<BeliefState>;

    /// Apply a patch (observation results) to a belief state.
    /// Handles added/updated/removed resources, facts, and binding updates.
    fn apply_patch(&self, belief: &mut BeliefState, patch: BeliefPatch) -> Result<()>;

    /// Query a specific resource by type and id.
    fn query_resource<'a>(
        &self,
        belief: &'a BeliefState,
        resource_type: &str,
        resource_id: &str,
    ) -> Option<&'a ResourceEntry>;

    /// Query all facts matching a given subject.
    fn query_facts<'a>(&self, belief: &'a BeliefState, subject: &str) -> Vec<&'a Fact>;

    /// Compute a SHA-256 hash of the serialized belief state (excluding the hash field itself).
    fn compute_world_hash(&self, belief: &BeliefState) -> String;

    /// Serialize the belief state for persistence.
    fn checkpoint(&self, belief: &BeliefState) -> Result<Vec<u8>>;

    /// Deserialize a belief state from a checkpoint.
    fn restore(&self, data: &[u8]) -> Result<BeliefState>;
}

/// Default implementation of BeliefRuntime.
pub struct DefaultBeliefRuntime;

impl DefaultBeliefRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultBeliefRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl BeliefRuntime for DefaultBeliefRuntime {
    fn create_belief(&self, session_id: Uuid) -> Result<BeliefState> {
        let belief = BeliefState {
            belief_id: Uuid::new_v4(),
            session_id,
            resources: Vec::new(),
            facts: Vec::new(),
            uncertainties: Vec::new(),
            provenance: Vec::new(),
            active_bindings: Vec::new(),
            world_hash: String::new(),
            updated_at: Utc::now(),
        };
        Ok(belief)
    }

    fn apply_patch(&self, belief: &mut BeliefState, patch: BeliefPatch) -> Result<()> {
        // --- Resources ---

        // Remove resources by id.
        if !patch.removed_resource_ids.is_empty() {
            belief.resources.retain(|r| {
                !patch
                    .removed_resource_ids
                    .contains(&r.resource_ref.resource_id)
            });
        }

        // Update existing resources (match on resource_type + resource_id).
        for updated in &patch.updated_resources {
            if let Some(existing) = belief.resources.iter_mut().find(|r| {
                r.resource_ref.resource_type == updated.resource_ref.resource_type
                    && r.resource_ref.resource_id == updated.resource_ref.resource_id
            }) {
                // Only apply if the incoming version is at least as new.
                if updated.resource_ref.version < existing.resource_ref.version {
                    return Err(SomaError::ResourceVersionConflict {
                        expected: existing.resource_ref.version,
                        found: updated.resource_ref.version,
                    });
                }
                existing.resource_ref.version = updated.resource_ref.version;
                existing.resource_ref.origin = updated.resource_ref.origin.clone();
                existing.data = updated.data.clone();
                existing.confidence = updated.confidence;
                existing.provenance = updated.provenance;
            } else {
                // Update target not found — treat as add.
                belief.resources.push(updated.clone());
            }
        }

        // Add new resources.
        for added in patch.added_resources {
            // Check for duplicates — if a resource with the same type+id already exists, reject.
            let duplicate = belief.resources.iter().any(|r| {
                r.resource_ref.resource_type == added.resource_ref.resource_type
                    && r.resource_ref.resource_id == added.resource_ref.resource_id
            });
            if duplicate {
                return Err(SomaError::BeliefConflict(format!(
                    "resource already exists: {}/{}",
                    added.resource_ref.resource_type, added.resource_ref.resource_id
                )));
            }
            belief.resources.push(added);
        }

        // --- Facts ---

        // Remove facts by id.
        if !patch.removed_fact_ids.is_empty() {
            belief
                .facts
                .retain(|f| !patch.removed_fact_ids.contains(&f.fact_id));
        }

        // Update existing facts (match on fact_id).
        for updated in &patch.updated_facts {
            if let Some(existing) = belief.facts.iter_mut().find(|f| f.fact_id == updated.fact_id) {
                existing.subject = updated.subject.clone();
                existing.predicate = updated.predicate.clone();
                existing.value = updated.value.clone();
                existing.confidence = updated.confidence;
                existing.provenance = updated.provenance;
                existing.timestamp = updated.timestamp;
            } else {
                // Update target not found — treat as add.
                belief.facts.push(updated.clone());
            }
        }

        // Add new facts.
        for added in patch.added_facts {
            let duplicate = belief.facts.iter().any(|f| f.fact_id == added.fact_id);
            if duplicate {
                return Err(SomaError::BeliefConflict(format!(
                    "fact already exists: {}",
                    added.fact_id
                )));
            }
            belief.facts.push(added);
        }

        // --- Bindings ---

        // Merge binding updates: upsert by name.
        for binding in patch.binding_updates {
            if let Some(existing) = belief
                .active_bindings
                .iter_mut()
                .find(|b| b.name == binding.name)
            {
                existing.value = binding.value;
                existing.source = binding.source;
                existing.confidence = binding.confidence;
            } else {
                belief.active_bindings.push(binding);
            }
        }

        // Recompute world hash and timestamp.
        belief.world_hash = self.compute_world_hash(belief);
        belief.updated_at = Utc::now();

        Ok(())
    }

    fn query_resource<'a>(
        &self,
        belief: &'a BeliefState,
        resource_type: &str,
        resource_id: &str,
    ) -> Option<&'a ResourceEntry> {
        belief.resources.iter().find(|r| {
            r.resource_ref.resource_type == resource_type
                && r.resource_ref.resource_id == resource_id
        })
    }

    fn query_facts<'a>(&self, belief: &'a BeliefState, subject: &str) -> Vec<&'a Fact> {
        belief.facts.iter().filter(|f| f.subject == subject).collect()
    }

    fn compute_world_hash(&self, belief: &BeliefState) -> String {
        // Hash over a deterministic representation: resources, facts, bindings.
        // We exclude world_hash itself and updated_at to avoid circular dependency.
        #[derive(serde::Serialize)]
        struct HashableState<'a> {
            belief_id: &'a Uuid,
            session_id: &'a Uuid,
            resources: &'a Vec<ResourceEntry>,
            facts: &'a Vec<Fact>,
            uncertainties: &'a Vec<crate::types::belief::Uncertainty>,
            provenance: &'a Vec<crate::types::belief::ProvenanceRecord>,
            active_bindings: &'a Vec<crate::types::belief::Binding>,
        }

        let hashable = HashableState {
            belief_id: &belief.belief_id,
            session_id: &belief.session_id,
            resources: &belief.resources,
            facts: &belief.facts,
            uncertainties: &belief.uncertainties,
            provenance: &belief.provenance,
            active_bindings: &belief.active_bindings,
        };

        let serialized =
            serde_json::to_vec(&hashable).expect("belief state serialization must not fail");
        let hash = Sha256::digest(&serialized);
        format!("{:x}", hash)
    }

    fn checkpoint(&self, belief: &BeliefState) -> Result<Vec<u8>> {
        serde_json::to_vec(belief).map_err(SomaError::from)
    }

    fn restore(&self, data: &[u8]) -> Result<BeliefState> {
        serde_json::from_slice(data).map_err(SomaError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::belief::{Binding, ResourceEntry};
    use crate::types::common::FactProvenance;
    use crate::types::resource::ResourceRef;

    fn make_runtime() -> DefaultBeliefRuntime {
        DefaultBeliefRuntime::new()
    }

    fn make_resource(rtype: &str, rid: &str, version: u64) -> ResourceEntry {
        ResourceEntry {
            resource_ref: ResourceRef {
                resource_type: rtype.to_string(),
                resource_id: rid.to_string(),
                version,
                origin: "test".to_string(),
            },
            data: serde_json::json!({"key": "value"}),
            confidence: 1.0,
            provenance: FactProvenance::Observed,
        }
    }

    fn make_fact(id: &str, subject: &str, predicate: &str) -> Fact {
        Fact {
            fact_id: id.to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            value: serde_json::json!(true),
            confidence: 0.9,
            provenance: FactProvenance::Asserted,
            timestamp: Utc::now(),
            ttl_ms: None,
        }
    }

    #[test]
    fn test_create_belief() {
        let rt = make_runtime();
        let sid = Uuid::new_v4();
        let belief = rt.create_belief(sid).unwrap();

        assert_eq!(belief.session_id, sid);
        assert!(belief.resources.is_empty());
        assert!(belief.facts.is_empty());
        assert!(belief.active_bindings.is_empty());
        assert!(belief.world_hash.is_empty());
    }

    #[test]
    fn test_apply_patch_add_resources() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![make_resource("file", "readme.md", 1)],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };

        rt.apply_patch(&mut belief, patch).unwrap();
        assert_eq!(belief.resources.len(), 1);
        assert!(!belief.world_hash.is_empty());
    }

    #[test]
    fn test_apply_patch_update_resources() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        // Add initial resource.
        let patch = BeliefPatch {
            added_resources: vec![make_resource("file", "readme.md", 1)],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        // Update it.
        let mut updated = make_resource("file", "readme.md", 2);
        updated.data = serde_json::json!({"key": "new_value"});
        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![updated],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        assert_eq!(belief.resources.len(), 1);
        assert_eq!(belief.resources[0].data["key"], "new_value");
        assert_eq!(belief.resources[0].resource_ref.version, 2);
    }

    #[test]
    fn test_apply_patch_version_conflict() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![make_resource("file", "readme.md", 5)],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        // Try to update with older version.
        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![make_resource("file", "readme.md", 3)],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        let result = rt.apply_patch(&mut belief, patch);
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_patch_remove_resources() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![
                make_resource("file", "a.txt", 1),
                make_resource("file", "b.txt", 1),
            ],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();
        assert_eq!(belief.resources.len(), 2);

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec!["a.txt".to_string()],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();
        assert_eq!(belief.resources.len(), 1);
        assert_eq!(belief.resources[0].resource_ref.resource_id, "b.txt");
    }

    #[test]
    fn test_apply_patch_facts() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![
                make_fact("f1", "user:alice", "is_active"),
                make_fact("f2", "user:alice", "has_role"),
                make_fact("f3", "user:bob", "is_active"),
            ],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        let alice_facts = rt.query_facts(&belief, "user:alice");
        assert_eq!(alice_facts.len(), 2);

        let bob_facts = rt.query_facts(&belief, "user:bob");
        assert_eq!(bob_facts.len(), 1);
    }

    #[test]
    fn test_apply_patch_remove_facts() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![make_fact("f1", "user:alice", "is_active")],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();
        assert_eq!(belief.facts.len(), 1);

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec!["f1".to_string()],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();
        assert!(belief.facts.is_empty());
    }

    #[test]
    fn test_apply_patch_bindings() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![Binding {
                name: "current_user".to_string(),
                value: serde_json::json!("alice"),
                source: "session".to_string(),
                confidence: 1.0,
            }],
        };
        rt.apply_patch(&mut belief, patch).unwrap();
        assert_eq!(belief.active_bindings.len(), 1);
        assert_eq!(belief.active_bindings[0].value, "alice");

        // Update existing binding.
        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![Binding {
                name: "current_user".to_string(),
                value: serde_json::json!("bob"),
                source: "session".to_string(),
                confidence: 1.0,
            }],
        };
        rt.apply_patch(&mut belief, patch).unwrap();
        assert_eq!(belief.active_bindings.len(), 1);
        assert_eq!(belief.active_bindings[0].value, "bob");
    }

    #[test]
    fn test_query_resource() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![
                make_resource("file", "readme.md", 1),
                make_resource("database", "users", 1),
            ],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        let found = rt.query_resource(&belief, "file", "readme.md");
        assert!(found.is_some());
        assert_eq!(found.unwrap().resource_ref.resource_id, "readme.md");

        let not_found = rt.query_resource(&belief, "file", "missing.txt");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_world_hash_deterministic() {
        let rt = make_runtime();
        let belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let h1 = rt.compute_world_hash(&belief);
        let h2 = rt.compute_world_hash(&belief);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex is 64 chars.
    }

    #[test]
    fn test_world_hash_changes_on_mutation() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();
        let h1 = rt.compute_world_hash(&belief);

        let patch = BeliefPatch {
            added_resources: vec![make_resource("file", "readme.md", 1)],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();
        let h2 = rt.compute_world_hash(&belief);

        assert_ne!(h1, h2);
    }

    #[test]
    fn test_checkpoint_restore_roundtrip() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![make_resource("file", "readme.md", 1)],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![make_fact("f1", "system", "initialized")],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![Binding {
                name: "mode".to_string(),
                value: serde_json::json!("production"),
                source: "config".to_string(),
                confidence: 1.0,
            }],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        let data = rt.checkpoint(&belief).unwrap();
        let restored = rt.restore(&data).unwrap();

        assert_eq!(restored.belief_id, belief.belief_id);
        assert_eq!(restored.session_id, belief.session_id);
        assert_eq!(restored.resources.len(), 1);
        assert_eq!(restored.facts.len(), 1);
        assert_eq!(restored.active_bindings.len(), 1);
        assert_eq!(restored.world_hash, belief.world_hash);
    }

    #[test]
    fn test_fact_provenance_types() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let facts = vec![
            Fact {
                fact_id: "f1".to_string(),
                subject: "sensor:temp".to_string(),
                predicate: "reading".to_string(),
                value: serde_json::json!(22.5),
                confidence: 0.95,
                provenance: FactProvenance::Observed,
                timestamp: Utc::now(),
                ttl_ms: None,
            },
            Fact {
                fact_id: "f2".to_string(),
                subject: "sensor:temp".to_string(),
                predicate: "trend".to_string(),
                value: serde_json::json!("rising"),
                confidence: 0.7,
                provenance: FactProvenance::Inferred,
                timestamp: Utc::now(),
                ttl_ms: None,
            },
            Fact {
                fact_id: "f3".to_string(),
                subject: "sensor:temp".to_string(),
                predicate: "calibrated".to_string(),
                value: serde_json::json!(true),
                confidence: 1.0,
                provenance: FactProvenance::Asserted,
                timestamp: Utc::now(),
                ttl_ms: None,
            },
            Fact {
                fact_id: "f4".to_string(),
                subject: "sensor:temp".to_string(),
                predicate: "last_reading".to_string(),
                value: serde_json::json!(21.0),
                confidence: 0.3,
                provenance: FactProvenance::Stale,
                timestamp: Utc::now(),
                ttl_ms: None,
            },
            Fact {
                fact_id: "f5".to_string(),
                subject: "sensor:temp".to_string(),
                predicate: "peer_reading".to_string(),
                value: serde_json::json!(22.8),
                confidence: 0.8,
                provenance: FactProvenance::Remote,
                timestamp: Utc::now(),
                ttl_ms: None,
            },
        ];

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: facts,
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        let temp_facts = rt.query_facts(&belief, "sensor:temp");
        assert_eq!(temp_facts.len(), 5);

        // Verify each provenance type is preserved.
        assert_eq!(temp_facts[0].provenance, FactProvenance::Observed);
        assert_eq!(temp_facts[1].provenance, FactProvenance::Inferred);
        assert_eq!(temp_facts[2].provenance, FactProvenance::Asserted);
        assert_eq!(temp_facts[3].provenance, FactProvenance::Stale);
        assert_eq!(temp_facts[4].provenance, FactProvenance::Remote);
    }

    #[test]
    fn test_duplicate_resource_rejected() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![make_resource("file", "readme.md", 1)],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        // Adding the same resource again should fail.
        let patch = BeliefPatch {
            added_resources: vec![make_resource("file", "readme.md", 1)],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        let result = rt.apply_patch(&mut belief, patch);
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_fact_rejected() {
        let rt = make_runtime();
        let mut belief = rt.create_belief(Uuid::new_v4()).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![make_fact("f1", "user:alice", "is_active")],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        rt.apply_patch(&mut belief, patch).unwrap();

        let patch = BeliefPatch {
            added_resources: vec![],
            updated_resources: vec![],
            removed_resource_ids: vec![],
            added_facts: vec![make_fact("f1", "user:alice", "is_active")],
            updated_facts: vec![],
            removed_fact_ids: vec![],
            binding_updates: vec![],
        };
        let result = rt.apply_patch(&mut belief, patch);
        assert!(result.is_err());
    }

    #[test]
    fn test_restore_invalid_data() {
        let rt = make_runtime();
        let result = rt.restore(b"not valid json");
        assert!(result.is_err());
    }
}
