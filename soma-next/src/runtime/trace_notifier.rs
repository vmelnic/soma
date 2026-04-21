//! Push-based trace notifications for async goals.
//!
//! When an async goal completes a control-loop step, the background thread
//! fires a JSON-RPC 2.0 notification through a `TraceNotifier`. This lets
//! the brain observe progress without polling `stream_goal_observations`.

use std::io::Write;
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::types::session::TraceStep;

/// Serializes a `TraceStep` into the same JSON shape that
/// `stream_goal_observations` returns per event.
pub fn serialize_trace_step(step: &TraceStep) -> Value {
    serde_json::json!({
        "step_index": step.step_index,
        "selected_skill": step.selected_skill,
        "selection_reason": step.selection_reason,
        "candidate_skills": step.candidate_skills,
        "predicted_scores": step.predicted_scores.iter().map(|c| {
            serde_json::json!({"skill_id": c.skill_id, "score": c.score})
        }).collect::<Vec<_>>(),
        "critic_decision": step.critic_decision,
        "progress_delta": step.progress_delta,
        "termination_reason": step.termination_reason.as_ref().map(|t| format!("{:?}", t)),
        "rollback_invoked": step.rollback_invoked,
        "observation_success": step.port_calls.iter().all(|p| p.success),
        "failure_detail": step.failure_detail,
        "timestamp": step.timestamp.to_rfc3339(),
        "port_calls": step.port_calls.iter().map(|p| {
            serde_json::json!({
                "port_id": p.port_id,
                "capability_id": p.capability_id,
                "success": p.success,
                "latency_ms": p.latency_ms
            })
        }).collect::<Vec<_>>(),
    })
}

fn build_notification(method: &str, params: Value) -> String {
    let frame = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    serde_json::to_string(&frame).unwrap_or_default()
}

const NOTIFICATION_METHOD: &str = "notifications/goal/trace_step";

/// Pushes trace-step notifications to one or more transports.
pub trait TraceNotifier: Send + Sync {
    /// A new trace step was recorded for an async goal.
    fn notify_step(&self, goal_id: Uuid, step: &TraceStep, status: &str, terminal: bool);

    /// The goal reached a terminal state. Sent even if no new trace step was
    /// recorded (e.g. cancellation before the first step).
    fn notify_terminal(&self, goal_id: Uuid, status: &str, error: Option<&str>) {
        let params = serde_json::json!({
            "goal_id": goal_id.to_string(),
            "status": status,
            "terminal": true,
            "event": null,
            "error": error,
        });
        self.send_raw(&build_notification(NOTIFICATION_METHOD, params));
    }

    /// Low-level: emit a pre-built notification string. Implementors
    /// override this; the default methods above call it.
    fn send_raw(&self, frame: &str);
}

/// Writes JSON-RPC notifications to stdout. Uses `Stdout::lock()` so
/// concurrent writes from the main loop and background goal threads
/// serialize correctly.
pub struct StdioTraceNotifier;

impl TraceNotifier for StdioTraceNotifier {
    fn notify_step(&self, goal_id: Uuid, step: &TraceStep, status: &str, terminal: bool) {
        let params = serde_json::json!({
            "goal_id": goal_id.to_string(),
            "status": status,
            "terminal": terminal,
            "event": serialize_trace_step(step),
        });
        self.send_raw(&build_notification(NOTIFICATION_METHOD, params));
    }

    fn send_raw(&self, frame: &str) {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        let _ = writeln!(handle, "{}", frame);
        let _ = handle.flush();
    }
}

/// Does nothing. Used in tests and when no transport is listening.
pub struct NoopTraceNotifier;

impl TraceNotifier for NoopTraceNotifier {
    fn notify_step(&self, _: Uuid, _: &TraceStep, _: &str, _: bool) {}
    fn send_raw(&self, _: &str) {}
}

/// Chains two notifiers so both stdio and WebSocket receive events.
pub struct CompositeTraceNotifier {
    pub inner: Vec<Arc<dyn TraceNotifier>>,
}

impl TraceNotifier for CompositeTraceNotifier {
    fn notify_step(&self, goal_id: Uuid, step: &TraceStep, status: &str, terminal: bool) {
        for n in &self.inner {
            n.notify_step(goal_id, step, status, terminal);
        }
    }

    fn notify_terminal(&self, goal_id: Uuid, status: &str, error: Option<&str>) {
        for n in &self.inner {
            n.notify_terminal(goal_id, status, error);
        }
    }

    fn send_raw(&self, frame: &str) {
        for n in &self.inner {
            n.send_raw(frame);
        }
    }
}
