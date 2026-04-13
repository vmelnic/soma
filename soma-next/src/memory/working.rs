use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::session::{WorkingBinding, WorkingMemory};

/// WorkingMemoryStore — per-session transient memory operations.
/// Low latency, checkpointable, scoped to a single control session.
pub trait WorkingMemoryStore {
    /// Create a fresh working memory for a new session.
    fn create(&self, session_id: Uuid) -> WorkingMemory;

    /// Replace the active bindings in working memory.
    fn update_bindings(&self, wm: &mut WorkingMemory, bindings: Vec<WorkingBinding>);

    /// Record a new observation reference in working memory.
    fn add_observation(&self, wm: &mut WorkingMemory, observation_id: Uuid);

    /// Set or clear the current subgoal.
    fn set_subgoal(&self, wm: &mut WorkingMemory, subgoal: Option<String>);

    /// Serialize working memory to a binary checkpoint.
    fn checkpoint(&self, wm: &WorkingMemory) -> Result<Vec<u8>>;

    /// Restore working memory from a binary checkpoint.
    fn restore(&self, data: &[u8]) -> Result<WorkingMemory>;
}

/// Default in-memory implementation of WorkingMemoryStore.
pub struct DefaultWorkingMemoryStore;

impl DefaultWorkingMemoryStore {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultWorkingMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkingMemoryStore for DefaultWorkingMemoryStore {
    fn create(&self, _session_id: Uuid) -> WorkingMemory {
        WorkingMemory {
            active_bindings: Vec::new(),
            unresolved_slots: Vec::new(),
            current_subgoal: None,
            recent_observations: Vec::new(),
            candidate_shortlist: Vec::new(),
            current_branch_state: None,
            budget_deltas: Vec::new(),
            output_bindings: Vec::new(),
            active_plan: None,
            plan_step: 0,
            used_plan_following: false,
        }
    }

    fn update_bindings(&self, wm: &mut WorkingMemory, bindings: Vec<WorkingBinding>) {
        // Merge: overwrite existing bindings by name, append new ones.
        for binding in bindings {
            if let Some(existing) = wm
                .active_bindings
                .iter_mut()
                .find(|b| b.name == binding.name)
            {
                existing.value = binding.value;
                existing.source = binding.source;
            } else {
                wm.active_bindings.push(binding);
            }
        }
    }

    fn add_observation(&self, wm: &mut WorkingMemory, observation_id: Uuid) {
        wm.recent_observations.push(observation_id);
    }

    fn set_subgoal(&self, wm: &mut WorkingMemory, subgoal: Option<String>) {
        wm.current_subgoal = subgoal;
    }

    fn checkpoint(&self, wm: &WorkingMemory) -> Result<Vec<u8>> {
        serde_json::to_vec(wm).map_err(SomaError::from)
    }

    fn restore(&self, data: &[u8]) -> Result<WorkingMemory> {
        serde_json::from_slice(data).map_err(SomaError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_working_memory() {
        let store = DefaultWorkingMemoryStore::new();
        let wm = store.create(Uuid::new_v4());
        assert!(wm.active_bindings.is_empty());
        assert!(wm.recent_observations.is_empty());
        assert!(wm.current_subgoal.is_none());
    }

    #[test]
    fn test_update_bindings_insert_and_overwrite() {
        let store = DefaultWorkingMemoryStore::new();
        let mut wm = store.create(Uuid::new_v4());

        let b1 = WorkingBinding {
            name: "x".into(),
            value: serde_json::json!(1),
            source: crate::types::session::BindingSource::WorkingMemory,
        };
        store.update_bindings(&mut wm, vec![b1]);
        assert_eq!(wm.active_bindings.len(), 1);
        assert_eq!(wm.active_bindings[0].value, serde_json::json!(1));

        // Overwrite
        let b2 = WorkingBinding {
            name: "x".into(),
            value: serde_json::json!(42),
            source: crate::types::session::BindingSource::WorkingMemory,
        };
        store.update_bindings(&mut wm, vec![b2]);
        assert_eq!(wm.active_bindings.len(), 1);
        assert_eq!(wm.active_bindings[0].value, serde_json::json!(42));
    }

    #[test]
    fn test_add_observation() {
        let store = DefaultWorkingMemoryStore::new();
        let mut wm = store.create(Uuid::new_v4());
        let obs_id = Uuid::new_v4();
        store.add_observation(&mut wm, obs_id);
        assert_eq!(wm.recent_observations.len(), 1);
        assert_eq!(wm.recent_observations[0], obs_id);
    }

    #[test]
    fn test_set_subgoal() {
        let store = DefaultWorkingMemoryStore::new();
        let mut wm = store.create(Uuid::new_v4());

        store.set_subgoal(&mut wm, Some("find_file".into()));
        assert_eq!(wm.current_subgoal.as_deref(), Some("find_file"));

        store.set_subgoal(&mut wm, None);
        assert!(wm.current_subgoal.is_none());
    }

    #[test]
    fn test_checkpoint_and_restore() {
        let store = DefaultWorkingMemoryStore::new();
        let mut wm = store.create(Uuid::new_v4());
        store.update_bindings(
            &mut wm,
            vec![WorkingBinding {
                name: "key".into(),
                value: serde_json::json!("val"),
                source: crate::types::session::BindingSource::WorkingMemory,
            }],
        );
        store.set_subgoal(&mut wm, Some("goal".into()));
        let obs = Uuid::new_v4();
        store.add_observation(&mut wm, obs);

        let data = store.checkpoint(&wm).unwrap();
        let restored = store.restore(&data).unwrap();

        assert_eq!(restored.active_bindings.len(), 1);
        assert_eq!(restored.active_bindings[0].name, "key");
        assert_eq!(restored.current_subgoal.as_deref(), Some("goal"));
        assert_eq!(restored.recent_observations.len(), 1);
        assert_eq!(restored.recent_observations[0], obs);
    }
}
