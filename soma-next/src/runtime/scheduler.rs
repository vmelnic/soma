use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uuid::Uuid;

use crate::errors::Result;
use crate::runtime::port::{DefaultPortRuntime, PortRuntime};
use crate::types::port::InvocationContext;

// ---------------------------------------------------------------------------
// Schedule types
// ---------------------------------------------------------------------------

/// What the scheduler fires: a single port capability invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleAction {
    pub port_id: String,
    pub capability_id: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

/// A scheduled task. Either interval-based (recurring) or one-shot.
/// `cron_expr` is accepted and stored but not evaluated yet — cron parsing
/// is deferred to a future iteration.
///
/// Two modes:
/// - **Port call**: `action` is Some — the scheduler invokes the port.
/// - **Message-only**: `action` is None, `message` is Some — the scheduler
///   emits the message as an SSE event without calling any port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Uuid,
    pub label: String,
    /// Fire once after N milliseconds (one-shot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    /// Interval in milliseconds (recurring). Mutually exclusive with cron_expr.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    /// Cron expression (e.g. "0 9 * * MON-FRI"). Stored for future use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_expr: Option<String>,
    /// Port invocation to fire. None for message-only schedules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ScheduleAction>,
    /// Plain message to emit (no port call). Shown directly in the
    /// operator's chat via SSE.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Maximum number of times to fire. None = unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fires: Option<u64>,
    /// Number of times this schedule has fired so far.
    #[serde(default)]
    pub fire_count: u64,
    /// When true, the parent process should route the fire result through
    /// the LLM brain for interpretation and follow-up actions.
    #[serde(default)]
    pub brain: bool,
    /// Next fire time as milliseconds since UNIX epoch.
    pub next_fire_epoch_ms: u64,
    pub created_at_epoch_ms: u64,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// ScheduleStore trait + in-memory implementation
// ---------------------------------------------------------------------------

pub trait ScheduleStore: Send {
    fn add(&mut self, schedule: Schedule) -> Result<()>;
    fn remove(&mut self, id: &Uuid) -> Result<bool>;
    fn list_all(&self) -> Vec<Schedule>;
    /// Return all enabled schedules whose `next_fire_epoch_ms <= now`.
    fn get_due(&self, now_epoch_ms: u64) -> Vec<Schedule>;
    fn update_next_fire(&mut self, id: &Uuid, next_fire_epoch_ms: u64) -> Result<()>;
    /// Increment fire_count by 1 and return the new value.
    fn increment_fire_count(&mut self, id: &Uuid) -> Result<u64>;
}

#[derive(Default)]
pub struct DefaultScheduleStore {
    schedules: HashMap<Uuid, Schedule>,
}

impl DefaultScheduleStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ScheduleStore for DefaultScheduleStore {
    fn add(&mut self, schedule: Schedule) -> Result<()> {
        self.schedules.insert(schedule.id, schedule);
        Ok(())
    }

    fn remove(&mut self, id: &Uuid) -> Result<bool> {
        Ok(self.schedules.remove(id).is_some())
    }

    fn list_all(&self) -> Vec<Schedule> {
        self.schedules.values().cloned().collect()
    }

    fn get_due(&self, now_epoch_ms: u64) -> Vec<Schedule> {
        self.schedules
            .values()
            .filter(|s| s.enabled && s.next_fire_epoch_ms <= now_epoch_ms)
            .cloned()
            .collect()
    }

    fn update_next_fire(&mut self, id: &Uuid, next_fire_epoch_ms: u64) -> Result<()> {
        if let Some(s) = self.schedules.get_mut(id) {
            s.next_fire_epoch_ms = next_fire_epoch_ms;
        }
        Ok(())
    }

    fn increment_fire_count(&mut self, id: &Uuid) -> Result<u64> {
        if let Some(s) = self.schedules.get_mut(id) {
            s.fire_count += 1;
            Ok(s.fire_count)
        } else {
            Ok(0)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Background scheduler thread
// ---------------------------------------------------------------------------

/// Spawn a background thread that fires due schedules every second.
///
/// The thread runs until the process exits. Caller should keep the returned
/// `JoinHandle` alive (or deliberately leak it) to prevent an early drop
/// from detaching the thread.
pub fn start_scheduler_thread(
    schedule_store: Arc<Mutex<dyn ScheduleStore + Send>>,
    port_runtime: Arc<Mutex<DefaultPortRuntime>>,
    world_state: Option<Arc<Mutex<dyn crate::runtime::world_state::WorldStateStore + Send>>>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("soma-scheduler".to_string())
        .spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(1));

                let now = now_epoch_ms();

                // Collect due schedules under a short lock.
                let due: Vec<Schedule> = {
                    let store = schedule_store.lock().unwrap();
                    store.get_due(now)
                };

                for schedule in due {
                    if let Some(ref action) = schedule.action {
                        // Port invocation mode.
                        let ctx = InvocationContext {
                            caller_identity: Some("scheduler".to_string()),
                            ..Default::default()
                        };

                        let result = {
                            let rt = port_runtime.lock().unwrap();
                            rt.invoke(
                                &action.port_id,
                                &action.capability_id,
                                action.input.clone(),
                                &ctx,
                            )
                        };

                        let (success, detail) = match &result {
                            Ok(record) if record.success => {
                                (true, serde_json::to_string(&record.structured_result).unwrap_or_default())
                            }
                            Ok(record) => {
                                let msg = format!("{:?}", record.failure_class);
                                (false, msg)
                            }
                            Err(e) => (false, e.to_string()),
                        };
                        eprintln!(
                            "{{\"_scheduler_event\":true,\"id\":\"{}\",\"label\":\"{}\",\"port_id\":\"{}\",\"capability_id\":\"{}\",\"success\":{},\"detail\":{},\"brain\":{}}}",
                            schedule.id,
                            schedule.label.replace('"', "\\\""),
                            action.port_id,
                            action.capability_id,
                            success,
                            if success { detail.clone() } else { format!("\"{}\"", detail.replace('"', "\\\"")) },
                            schedule.brain,
                        );

                        // Record the scheduled fire as a WorldState fact.
                        if let Some(ref ws_arc) = world_state
                            && let Ok(mut ws) = ws_arc.lock()
                        {
                            let fact = crate::types::belief::Fact {
                                fact_id: format!("scheduler.{}", schedule.id),
                                subject: "scheduler".to_string(),
                                predicate: format!("{}_fired", schedule.label),
                                value: serde_json::json!(success),
                                confidence: 1.0,
                                provenance: crate::types::common::FactProvenance::Observed,
                                timestamp: chrono::Utc::now(),
                            };
                            let _ = ws.add_fact(fact);
                        }
                    } else if let Some(ref message) = schedule.message {
                        // Message-only mode — emit directly, no port call.
                        let escaped = message.replace('"', "\\\"");
                        eprintln!(
                            "{{\"_scheduler_event\":true,\"id\":\"{}\",\"label\":\"{}\",\"message\":\"{}\",\"success\":true,\"detail\":\"{}\",\"brain\":{}}}",
                            schedule.id,
                            schedule.label.replace('"', "\\\""),
                            escaped,
                            escaped,
                            schedule.brain,
                        );
                    }

                    // Increment fire count.
                    let mut store = schedule_store.lock().unwrap();
                    let fire_count = store.increment_fire_count(&schedule.id).unwrap_or(0);

                    // Check if max_fires reached — remove instead of advancing.
                    if schedule.max_fires.is_some_and(|max| fire_count >= max) {
                        let _ = store.remove(&schedule.id);
                        continue;
                    }

                    // Advance the schedule.
                    if let Some(interval) = schedule.interval_ms {
                        let next = now + interval;
                        let _ = store.update_next_fire(&schedule.id, next);
                    } else if schedule.cron_expr.is_some() {
                        // Cron parsing is deferred. Disable so it doesn't
                        // fire every tick.
                        let _ = store.remove(&schedule.id);
                    } else {
                        // One-shot: already fired, remove it.
                        let _ = store.remove(&schedule.id);
                    }
                }
            }
        })
        .expect("failed to spawn soma-scheduler thread")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_store_add_and_list() {
        let mut store = DefaultScheduleStore::new();
        assert!(store.list_all().is_empty());

        let s = Schedule {
            id: Uuid::new_v4(),
            label: "heartbeat".to_string(),
            delay_ms: None,
            interval_ms: Some(5000),
            cron_expr: None,
            action: Some(ScheduleAction {
                port_id: "http".to_string(),
                capability_id: "get".to_string(),
                input: serde_json::json!({"url": "http://localhost/health"}),
            }),
            message: None,
            max_fires: None,
            fire_count: 0,
            brain: false,
            next_fire_epoch_ms: 1000,
            created_at_epoch_ms: 0,
            enabled: true,
        };

        store.add(s.clone()).unwrap();
        let all = store.list_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].label, "heartbeat");
    }

    #[test]
    fn test_default_store_get_due() {
        let mut store = DefaultScheduleStore::new();

        let past = Schedule {
            id: Uuid::new_v4(),
            label: "past".to_string(),
            delay_ms: None,
            interval_ms: Some(1000),
            cron_expr: None,
            action: Some(ScheduleAction {
                port_id: "p".to_string(),
                capability_id: "c".to_string(),
                input: serde_json::json!({}),
            }),
            message: None,
            max_fires: None,
            fire_count: 0,
            brain: false,
            next_fire_epoch_ms: 100,
            created_at_epoch_ms: 0,
            enabled: true,
        };

        let future = Schedule {
            id: Uuid::new_v4(),
            label: "future".to_string(),
            delay_ms: None,
            interval_ms: Some(1000),
            cron_expr: None,
            action: Some(ScheduleAction {
                port_id: "p".to_string(),
                capability_id: "c".to_string(),
                input: serde_json::json!({}),
            }),
            message: None,
            max_fires: None,
            fire_count: 0,
            brain: false,
            next_fire_epoch_ms: 9999,
            created_at_epoch_ms: 0,
            enabled: true,
        };

        let disabled = Schedule {
            id: Uuid::new_v4(),
            label: "disabled".to_string(),
            delay_ms: None,
            interval_ms: Some(1000),
            cron_expr: None,
            action: Some(ScheduleAction {
                port_id: "p".to_string(),
                capability_id: "c".to_string(),
                input: serde_json::json!({}),
            }),
            message: None,
            max_fires: None,
            fire_count: 0,
            brain: false,
            next_fire_epoch_ms: 100,
            created_at_epoch_ms: 0,
            enabled: false,
        };

        store.add(past).unwrap();
        store.add(future).unwrap();
        store.add(disabled).unwrap();

        let due = store.get_due(500);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].label, "past");
    }

    #[test]
    fn test_default_store_remove() {
        let mut store = DefaultScheduleStore::new();
        let id = Uuid::new_v4();

        let s = Schedule {
            id,
            label: "temp".to_string(),
            delay_ms: None,
            interval_ms: None,
            cron_expr: None,
            action: Some(ScheduleAction {
                port_id: "p".to_string(),
                capability_id: "c".to_string(),
                input: serde_json::json!({}),
            }),
            message: None,
            max_fires: None,
            fire_count: 0,
            brain: false,
            next_fire_epoch_ms: 0,
            created_at_epoch_ms: 0,
            enabled: true,
        };

        store.add(s).unwrap();
        assert_eq!(store.list_all().len(), 1);

        let removed = store.remove(&id).unwrap();
        assert!(removed);
        assert!(store.list_all().is_empty());

        // Removing again returns false.
        let removed_again = store.remove(&id).unwrap();
        assert!(!removed_again);
    }
}
