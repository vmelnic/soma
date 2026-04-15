//! WorldState: stores facts about the external world and produces JSON snapshots
//! for routine precondition matching.
//!
//! The reactive monitor thread polls this state at a configurable interval,
//! compares the snapshot hash to the previous tick, and fires any autonomous
//! routines whose match_conditions are newly satisfied.

use std::collections::HashMap;
#[cfg(feature = "native")]
use std::collections::HashSet;
#[cfg(feature = "native")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "native")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "native")]
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::errors::Result;
use crate::types::belief::Fact;

// ---------------------------------------------------------------------------
// WorldStateStore trait
// ---------------------------------------------------------------------------

pub trait WorldStateStore: Send {
    /// Produce a flat JSON object where each key is `"{subject}.{predicate}"`
    /// and each value is the fact's `value`. Directly compatible with
    /// `precondition_matches()` in routines.rs.
    fn snapshot(&self) -> serde_json::Value;

    /// Add a new fact. If a fact with the same `fact_id` already exists it is
    /// replaced (upsert semantics).
    fn add_fact(&mut self, fact: Fact) -> Result<()>;

    /// Remove a fact by its `fact_id`. Returns `Ok(true)` if found and removed,
    /// `Ok(false)` if not present.
    fn remove_fact(&mut self, fact_id: &str) -> Result<bool>;

    /// Update an existing fact (same as `add_fact` with upsert semantics).
    fn update_fact(&mut self, fact: Fact) -> Result<()>;

    /// Return references to all stored facts.
    fn list_facts(&self) -> Vec<&Fact>;

    /// SHA-256 hex digest of the serialized snapshot. Used by the reactive
    /// monitor to detect world-state changes between ticks.
    fn snapshot_hash(&self) -> String;
}

// ---------------------------------------------------------------------------
// DefaultWorldStateStore
// ---------------------------------------------------------------------------

pub struct DefaultWorldStateStore {
    facts: HashMap<String, Fact>,
}

impl DefaultWorldStateStore {
    pub fn new() -> Self {
        Self {
            facts: HashMap::new(),
        }
    }
}

impl Default for DefaultWorldStateStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldStateStore for DefaultWorldStateStore {
    fn snapshot(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for fact in self.facts.values() {
            let key = format!("{}.{}", fact.subject, fact.predicate);
            map.insert(key, fact.value.clone());
        }
        serde_json::Value::Object(map)
    }

    fn add_fact(&mut self, fact: Fact) -> Result<()> {
        self.facts.insert(fact.fact_id.clone(), fact);
        Ok(())
    }

    fn remove_fact(&mut self, fact_id: &str) -> Result<bool> {
        Ok(self.facts.remove(fact_id).is_some())
    }

    fn update_fact(&mut self, fact: Fact) -> Result<()> {
        self.facts.insert(fact.fact_id.clone(), fact);
        Ok(())
    }

    fn list_facts(&self) -> Vec<&Fact> {
        self.facts.values().collect()
    }

    fn snapshot_hash(&self) -> String {
        let snapshot = self.snapshot();
        let serialized = serde_json::to_string(&snapshot).unwrap_or_default();
        let hash = Sha256::digest(serialized.as_bytes());
        format!("{hash:x}")
    }
}

// ---------------------------------------------------------------------------
// Reactive monitor thread (native only)
// ---------------------------------------------------------------------------

#[cfg(feature = "native")]
#[allow(clippy::too_many_arguments)]
pub fn start_reactive_monitor(
    world_state: Arc<Mutex<dyn WorldStateStore + Send>>,
    routine_store: Arc<Mutex<dyn crate::memory::routines::RoutineStore + Send>>,
    session_controller: Arc<Mutex<crate::runtime::session::SessionController>>,
    goal_runtime: Arc<Mutex<crate::runtime::goal::DefaultGoalRuntime>>,
    episode_store: Arc<Mutex<dyn crate::memory::episodes::EpisodeStore + Send>>,
    embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync>,
    interval_secs: u64,
) -> JoinHandle<()> {
    thread::spawn(move || {
        use crate::runtime::goal::GoalRuntime;
        use crate::runtime::session::SessionRuntime;

        let mut last_hash = String::new();
        let mut fired_set: HashSet<String> = HashSet::new();
        // Track consecutive failure count per routine for feedback loop.
        // After max_consecutive_failures, reduce confidence. Below
        // confidence_invalidation_threshold, invalidate the routine.
        let mut failure_counts: HashMap<String, u32> = HashMap::new();
        let max_consecutive_failures: u32 = 3;
        let confidence_decay: f64 = 0.7; // multiply confidence by this on failure
        let confidence_invalidation_threshold: f64 = 0.3;

        loop {
            thread::sleep(Duration::from_secs(interval_secs));

            // 1. Snapshot the world state.
            let (snapshot, hash) = {
                let ws = world_state.lock().unwrap();
                (ws.snapshot(), ws.snapshot_hash())
            };

            // 2. If the hash changed, clear the fired set so routines can
            //    re-fire under the new world state.
            if hash != last_hash {
                fired_set.clear();
                last_hash = hash;
            }

            // 3. Find matching autonomous routines.
            let matching_routines: Vec<crate::types::routine::Routine> = {
                let rs = routine_store.lock().unwrap();
                rs.find_matching(&snapshot)
                    .into_iter()
                    .filter(|r| r.autonomous && !fired_set.contains(&r.routine_id))
                    .cloned()
                    .collect()
            };

            // 4. Apply exclusive handling: if the highest-priority match
            //    is exclusive, only fire that one.
            let routines_to_fire = if matching_routines.first().is_some_and(|r| r.exclusive) {
                matching_routines.into_iter().take(1).collect::<Vec<_>>()
            } else {
                matching_routines
            };

            for routine in routines_to_fire {
                let routine_id = routine.routine_id.clone();
                let label = format!("reactive: {routine_id}");

                // Build a GoalInput.
                let source = crate::types::goal::GoalSource {
                    source_type: crate::types::goal::GoalSourceType::Internal,
                    identity: None,
                    session_id: None,
                    peer_id: None,
                };
                let input = crate::runtime::goal::GoalInput::NaturalLanguage {
                    text: label.clone(),
                    source,
                };

                // Parse the goal.
                let goal = {
                    let goal_rt = goal_runtime.lock().unwrap();
                    match goal_rt.parse_goal(input) {
                        Ok(g) => g,
                        Err(e) => {
                            let event = serde_json::json!({
                                "_reactive_event": true,
                                "routine_id": routine_id,
                                "label": label,
                                "success": false,
                                "error": format!("goal parse failed: {e}"),
                                "steps": 0,
                            });
                            eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());
                            continue;
                        }
                    }
                };

                // Create session, inject plan, step until done.
                let (success, steps) = {
                    let mut ctrl = session_controller.lock().unwrap();
                    match ctrl.create_session(goal) {
                        Ok(mut session) => {
                            // Pre-load the routine's steps.
                            let steps = routine.effective_steps();
                            if !steps.is_empty() {
                                session.working_memory.active_steps = Some(steps);
                            } else {
                                session.working_memory.active_plan =
                                    Some(routine.compiled_skill_path.clone());
                            }
                            session.working_memory.plan_step = 0;
                            session.working_memory.used_plan_following = true;

                            let mut final_success = false;
                            loop {
                                use crate::runtime::session::StepResult;
                                match ctrl.run_step(&mut session) {
                                    Ok(StepResult::Continue) => continue,
                                    Ok(StepResult::Completed) => {
                                        final_success = true;
                                        break;
                                    }
                                    Ok(_) | Err(_) => break,
                                }
                            }

                            let step_count = session.trace.steps.len();

                            // Store episode on failure (same pattern as handle_execute_routine).
                            if !final_success {
                                let mut episode =
                                    crate::interfaces::cli::build_episode_from_session(
                                        &session,
                                        Some(&*embedder),
                                    );
                                episode.world_state_context = world_state.lock().ok()
                                    .map(|ws| ws.snapshot())
                                    .unwrap_or(serde_json::json!({}));
                                let adapter = crate::adapters::EpisodeMemoryAdapter::new(
                                    Arc::clone(&episode_store),
                                    Arc::clone(&embedder),
                                );
                                if let Err(e) = adapter.store(episode) {
                                    tracing::warn!(
                                        error = %e,
                                        "reactive monitor: failed to store episode"
                                    );
                                }
                            }

                            (final_success, step_count)
                        }
                        Err(e) => {
                            // Emit failure fact for session creation failure.
                            if let Ok(mut ws) = world_state.lock() {
                                let fact = crate::types::belief::Fact {
                                    fact_id: format!("routine_failure_{routine_id}"),
                                    subject: "routine".to_string(),
                                    predicate: format!("{routine_id}.last_failure"),
                                    value: serde_json::json!({
                                        "timestamp": chrono::Utc::now().to_rfc3339(),
                                        "error": format!("session creation failed: {e}"),
                                        "steps": 0,
                                    }),
                                    confidence: 1.0,
                                    provenance: crate::types::common::FactProvenance::Observed,
                                    timestamp: chrono::Utc::now(),
                                };
                                let _ = ws.add_fact(fact);
                            }
                            let event = serde_json::json!({
                                "_reactive_event": true,
                                "routine_id": routine_id,
                                "label": label,
                                "success": false,
                                "error": format!("session creation failed: {e}"),
                                "steps": 0,
                            });
                            eprintln!(
                                "{}",
                                serde_json::to_string(&event).unwrap_or_default()
                            );
                            continue;
                        }
                    }
                };

                // Mark as fired so we don't re-trigger on the same world state.
                fired_set.insert(routine_id.clone());

                // Emit success/failure facts to world state for declarative alerting.
                if let Ok(mut ws) = world_state.lock() {
                    if success {
                        let fact = crate::types::belief::Fact {
                            fact_id: format!("routine_success_{routine_id}"),
                            subject: "routine".to_string(),
                            predicate: format!("{routine_id}.last_success"),
                            value: serde_json::json!({
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                                "steps": steps,
                            }),
                            confidence: 1.0,
                            provenance: crate::types::common::FactProvenance::Observed,
                            timestamp: chrono::Utc::now(),
                        };
                        let _ = ws.add_fact(fact);
                        // Clear any prior failure fact.
                        let _ = ws.remove_fact(&format!("routine_failure_{routine_id}"));
                    } else {
                        let fact = crate::types::belief::Fact {
                            fact_id: format!("routine_failure_{routine_id}"),
                            subject: "routine".to_string(),
                            predicate: format!("{routine_id}.last_failure"),
                            value: serde_json::json!({
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                                "steps": steps,
                                "error": "execution failed",
                            }),
                            confidence: 1.0,
                            provenance: crate::types::common::FactProvenance::Observed,
                            timestamp: chrono::Utc::now(),
                        };
                        let _ = ws.add_fact(fact);
                        // Clear any prior success fact.
                        let _ = ws.remove_fact(&format!("routine_success_{routine_id}"));
                    }
                }

                // Feedback loop: track consecutive failures, decay confidence,
                // auto-invalidate routines that fail too often.
                if success {
                    failure_counts.remove(&routine_id);
                } else {
                    let count = failure_counts.entry(routine_id.clone()).or_insert(0);
                    *count += 1;
                    if *count >= max_consecutive_failures {
                        // Decay confidence on the routine.
                        let mut rs = routine_store.lock().unwrap();
                        if let Some(routine) = rs.get(&routine_id).cloned() {
                            let new_confidence = routine.confidence * confidence_decay;
                            tracing::warn!(
                                routine_id = %routine_id,
                                failures = *count,
                                old_confidence = routine.confidence,
                                new_confidence = new_confidence,
                                "decaying routine confidence after consecutive failures"
                            );
                            if new_confidence < confidence_invalidation_threshold {
                                tracing::warn!(
                                    routine_id = %routine_id,
                                    "invalidating routine — confidence below threshold"
                                );
                                let _ = rs.invalidate(&routine_id);
                                failure_counts.remove(&routine_id);
                            } else {
                                // Re-register with reduced confidence.
                                let mut updated = routine;
                                updated.confidence = new_confidence;
                                let _ = rs.register(updated);
                            }
                        }
                    }
                }

                // Emit structured event to stderr for SSE consumers.
                let event = serde_json::json!({
                    "_reactive_event": true,
                    "routine_id": routine_id,
                    "label": label,
                    "success": success,
                    "steps": steps,
                });
                eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::FactProvenance;
    use chrono::Utc;

    fn make_fact(id: &str, subject: &str, predicate: &str, value: serde_json::Value) -> Fact {
        Fact {
            fact_id: id.to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            value,
            confidence: 1.0,
            provenance: FactProvenance::Observed,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_add_and_list_facts() {
        let mut store = DefaultWorldStateStore::new();
        assert!(store.list_facts().is_empty());

        let f1 = make_fact("f1", "sensor", "temperature", serde_json::json!(22.5));
        let f2 = make_fact("f2", "sensor", "humidity", serde_json::json!(45));
        store.add_fact(f1).unwrap();
        store.add_fact(f2).unwrap();

        assert_eq!(store.list_facts().len(), 2);
    }

    #[test]
    fn test_snapshot_produces_subject_predicate_keys() {
        let mut store = DefaultWorldStateStore::new();

        store
            .add_fact(make_fact(
                "f1",
                "sensor",
                "temperature",
                serde_json::json!(22.5),
            ))
            .unwrap();
        store
            .add_fact(make_fact(
                "f2",
                "door",
                "open",
                serde_json::json!(true),
            ))
            .unwrap();

        let snap = store.snapshot();
        let obj = snap.as_object().unwrap();
        assert_eq!(obj.get("sensor.temperature"), Some(&serde_json::json!(22.5)));
        assert_eq!(obj.get("door.open"), Some(&serde_json::json!(true)));
        assert_eq!(obj.len(), 2);
    }

    #[test]
    fn test_remove_fact() {
        let mut store = DefaultWorldStateStore::new();
        store
            .add_fact(make_fact("f1", "sensor", "temp", serde_json::json!(20)))
            .unwrap();
        assert_eq!(store.list_facts().len(), 1);

        assert_eq!(store.remove_fact("f1").unwrap(), true);
        assert!(store.list_facts().is_empty());

        // Removing a non-existent fact returns false.
        assert_eq!(store.remove_fact("f1").unwrap(), false);
    }

    #[test]
    fn test_snapshot_hash_changes_on_mutation() {
        let mut store = DefaultWorldStateStore::new();

        let hash_empty = store.snapshot_hash();

        store
            .add_fact(make_fact("f1", "sensor", "temp", serde_json::json!(20)))
            .unwrap();
        let hash_one = store.snapshot_hash();
        assert_ne!(hash_empty, hash_one);

        store
            .add_fact(make_fact("f2", "sensor", "humidity", serde_json::json!(50)))
            .unwrap();
        let hash_two = store.snapshot_hash();
        assert_ne!(hash_one, hash_two);

        // Removing a fact changes the hash again.
        store.remove_fact("f2").unwrap();
        let hash_after_remove = store.snapshot_hash();
        assert_eq!(hash_after_remove, hash_one);
    }

    #[test]
    fn test_update_fact_replaces_value() {
        let mut store = DefaultWorldStateStore::new();
        store
            .add_fact(make_fact("f1", "sensor", "temp", serde_json::json!(20)))
            .unwrap();

        let snap1 = store.snapshot();
        assert_eq!(
            snap1.as_object().unwrap().get("sensor.temp"),
            Some(&serde_json::json!(20))
        );

        store
            .update_fact(make_fact("f1", "sensor", "temp", serde_json::json!(25)))
            .unwrap();

        let snap2 = store.snapshot();
        assert_eq!(
            snap2.as_object().unwrap().get("sensor.temp"),
            Some(&serde_json::json!(25))
        );

        // Still only one fact.
        assert_eq!(store.list_facts().len(), 1);
    }

    #[test]
    fn test_add_fact_upserts_on_same_id() {
        let mut store = DefaultWorldStateStore::new();
        store
            .add_fact(make_fact("f1", "a", "b", serde_json::json!(1)))
            .unwrap();
        store
            .add_fact(make_fact("f1", "a", "b", serde_json::json!(2)))
            .unwrap();

        assert_eq!(store.list_facts().len(), 1);
        let snap = store.snapshot();
        assert_eq!(
            snap.as_object().unwrap().get("a.b"),
            Some(&serde_json::json!(2))
        );
    }
}
