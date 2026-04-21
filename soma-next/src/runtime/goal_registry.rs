//! Registry and background executor for asynchronous goals.
//!
//! An "async goal" is one that the caller submits via
//! `create_goal_async` and then polls via `get_goal_status` — the MCP
//! request returns immediately with a goal_id while a background thread
//! walks the 16-step control loop to completion.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde::Serialize;
use uuid::Uuid;

use crate::memory::checkpoint::SessionCheckpointStore;
use crate::memory::embedder::GoalEmbedder;
use crate::memory::episodes::EpisodeStore;
use crate::memory::routines::RoutineStore;
use crate::memory::schemas::SchemaStore;
use crate::runtime::goal_executor::EpisodeContext;
use crate::runtime::session::{SessionController, SessionRuntime, StepResult};
use crate::runtime::world_state::WorldStateStore;
use crate::types::session::{ControlSession, SessionStatus};

/// Public status of an asynchronously-executing goal.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsyncGoalStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Aborted,
    WaitingForInput,
    WaitingForRemote,
    Error,
}

/// One tracked async goal: the shared session, a run/cancel flag, and the
/// last observed status.
pub struct AsyncGoalEntry {
    pub goal_id: Uuid,
    pub session_id: Uuid,
    pub status: Arc<Mutex<AsyncGoalStatus>>,
    pub session: Arc<Mutex<ControlSession>>,
    pub cancel: Arc<AtomicBool>,
    pub error: Arc<Mutex<Option<String>>>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl AsyncGoalEntry {
    pub fn new(goal_id: Uuid, session: ControlSession) -> Self {
        let session_id = session.session_id;
        Self {
            goal_id,
            session_id,
            status: Arc::new(Mutex::new(AsyncGoalStatus::Pending)),
            session: Arc::new(Mutex::new(session)),
            cancel: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
            join: Mutex::new(None),
        }
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn current_status(&self) -> AsyncGoalStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn last_error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

/// Tracks every async goal the runtime has accepted. Bounded retention is
/// left to the caller (typically by pruning terminal entries on query).
pub struct GoalRegistry {
    entries: Mutex<HashMap<Uuid, Arc<AsyncGoalEntry>>>,
}

impl GoalRegistry {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn insert(&self, entry: Arc<AsyncGoalEntry>) {
        self.entries
            .lock()
            .unwrap()
            .insert(entry.goal_id, entry);
    }

    pub fn get(&self, goal_id: &Uuid) -> Option<Arc<AsyncGoalEntry>> {
        self.entries.lock().unwrap().get(goal_id).cloned()
    }

    pub fn list(&self) -> Vec<Uuid> {
        self.entries
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect()
    }

    pub fn remove(&self, goal_id: &Uuid) -> Option<Arc<AsyncGoalEntry>> {
        self.entries.lock().unwrap().remove(goal_id)
    }
}

impl Default for GoalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Owned references to the stores needed to finalize an episode from a
/// background thread. Mirrors `goal_executor::EpisodeContext` but holds
/// `Arc` clones rather than lifetime-bound borrows so it can cross a
/// thread boundary.
#[derive(Clone)]
pub struct OwnedEpisodeContext {
    pub episode_store: Arc<Mutex<dyn EpisodeStore + Send>>,
    pub schema_store: Arc<Mutex<dyn SchemaStore + Send>>,
    pub routine_store: Arc<Mutex<dyn RoutineStore + Send>>,
    pub embedder: Arc<dyn GoalEmbedder + Send + Sync>,
    pub world_state: Arc<Mutex<dyn WorldStateStore + Send>>,
    pub skill_stats: Option<crate::memory::skill_stats::SharedSkillStats>,
}

impl OwnedEpisodeContext {
    fn as_borrowed(&self) -> EpisodeContext<'_> {
        EpisodeContext {
            episode_store: &self.episode_store,
            schema_store: &self.schema_store,
            routine_store: &self.routine_store,
            embedder: &self.embedder,
            world_state: &self.world_state,
            skill_stats: self.skill_stats.as_ref(),
        }
    }
}

/// Spawn a background thread that drives `entry`'s session to terminal
/// state. The thread:
///   - re-acquires the controller lock per step (so other MCP work can
///     progress between steps),
///   - checks the cancel flag between steps,
///   - writes a checkpoint every `checkpoint_every_n` steps and once more
///     before exit,
///   - finalizes the episode (stores + attempts learning) on any terminal
///     outcome including errors.
///
/// The join handle is retained inside the entry; callers that need to
/// wait can drain it.
pub fn spawn_async_goal(
    entry: Arc<AsyncGoalEntry>,
    session_controller: Arc<Mutex<SessionController>>,
    checkpoint_store: Arc<SessionCheckpointStore>,
    checkpoint_every_n: u32,
    episode_ctx: OwnedEpisodeContext,
) {
    let handle = std::thread::Builder::new()
        .name(format!("soma-goal-{}", entry.goal_id))
        .spawn({
            let entry = Arc::clone(&entry);
            move || {
                *entry.status.lock().unwrap() = AsyncGoalStatus::Running;
                let mut steps_since_save: u32 = 0;

                loop {
                    if entry.cancel.load(Ordering::Relaxed) {
                        let mut session = entry.session.lock().unwrap();
                        session.status = SessionStatus::Aborted;
                        *entry.status.lock().unwrap() = AsyncGoalStatus::Aborted;
                        break;
                    }

                    let step_result = {
                        let mut ctrl = match session_controller.lock() {
                            Ok(c) => c,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        let mut session = entry.session.lock().unwrap();
                        ctrl.run_step(&mut session)
                    };

                    match step_result {
                        Ok(StepResult::Continue) => {
                            if checkpoint_every_n > 0 {
                                steps_since_save = steps_since_save.saturating_add(1);
                                if steps_since_save >= checkpoint_every_n {
                                    let snapshot = entry.session.lock().unwrap().clone();
                                    if let Err(e) = checkpoint_store.save(&snapshot) {
                                        tracing::warn!(
                                            goal_id = %entry.goal_id,
                                            error = %e,
                                            "async goal checkpoint failed"
                                        );
                                    }
                                    steps_since_save = 0;
                                }
                            }
                            continue;
                        }
                        Ok(StepResult::Completed) => {
                            *entry.error.lock().unwrap() = None;
                            *entry.status.lock().unwrap() = AsyncGoalStatus::Completed;
                            break;
                        }
                        Ok(StepResult::Failed(reason)) => {
                            *entry.error.lock().unwrap() = Some(reason);
                            *entry.status.lock().unwrap() = AsyncGoalStatus::Failed;
                            break;
                        }
                        Ok(StepResult::Aborted) => {
                            *entry.status.lock().unwrap() = AsyncGoalStatus::Aborted;
                            break;
                        }
                        Ok(StepResult::WaitingForInput(msg)) => {
                            *entry.error.lock().unwrap() = Some(msg);
                            *entry.status.lock().unwrap() = AsyncGoalStatus::WaitingForInput;
                            break;
                        }
                        Ok(StepResult::WaitingForRemote(msg)) => {
                            *entry.error.lock().unwrap() = Some(msg);
                            *entry.status.lock().unwrap() = AsyncGoalStatus::WaitingForRemote;
                            break;
                        }
                        Err(e) => {
                            *entry.error.lock().unwrap() = Some(e.to_string());
                            *entry.status.lock().unwrap() = AsyncGoalStatus::Error;
                            break;
                        }
                    }
                }

                // Final checkpoint + episode finalization.
                let snapshot = entry.session.lock().unwrap().clone();
                if let Err(e) = checkpoint_store.save(&snapshot) {
                    tracing::warn!(
                        goal_id = %entry.goal_id,
                        error = %e,
                        "async goal final checkpoint failed"
                    );
                }
                let borrowed = episode_ctx.as_borrowed();
                crate::runtime::goal_executor::finalize_episode(&snapshot, &borrowed);
            }
        });

    match handle {
        Ok(h) => {
            *entry.join.lock().unwrap() = Some(h);
        }
        Err(e) => {
            tracing::error!(
                goal_id = %entry.goal_id,
                error = %e,
                "failed to spawn async goal thread"
            );
            *entry.error.lock().unwrap() = Some(format!("spawn failed: {e}"));
            *entry.status.lock().unwrap() = AsyncGoalStatus::Error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_insert_get_remove() {
        let reg = GoalRegistry::new();
        // Smoke: empty registry lists nothing.
        assert!(reg.list().is_empty());
        assert!(reg.get(&Uuid::new_v4()).is_none());
        assert!(reg.remove(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn cancel_flag_sets() {
        use crate::types::belief::BeliefState;
        use crate::types::common::Budget;
        use crate::types::goal::*;
        use crate::types::session::*;
        use chrono::Utc;

        let now = Utc::now();
        let session = ControlSession {
            session_id: Uuid::new_v4(),
            goal: GoalSpec {
                goal_id: Uuid::new_v4(),
                source: GoalSource {
                    source_type: GoalSourceType::Mcp,
                    identity: None,
                    session_id: None,
                    peer_id: None,
                },
                objective: Objective {
                    description: "test".into(),
                    structured: None,
                },
                constraints: vec![],
                success_conditions: vec![],
                risk_budget: 0.5,
                latency_budget_ms: 10_000,
                resource_budget: 1.0,
                deadline: None,
                permissions_scope: vec![],
                priority: Priority::Normal,
                max_steps: None,
                exploration: crate::types::goal::ExplorationStrategy::Greedy,
            },
            belief: BeliefState {
                belief_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                resources: vec![],
                facts: vec![],
                uncertainties: vec![],
                provenance: vec![],
                active_bindings: vec![],
                world_hash: String::new(),
                updated_at: now,
            },
            working_memory: WorkingMemory {
                active_bindings: vec![],
                unresolved_slots: vec![],
                current_subgoal: None,
                recent_observations: vec![],
                candidate_shortlist: vec![],
                current_branch_state: None,
                budget_deltas: vec![],
                output_bindings: vec![],
                active_plan: None,
                active_steps: None,
                plan_step: 0,
                plan_stack: vec![],
                used_plan_following: false,
                active_policy_scope: None,
                loop_counts: std::collections::HashMap::new(),
                pending_input_request: None,
            },
            status: SessionStatus::Created,
            trace: SessionTrace { steps: vec![] },
            budget_remaining: Budget {
                risk_remaining: 0.5,
                latency_remaining_ms: 10_000,
                resource_remaining: 1.0,
                steps_remaining: 100,
            },
            created_at: now,
            updated_at: now,
        };
        let entry = Arc::new(AsyncGoalEntry::new(Uuid::new_v4(), session));
        assert_eq!(entry.current_status(), AsyncGoalStatus::Pending);
        entry.request_cancel();
        assert!(entry.cancel.load(Ordering::Relaxed));
    }
}
