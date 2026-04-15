//! Disk-backed checkpoint store for session state.
//!
//! Saves and loads serialized session checkpoints as individual JSON files
//! under `<data_dir>/sessions/<session_id>.json`. This allows sessions to
//! survive process restarts and be restored on demand.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::session::ControlSession;

/// Persists session checkpoints as JSON files on disk.
///
/// Directory layout:
///   <data_dir>/sessions/<uuid>.json
///
/// Each file contains the full serialized ControlSession.
pub struct SessionCheckpointStore {
    sessions_dir: PathBuf,
}

impl SessionCheckpointStore {
    /// Create a new store rooted at `data_dir`. The `sessions/` subdirectory
    /// is created lazily on the first write.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            sessions_dir: data_dir.join("sessions"),
        }
    }

    /// Ensure the sessions directory exists.
    fn ensure_dir(&self) -> Result<()> {
        if !self.sessions_dir.exists() {
            std::fs::create_dir_all(&self.sessions_dir).map_err(|e| {
                SomaError::Memory(format!(
                    "failed to create sessions dir {}: {}",
                    self.sessions_dir.display(),
                    e
                ))
            })?;
        }
        Ok(())
    }

    /// File path for a given session ID.
    fn path_for(&self, session_id: &Uuid) -> PathBuf {
        self.sessions_dir.join(format!("{}.json", session_id))
    }

    /// Save a checkpoint for the given session. Overwrites any existing
    /// checkpoint for the same session ID.
    pub fn save(&self, session: &ControlSession) -> Result<()> {
        self.ensure_dir()?;
        let data = serde_json::to_vec_pretty(session).map_err(SomaError::from)?;
        let path = self.path_for(&session.session_id);
        std::fs::write(&path, &data).map_err(|e| {
            SomaError::Memory(format!(
                "failed to write checkpoint {}: {}",
                path.display(),
                e
            ))
        })?;
        tracing::debug!(
            session_id = %session.session_id,
            path = %path.display(),
            "session checkpoint saved"
        );
        Ok(())
    }

    /// Load a checkpoint by session ID. Returns the deserialized session.
    pub fn load(&self, session_id: &Uuid) -> Result<ControlSession> {
        let path = self.path_for(session_id);
        if !path.exists() {
            return Err(SomaError::SessionNotFound(format!(
                "no checkpoint file for session {}",
                session_id
            )));
        }
        let data = std::fs::read(&path).map_err(|e| {
            SomaError::Memory(format!(
                "failed to read checkpoint {}: {}",
                path.display(),
                e
            ))
        })?;
        let session: ControlSession =
            serde_json::from_slice(&data).map_err(SomaError::from)?;
        tracing::debug!(
            session_id = %session.session_id,
            path = %path.display(),
            "session checkpoint loaded"
        );
        Ok(session)
    }

    /// List all session IDs that have checkpoints on disk.
    pub fn list(&self) -> Result<Vec<Uuid>> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        let entries = std::fs::read_dir(&self.sessions_dir).map_err(|e| {
            SomaError::Memory(format!(
                "failed to read sessions dir {}: {}",
                self.sessions_dir.display(),
                e
            ))
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                SomaError::Memory(format!("failed to read dir entry: {}", e))
            })?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(stem) = name_str.strip_suffix(".json")
                && let Ok(uuid) = Uuid::parse_str(stem) {
                    ids.push(uuid);
                }
        }
        Ok(ids)
    }

    /// Delete the checkpoint file for a session.
    pub fn delete(&self, session_id: &Uuid) -> Result<()> {
        let path = self.path_for(session_id);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                SomaError::Memory(format!(
                    "failed to delete checkpoint {}: {}",
                    path.display(),
                    e
                ))
            })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::types::common::Budget;
    use crate::types::goal::*;
    use crate::types::session::*;
    use crate::types::belief::BeliefState;

    fn test_session() -> ControlSession {
        let now = Utc::now();
        ControlSession {
            session_id: Uuid::new_v4(),
            goal: GoalSpec {
                goal_id: Uuid::new_v4(),
                source: GoalSource {
                    source_type: GoalSourceType::User,
                    identity: Some("test".into()),
                    session_id: None,
                    peer_id: None,
                },
                objective: Objective {
                    description: "checkpoint test".into(),
                    structured: None,
                },
                constraints: Vec::new(),
                success_conditions: Vec::new(),
                risk_budget: 0.5,
                latency_budget_ms: 10_000,
                resource_budget: 1.0,
                deadline: None,
                permissions_scope: Vec::new(),
                priority: Priority::Normal,
            },
            belief: BeliefState {
                belief_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                resources: Vec::new(),
                facts: Vec::new(),
                uncertainties: Vec::new(),
                provenance: Vec::new(),
                active_bindings: Vec::new(),
                world_hash: String::new(),
                updated_at: now,
            },
            working_memory: WorkingMemory {
                active_bindings: Vec::new(),
                unresolved_slots: Vec::new(),
                current_subgoal: None,
                recent_observations: Vec::new(),
                candidate_shortlist: Vec::new(),
                current_branch_state: None,
                budget_deltas: Vec::new(),
                output_bindings: Vec::new(),
                active_plan: None,
                active_steps: None,
                plan_step: 0,
                plan_stack: Vec::new(),
                used_plan_following: false,
                active_policy_scope: None,
            },
            status: SessionStatus::Completed,
            trace: SessionTrace { steps: Vec::new() },
            budget_remaining: Budget {
                risk_remaining: 0.3,
                latency_remaining_ms: 5000,
                resource_remaining: 0.7,
                steps_remaining: 50,
            },
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("soma_checkpoint_test_roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        let store = SessionCheckpointStore::new(&dir);
        let session = test_session();
        let sid = session.session_id;

        store.save(&session).unwrap();
        let loaded = store.load(&sid).unwrap();

        assert_eq!(loaded.session_id, sid);
        assert_eq!(loaded.status, SessionStatus::Completed);
        assert_eq!(
            loaded.goal.objective.description,
            "checkpoint test"
        );
        assert!(
            (loaded.budget_remaining.risk_remaining - 0.3).abs() < f64::EPSILON
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_nonexistent_returns_not_found() {
        let dir = std::env::temp_dir().join("soma_checkpoint_test_missing");
        let _ = std::fs::remove_dir_all(&dir);

        let store = SessionCheckpointStore::new(&dir);
        let result = store.load(&Uuid::new_v4());
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_checkpoints() {
        let dir = std::env::temp_dir().join("soma_checkpoint_test_list");
        let _ = std::fs::remove_dir_all(&dir);

        let store = SessionCheckpointStore::new(&dir);

        // Empty before any saves.
        assert!(store.list().unwrap().is_empty());

        let s1 = test_session();
        let s2 = test_session();
        store.save(&s1).unwrap();
        store.save(&s2).unwrap();

        let ids = store.list().unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&s1.session_id));
        assert!(ids.contains(&s2.session_id));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_checkpoint() {
        let dir = std::env::temp_dir().join("soma_checkpoint_test_delete");
        let _ = std::fs::remove_dir_all(&dir);

        let store = SessionCheckpointStore::new(&dir);
        let session = test_session();
        let sid = session.session_id;

        store.save(&session).unwrap();
        assert!(store.load(&sid).is_ok());

        store.delete(&sid).unwrap();
        assert!(store.load(&sid).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_overwrites_existing() {
        let dir = std::env::temp_dir().join("soma_checkpoint_test_overwrite");
        let _ = std::fs::remove_dir_all(&dir);

        let store = SessionCheckpointStore::new(&dir);
        let mut session = test_session();
        let sid = session.session_id;

        store.save(&session).unwrap();
        let loaded1 = store.load(&sid).unwrap();
        assert_eq!(loaded1.status, SessionStatus::Completed);

        session.status = SessionStatus::Failed;
        store.save(&session).unwrap();
        let loaded2 = store.load(&sid).unwrap();
        assert_eq!(loaded2.status, SessionStatus::Failed);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_empty_dir() {
        let dir = std::env::temp_dir().join("soma_checkpoint_test_empty");
        let _ = std::fs::remove_dir_all(&dir);
        let store = SessionCheckpointStore::new(&dir);
        assert!(store.list().unwrap().is_empty());
    }
}
