//! SOMA Timer Plugin -- timer/scheduler conventions for the SOMA runtime.
//!
//! Four conventions:
//!
//! | ID | Name            | Description                                         |
//! |----|-----------------|-----------------------------------------------------|
//! | 0  | `set_timeout`   | Set a one-shot timer that fires after a delay       |
//! | 1  | `set_interval`  | Set a recurring timer that fires at regular intervals|
//! | 2  | `cancel`        | Cancel an active timer by handle                    |
//! | 3  | `list_active`   | List all active timers with remaining time          |
//!
//! This plugin is a pure state machine -- it stores timer entries in a
//! `HashMap<u64, TimerEntry>` keyed by monotonic handle IDs and uses
//! `std::time::Instant` for all timing.  No async runtime is needed.
//!
//! The actual firing of timers is handled by the SOMA runtime polling
//! `check_expired()`.  This plugin tracks creation, cancellation, and
//! expiry bookkeeping.

use soma_plugin_sdk::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Timer types
// ---------------------------------------------------------------------------

/// Whether a timer fires once or repeatedly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerType {
    /// Fires once after the delay, then expires.
    Timeout,
    /// Fires repeatedly at the given interval.
    Interval,
}

/// A single timer entry tracked by the plugin.
#[derive(Debug, Clone)]
pub struct TimerEntry {
    /// Unique handle for this timer.
    pub handle: u64,
    /// Human-readable label (e.g., "reminder", "heartbeat").
    pub label: String,
    /// Whether this is a one-shot timeout or recurring interval.
    timer_type: TimerType,
    /// Interval in milliseconds (for both timeout delay and interval period).
    pub interval_ms: u64,
    /// When this timer was created (retained for runtime diagnostics).
    pub created_at: Instant,
    /// When this timer should next fire.
    pub next_fire: Instant,
}

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The SOMA timer/scheduler plugin.
///
/// Internal state is behind a `Mutex` because `SomaPlugin::execute` takes
/// `&self` (not `&mut self`), yet we need to mutate the timer map.
pub struct TimerPlugin {
    state: Mutex<TimerState>,
}

/// Mutable interior state guarded by the plugin's `Mutex`.
struct TimerState {
    /// All active timers, keyed by handle.
    timers: HashMap<u64, TimerEntry>,
    /// Monotonically increasing counter for handle generation.
    next_handle: u64,
}

impl TimerPlugin {
    /// Create a new `TimerPlugin` with empty state.
    fn new() -> Self {
        Self {
            state: Mutex::new(TimerState {
                timers: HashMap::new(),
                next_handle: 1,
            }),
        }
    }

    /// Check for expired timers and return them.
    ///
    /// - One-shot timeouts are removed from the map.
    /// - Intervals have their `next_fire` advanced by one period.
    ///
    /// The SOMA runtime calls this during its tick loop to discover which
    /// timers have fired.
    pub fn check_expired(&self) -> Vec<TimerEntry> {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        let mut expired = Vec::new();
        let mut to_remove = Vec::new();

        for (handle, entry) in state.timers.iter_mut() {
            if now >= entry.next_fire {
                expired.push(entry.clone());
                match entry.timer_type {
                    TimerType::Timeout => {
                        to_remove.push(*handle);
                    }
                    TimerType::Interval => {
                        // Advance to the next fire time, skipping missed beats
                        let elapsed = now.duration_since(entry.next_fire);
                        let periods_missed =
                            elapsed.as_millis() / u128::from(entry.interval_ms) + 1;
                        entry.next_fire += std::time::Duration::from_millis(
                            entry.interval_ms * periods_missed as u64,
                        );
                    }
                }
            }
        }

        for handle in to_remove {
            state.timers.remove(&handle);
        }

        expired
    }
}

// ---------------------------------------------------------------------------
// SomaPlugin implementation
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_literal_bound)]
impl SomaPlugin for TimerPlugin {
    fn name(&self) -> &str {
        "timer"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Timer/scheduler: timeouts, intervals, cancellation, and active timer listing"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::BuiltIn
    }

    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: set_timeout
            Convention {
                id: 0,
                name: "set_timeout".into(),
                description: "Set a one-shot timer that fires after a delay".into(),
                call_pattern: "set_timeout(label, delay_ms)".into(),
                args: vec![
                    ArgSpec {
                        name: "label".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Human-readable label for this timer".into(),
                    },
                    ArgSpec {
                        name: "delay_ms".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Delay in milliseconds before the timer fires".into(),
                    },
                ],
                returns: ReturnSpec::Value("Int".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![SideEffect("creates timer".into())],
                cleanup: None,
            },
            // 1: set_interval
            Convention {
                id: 1,
                name: "set_interval".into(),
                description: "Set a recurring timer that fires at regular intervals".into(),
                call_pattern: "set_interval(label, interval_ms)".into(),
                args: vec![
                    ArgSpec {
                        name: "label".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Human-readable label for this interval".into(),
                    },
                    ArgSpec {
                        name: "interval_ms".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Interval in milliseconds between firings".into(),
                    },
                ],
                returns: ReturnSpec::Value("Int".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![SideEffect("creates timer".into())],
                cleanup: None,
            },
            // 2: cancel
            Convention {
                id: 2,
                name: "cancel".into(),
                description: "Cancel an active timer by handle".into(),
                call_pattern: "cancel(handle)".into(),
                args: vec![ArgSpec {
                    name: "handle".into(),
                    arg_type: ArgType::Int,
                    required: true,
                    description: "Handle of the timer to cancel".into(),
                }],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![SideEffect("removes timer".into())],
                cleanup: None,
            },
            // 3: list_active
            Convention {
                id: 3,
                name: "list_active".into(),
                description: "List all active (non-expired) timers with remaining time".into(),
                call_pattern: "list_active()".into(),
                args: vec![],
                returns: ReturnSpec::Value("List".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.set_timeout(&args),
            1 => self.set_interval(&args),
            2 => self.cancel(&args),
            3 => self.list_active(),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention_id: {convention_id}"
            ))),
        }
    }

    fn checkpoint_state(&self) -> Option<serde_json::Value> {
        let state = self.state.lock().unwrap();
        let timers: Vec<serde_json::Value> = state
            .timers
            .values()
            .map(|entry| {
                serde_json::json!({
                    "handle": entry.handle,
                    "label": entry.label,
                    "type": match entry.timer_type {
                        TimerType::Timeout => "timeout",
                        TimerType::Interval => "interval",
                    },
                    "interval_ms": entry.interval_ms,
                })
            })
            .collect();
        Some(serde_json::json!({
            "timers": timers,
            "next_handle": state.next_handle,
        }))
    }
}

// ---------------------------------------------------------------------------
// Convention implementations
// ---------------------------------------------------------------------------

impl TimerPlugin {
    /// Convention 0 -- Set a one-shot timeout timer.
    ///
    /// Returns the handle (Int) of the newly created timer.
    fn set_timeout(&self, args: &[Value]) -> Result<Value, PluginError> {
        let label = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: label".into()))?
            .as_str()?
            .to_string();
        let delay_ms = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: delay_ms".into()))?
            .as_int()?;

        if delay_ms <= 0 {
            return Err(PluginError::InvalidArg(
                "delay_ms must be positive".into(),
            ));
        }

        #[allow(clippy::cast_sign_loss)]
        let delay_ms = delay_ms as u64;
        let now = Instant::now();

        let mut state = self.state.lock().unwrap();
        let handle = state.next_handle;
        state.next_handle += 1;

        state.timers.insert(
            handle,
            TimerEntry {
                handle,
                label,
                timer_type: TimerType::Timeout,
                interval_ms: delay_ms,
                created_at: now,
                next_fire: now + std::time::Duration::from_millis(delay_ms),
            },
        );

        #[allow(clippy::cast_possible_wrap)]
        Ok(Value::Int(handle as i64))
    }

    /// Convention 1 -- Set a recurring interval timer.
    ///
    /// Returns the handle (Int) of the newly created timer.
    fn set_interval(&self, args: &[Value]) -> Result<Value, PluginError> {
        let label = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: label".into()))?
            .as_str()?
            .to_string();
        let interval_ms = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: interval_ms".into()))?
            .as_int()?;

        if interval_ms <= 0 {
            return Err(PluginError::InvalidArg(
                "interval_ms must be positive".into(),
            ));
        }

        #[allow(clippy::cast_sign_loss)]
        let interval_ms = interval_ms as u64;
        let now = Instant::now();

        let mut state = self.state.lock().unwrap();
        let handle = state.next_handle;
        state.next_handle += 1;

        state.timers.insert(
            handle,
            TimerEntry {
                handle,
                label,
                timer_type: TimerType::Interval,
                interval_ms,
                created_at: now,
                next_fire: now + std::time::Duration::from_millis(interval_ms),
            },
        );

        #[allow(clippy::cast_possible_wrap)]
        Ok(Value::Int(handle as i64))
    }

    /// Convention 2 -- Cancel an active timer by handle.
    ///
    /// Returns `Bool(true)` if the timer existed and was removed,
    /// `Bool(false)` if no timer with that handle was found.
    fn cancel(&self, args: &[Value]) -> Result<Value, PluginError> {
        let handle = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: handle".into()))?
            .as_int()?;

        #[allow(clippy::cast_sign_loss)]
        let handle = handle as u64;

        let mut state = self.state.lock().unwrap();
        let removed = state.timers.remove(&handle).is_some();

        Ok(Value::Bool(removed))
    }

    /// Convention 3 -- List all active (non-expired) timers.
    ///
    /// Returns a `List` of `Map` values, each containing:
    /// - `handle`: Int -- the timer handle
    /// - `label`: String -- the human-readable label
    /// - `type`: String -- "timeout" or "interval"
    /// - `remaining_ms`: Int -- milliseconds until next fire (0 if overdue)
    fn list_active(&self) -> Result<Value, PluginError> {
        let state = self.state.lock().unwrap();
        let now = Instant::now();

        let mut entries: Vec<Value> = state
            .timers
            .values()
            .map(|entry| {
                let remaining_ms = if entry.next_fire > now {
                    entry.next_fire.duration_since(now).as_millis() as i64
                } else {
                    0
                };

                let mut map = HashMap::new();
                map.insert("handle".into(), Value::Int(entry.handle as i64));
                map.insert("label".into(), Value::String(entry.label.clone()));
                map.insert(
                    "type".into(),
                    Value::String(
                        match entry.timer_type {
                            TimerType::Timeout => "timeout",
                            TimerType::Interval => "interval",
                        }
                        .into(),
                    ),
                );
                map.insert("remaining_ms".into(), Value::Int(remaining_ms));

                Value::Map(map)
            })
            .collect();

        // Sort by handle for deterministic output
        entries.sort_by(|a, b| {
            let ha = match a {
                Value::Map(m) => match m.get("handle") {
                    Some(Value::Int(n)) => *n,
                    _ => 0,
                },
                _ => 0,
            };
            let hb = match b {
                Value::Map(m) => match m.get("handle") {
                    Some(Value::Int(n)) => *n,
                    _ => 0,
                },
                _ => 0,
            };
            ha.cmp(&hb)
        });

        Ok(Value::List(entries))
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

/// Create a heap-allocated `TimerPlugin` and return a raw pointer for dynamic loading.
///
/// Called by the SOMA runtime's `libloading`-based plugin loader.  The runtime
/// takes ownership of the pointer and drops it on unload.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(TimerPlugin::new()))
}
