use std::collections::HashMap;
use std::fmt::Write as _;

use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::session::{SessionTrace, TraceStep};

// --- TraceRuntime trait ---

/// The Trace Runtime: full per-session trace storage.
///
/// Responsibilities:
/// - Create and store per-session traces
/// - Record each step with belief, candidates, scores, selection, observation, patch, critic
/// - Render human-readable audit output
/// - Export full trace as JSON for replay and post-mortem analysis
/// - Track denied invocations and partial success via PolicyTraceEntry records on each step
pub trait TraceRuntime: Send + Sync {
    /// Create a new empty trace for a session.
    fn create_trace(&mut self, session_id: Uuid) -> Result<SessionTrace>;

    /// Record a step into the trace for a given session.
    /// The step must contain all required fields: belief summary, retrieved episodes/schemas/routines,
    /// candidate skills, predicted scores, selected skill, port calls, observation id, belief patch,
    /// progress delta, critic decision, policy decisions (including denied invocations), and timestamp.
    fn record_step(&mut self, session_id: &Uuid, step: TraceStep) -> Result<()>;

    /// Retrieve the full trace for a session.
    fn get_trace(&self, session_id: &Uuid) -> Option<&SessionTrace>;

    /// Retrieve a single step from a session trace by index.
    fn get_step(&self, session_id: &Uuid, step_index: u32) -> Option<&TraceStep>;

    /// Render a human-readable audit string for a session trace.
    /// Shows each step: step number, skill selected, observation success/fail,
    /// progress delta, critic decision, and any denied invocations.
    fn render_audit(&self, session_id: &Uuid) -> Result<String>;

    /// Export the full session trace as JSON for replay and post-mortem analysis.
    /// Preserves all data needed to reconstruct the session.
    fn export_trace(&self, session_id: &Uuid) -> Result<serde_json::Value>;

    /// List all session IDs that have traces.
    fn list_sessions(&self) -> Vec<Uuid>;

    /// Delete the trace for a session.
    fn delete_trace(&mut self, session_id: &Uuid) -> Result<()>;
}

// --- DefaultTraceRuntime ---

/// Default in-memory implementation of TraceRuntime.
pub struct DefaultTraceRuntime {
    traces: HashMap<Uuid, SessionTrace>,
}

impl DefaultTraceRuntime {
    pub fn new() -> Self {
        Self {
            traces: HashMap::new(),
        }
    }
}

impl Default for DefaultTraceRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceRuntime for DefaultTraceRuntime {
    fn create_trace(&mut self, session_id: Uuid) -> Result<SessionTrace> {
        if self.traces.contains_key(&session_id) {
            return Err(SomaError::Trace(format!(
                "trace already exists for session {}",
                session_id
            )));
        }

        let trace = SessionTrace { steps: Vec::new() };
        self.traces.insert(session_id, trace.clone());
        Ok(trace)
    }

    fn record_step(&mut self, session_id: &Uuid, step: TraceStep) -> Result<()> {
        let trace = self.traces.get_mut(session_id).ok_or_else(|| {
            SomaError::Trace(format!("no trace for session {}", session_id))
        })?;

        // Validate step_index is sequential.
        let expected_index = trace.steps.len() as u32;
        if step.step_index != expected_index {
            return Err(SomaError::Trace(format!(
                "step_index mismatch: expected {}, got {}",
                expected_index, step.step_index
            )));
        }

        trace.steps.push(step);
        Ok(())
    }

    fn get_trace(&self, session_id: &Uuid) -> Option<&SessionTrace> {
        self.traces.get(session_id)
    }

    fn get_step(&self, session_id: &Uuid, step_index: u32) -> Option<&TraceStep> {
        self.traces
            .get(session_id)
            .and_then(|trace| trace.steps.get(step_index as usize))
    }

    fn render_audit(&self, session_id: &Uuid) -> Result<String> {
        let trace = self.traces.get(session_id).ok_or_else(|| {
            SomaError::Trace(format!("no trace for session {}", session_id))
        })?;

        let mut output = String::new();
        writeln!(output, "=== Session Trace: {} ===", session_id)
            .expect("write to String cannot fail");
        writeln!(output, "Total steps: {}", trace.steps.len())
            .expect("write to String cannot fail");
        writeln!(output).expect("write to String cannot fail");

        for step in &trace.steps {
            writeln!(output, "--- Step {} ---", step.step_index)
                .expect("write to String cannot fail");
            writeln!(output, "  Timestamp:      {}", step.timestamp)
                .expect("write to String cannot fail");
            writeln!(output, "  Selected skill: {}", step.selected_skill)
                .expect("write to String cannot fail");
            writeln!(
                output,
                "  Candidates:     {} considered",
                step.candidate_skills.len()
            )
            .expect("write to String cannot fail");

            // Show top candidate scores.
            if !step.predicted_scores.is_empty() {
                writeln!(output, "  Predicted scores:")
                    .expect("write to String cannot fail");
                for cs in &step.predicted_scores {
                    writeln!(
                        output,
                        "    {}: score={:.3}, success={:.3}, cost={:.3}, latency={}ms, info_gain={:.3}",
                        cs.skill_id, cs.score, cs.predicted_success, cs.predicted_cost,
                        cs.predicted_latency_ms, cs.information_gain
                    )
                    .expect("write to String cannot fail");
                }
            }

            writeln!(output, "  Observation:    {}", step.observation_id)
                .expect("write to String cannot fail");

            // Port calls.
            if !step.port_calls.is_empty() {
                let calls: Vec<String> = step
                    .port_calls
                    .iter()
                    .map(|pc| format!("{}:{}", pc.port_id, pc.capability_id))
                    .collect();
                writeln!(output, "  Port calls:     {}", calls.join(", "))
                    .expect("write to String cannot fail");
            }

            writeln!(output, "  Progress delta: {:+.3}", step.progress_delta)
                .expect("write to String cannot fail");
            writeln!(output, "  Critic:         {}", step.critic_decision)
                .expect("write to String cannot fail");

            // Policy decisions (includes denied invocations and partial success).
            if !step.policy_decisions.is_empty() {
                writeln!(output, "  Policy decisions:")
                    .expect("write to String cannot fail");
                for pd in &step.policy_decisions {
                    writeln!(
                        output,
                        "    [{}] {} - {}",
                        pd.decision, pd.action, pd.reason
                    )
                    .expect("write to String cannot fail");
                }
            }

            // Retrieved memory context.
            if !step.retrieved_episodes.is_empty()
                || !step.retrieved_schemas.is_empty()
                || !step.retrieved_routines.is_empty()
            {
                writeln!(output, "  Retrieved:      {} episodes, {} schemas, {} routines",
                    step.retrieved_episodes.len(),
                    step.retrieved_schemas.len(),
                    step.retrieved_routines.len(),
                )
                .expect("write to String cannot fail");
            }

            writeln!(output).expect("write to String cannot fail");
        }

        Ok(output)
    }

    fn export_trace(&self, session_id: &Uuid) -> Result<serde_json::Value> {
        let trace = self.traces.get(session_id).ok_or_else(|| {
            SomaError::Trace(format!("no trace for session {}", session_id))
        })?;

        let export = serde_json::json!({
            "session_id": session_id,
            "step_count": trace.steps.len(),
            "trace": serde_json::to_value(trace)?,
        });

        Ok(export)
    }

    fn list_sessions(&self) -> Vec<Uuid> {
        self.traces.keys().copied().collect()
    }

    fn delete_trace(&mut self, session_id: &Uuid) -> Result<()> {
        self.traces.remove(session_id).ok_or_else(|| {
            SomaError::Trace(format!("no trace for session {}", session_id))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::types::observation::PortCallRecord;
    use crate::types::session::{CandidateScore, PolicyTraceEntry};

    fn stub_port_call(port_id: &str, capability_id: &str) -> PortCallRecord {
        PortCallRecord {
            observation_id: Uuid::new_v4(),
            port_id: port_id.to_string(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: true,
            failure_class: None,
            raw_result: serde_json::Value::Null,
            structured_result: serde_json::Value::Null,
            effect_patch: None,
            side_effect_summary: None,
            latency_ms: 1,
            resource_cost: 0.0,
            confidence: 1.0,
            timestamp: Utc::now(),
            retry_safe: true,
            input_hash: None,
            session_id: None,
            goal_id: None,
            caller_identity: None,
            auth_result: None,
            policy_result: None,
            sandbox_result: None,
        }
    }

    fn make_runtime() -> DefaultTraceRuntime {
        DefaultTraceRuntime::new()
    }

    fn make_step(index: u32) -> TraceStep {
        TraceStep {
            step_index: index,
            belief_summary_before: serde_json::json!({"resources": 3, "facts": 5}),
            retrieved_episodes: vec!["ep-001".to_string()],
            retrieved_schemas: vec!["schema-fs".to_string()],
            retrieved_routines: vec!["routine-list-dir".to_string()],
            candidate_skills: vec!["fs.list".to_string(), "fs.stat".to_string()],
            predicted_scores: vec![
                CandidateScore {
                    skill_id: "fs.list".to_string(),
                    score: 0.92,
                    predicted_success: 0.95,
                    predicted_cost: 0.1,
                    predicted_latency_ms: 50,
                    information_gain: 0.8,
                },
                CandidateScore {
                    skill_id: "fs.stat".to_string(),
                    score: 0.65,
                    predicted_success: 0.80,
                    predicted_cost: 0.05,
                    predicted_latency_ms: 20,
                    information_gain: 0.3,
                },
            ],
            selected_skill: "fs.list".to_string(),
            port_calls: vec![stub_port_call("posix.readdir", "read")],
            observation_id: Uuid::new_v4(),
            belief_patch: serde_json::json!({"added_resources": [{"type": "directory_listing"}]}),
            progress_delta: 0.4,
            critic_decision: "continue".to_string(),
            policy_decisions: vec![],
            bound_inputs: vec![],
            precondition_results: vec![],
            termination_reason: None,
            rollback_invoked: false,
            selection_reason: crate::types::session::SelectionReason::HighestScore,
            failure_detail: None,
            timestamp: Utc::now(),
        }
    }

    fn make_step_with_denial(index: u32) -> TraceStep {
        let mut step = make_step(index);
        step.policy_decisions = vec![
            PolicyTraceEntry {
                action: "port.invoke:posix.rm".to_string(),
                decision: "denied".to_string(),
                reason: "destructive operation not permitted by session policy".to_string(),
            },
        ];
        step.selected_skill = "fs.list".to_string();
        step.critic_decision = "continue_with_fallback".to_string();
        step
    }

    fn make_partial_success_step(index: u32) -> TraceStep {
        let mut step = make_step(index);
        step.port_calls = vec![];
        step.policy_decisions = vec![
            PolicyTraceEntry {
                action: "port.invoke:posix.readdir".to_string(),
                decision: "allowed".to_string(),
                reason: "within permissions scope".to_string(),
            },
            PolicyTraceEntry {
                action: "port.invoke:posix.stat".to_string(),
                decision: "partial".to_string(),
                reason: "permission denied on 2 of 5 entries".to_string(),
            },
        ];
        step.progress_delta = 0.25;
        step.critic_decision = "continue".to_string();
        step
    }

    #[test]
    fn create_trace_returns_empty() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        let trace = rt.create_trace(sid).unwrap();
        assert!(trace.steps.is_empty());
    }

    #[test]
    fn create_trace_duplicate_fails() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        let result = rt.create_trace(sid);
        assert!(result.is_err());
    }

    #[test]
    fn record_and_retrieve_step() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();

        let step = make_step(0);
        let obs_id = step.observation_id;
        rt.record_step(&sid, step).unwrap();

        let trace = rt.get_trace(&sid).unwrap();
        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.steps[0].selected_skill, "fs.list");
        assert_eq!(trace.steps[0].observation_id, obs_id);
    }

    #[test]
    fn record_step_no_trace_fails() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        let step = make_step(0);
        let result = rt.record_step(&sid, step);
        assert!(result.is_err());
    }

    #[test]
    fn record_step_index_mismatch_fails() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();

        // Try to record step 1 without step 0.
        let step = make_step(1);
        let result = rt.record_step(&sid, step);
        assert!(result.is_err());
    }

    #[test]
    fn get_step_by_index() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();

        rt.record_step(&sid, make_step(0)).unwrap();
        let mut step1 = make_step(1);
        step1.selected_skill = "fs.stat".to_string();
        rt.record_step(&sid, step1).unwrap();

        let s0 = rt.get_step(&sid, 0).unwrap();
        assert_eq!(s0.selected_skill, "fs.list");

        let s1 = rt.get_step(&sid, 1).unwrap();
        assert_eq!(s1.selected_skill, "fs.stat");

        assert!(rt.get_step(&sid, 2).is_none());
    }

    #[test]
    fn get_step_no_trace_returns_none() {
        let rt = make_runtime();
        let sid = Uuid::new_v4();
        assert!(rt.get_step(&sid, 0).is_none());
    }

    #[test]
    fn render_audit_basic() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_step(0)).unwrap();

        let audit = rt.render_audit(&sid).unwrap();

        assert!(audit.contains(&format!("Session Trace: {}", sid)));
        assert!(audit.contains("Total steps: 1"));
        assert!(audit.contains("Step 0"));
        assert!(audit.contains("fs.list"));
        assert!(audit.contains("2 considered"));
        assert!(audit.contains("+0.400"));
        assert!(audit.contains("continue"));
        assert!(audit.contains("posix.readdir:read"));
        assert!(audit.contains("score=0.920"));
    }

    #[test]
    fn render_audit_no_trace_fails() {
        let rt = make_runtime();
        let sid = Uuid::new_v4();
        let result = rt.render_audit(&sid);
        assert!(result.is_err());
    }

    #[test]
    fn render_audit_shows_denied_invocations() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_step_with_denial(0)).unwrap();

        let audit = rt.render_audit(&sid).unwrap();

        assert!(audit.contains("[denied]"));
        assert!(audit.contains("posix.rm"));
        assert!(audit.contains("destructive operation"));
    }

    #[test]
    fn render_audit_shows_partial_success() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_partial_success_step(0)).unwrap();

        let audit = rt.render_audit(&sid).unwrap();

        assert!(audit.contains("[partial]"));
        assert!(audit.contains("permission denied on 2 of 5"));
    }

    #[test]
    fn render_audit_multi_step() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();

        rt.record_step(&sid, make_step(0)).unwrap();
        let mut step1 = make_step(1);
        step1.selected_skill = "http.get".to_string();
        step1.progress_delta = 0.6;
        step1.critic_decision = "complete".to_string();
        rt.record_step(&sid, step1).unwrap();

        let audit = rt.render_audit(&sid).unwrap();

        assert!(audit.contains("Total steps: 2"));
        assert!(audit.contains("Step 0"));
        assert!(audit.contains("Step 1"));
        assert!(audit.contains("http.get"));
        assert!(audit.contains("complete"));
    }

    #[test]
    fn export_trace_contains_all_data() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_step(0)).unwrap();

        let export = rt.export_trace(&sid).unwrap();

        assert_eq!(export["session_id"], sid.to_string());
        assert_eq!(export["step_count"], 1);

        let trace = &export["trace"];
        let steps = trace["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1);

        let step = &steps[0];
        assert_eq!(step["step_index"], 0);
        assert_eq!(step["selected_skill"], "fs.list");
        assert_eq!(step["progress_delta"], 0.4);
        assert_eq!(step["critic_decision"], "continue");

        // Verify all fields needed for replay are present.
        assert!(step.get("belief_summary_before").is_some());
        assert!(step.get("retrieved_episodes").is_some());
        assert!(step.get("retrieved_schemas").is_some());
        assert!(step.get("retrieved_routines").is_some());
        assert!(step.get("candidate_skills").is_some());
        assert!(step.get("predicted_scores").is_some());
        assert!(step.get("port_calls").is_some());
        assert!(step.get("observation_id").is_some());
        assert!(step.get("belief_patch").is_some());
        assert!(step.get("policy_decisions").is_some());
        assert!(step.get("timestamp").is_some());
    }

    #[test]
    fn export_trace_no_trace_fails() {
        let rt = make_runtime();
        let sid = Uuid::new_v4();
        let result = rt.export_trace(&sid);
        assert!(result.is_err());
    }

    #[test]
    fn export_trace_preserves_denied_invocations() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_step_with_denial(0)).unwrap();

        let export = rt.export_trace(&sid).unwrap();
        let steps = export["trace"]["steps"].as_array().unwrap();
        let policies = steps[0]["policy_decisions"].as_array().unwrap();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0]["decision"], "denied");
        assert_eq!(policies[0]["action"], "port.invoke:posix.rm");
    }

    #[test]
    fn export_trace_preserves_partial_success() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_partial_success_step(0)).unwrap();

        let export = rt.export_trace(&sid).unwrap();
        let steps = export["trace"]["steps"].as_array().unwrap();
        let policies = steps[0]["policy_decisions"].as_array().unwrap();
        assert_eq!(policies.len(), 2);

        let partial = policies.iter().find(|p| p["decision"] == "partial").unwrap();
        assert!(partial["reason"].as_str().unwrap().contains("2 of 5"));
    }

    #[test]
    fn list_sessions_empty() {
        let rt = make_runtime();
        assert!(rt.list_sessions().is_empty());
    }

    #[test]
    fn list_sessions_returns_all() {
        let mut rt = make_runtime();
        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();
        let s3 = Uuid::new_v4();
        rt.create_trace(s1).unwrap();
        rt.create_trace(s2).unwrap();
        rt.create_trace(s3).unwrap();

        let sessions = rt.list_sessions();
        assert_eq!(sessions.len(), 3);
        assert!(sessions.contains(&s1));
        assert!(sessions.contains(&s2));
        assert!(sessions.contains(&s3));
    }

    #[test]
    fn delete_trace_removes_session() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_step(0)).unwrap();

        rt.delete_trace(&sid).unwrap();

        assert!(rt.get_trace(&sid).is_none());
        assert!(!rt.list_sessions().contains(&sid));
    }

    #[test]
    fn delete_trace_not_found_fails() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        let result = rt.delete_trace(&sid);
        assert!(result.is_err());
    }

    #[test]
    fn full_lifecycle() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();

        // Create.
        rt.create_trace(sid).unwrap();

        // Record multiple steps.
        rt.record_step(&sid, make_step(0)).unwrap();

        let mut step1 = make_step(1);
        step1.selected_skill = "http.get".to_string();
        step1.progress_delta = 0.3;
        step1.critic_decision = "continue".to_string();
        rt.record_step(&sid, step1).unwrap();

        let mut step2 = make_step(2);
        step2.selected_skill = "db.query".to_string();
        step2.progress_delta = 0.3;
        step2.critic_decision = "complete".to_string();
        rt.record_step(&sid, step2).unwrap();

        // Verify trace.
        let trace = rt.get_trace(&sid).unwrap();
        assert_eq!(trace.steps.len(), 3);

        // Audit.
        let audit = rt.render_audit(&sid).unwrap();
        assert!(audit.contains("Total steps: 3"));
        assert!(audit.contains("fs.list"));
        assert!(audit.contains("http.get"));
        assert!(audit.contains("db.query"));
        assert!(audit.contains("complete"));

        // Export.
        let export = rt.export_trace(&sid).unwrap();
        assert_eq!(export["step_count"], 3);

        // Export is valid JSON that can be re-parsed (replay support).
        let trace_json = &export["trace"];
        let restored: SessionTrace = serde_json::from_value(trace_json.clone()).unwrap();
        assert_eq!(restored.steps.len(), 3);
        assert_eq!(restored.steps[0].selected_skill, "fs.list");
        assert_eq!(restored.steps[1].selected_skill, "http.get");
        assert_eq!(restored.steps[2].selected_skill, "db.query");

        // Delete.
        rt.delete_trace(&sid).unwrap();
        assert!(rt.get_trace(&sid).is_none());
    }

    #[test]
    fn export_roundtrip_preserves_all_fields() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();

        let step = make_step(0);
        let original_belief = step.belief_summary_before.clone();
        let original_patch = step.belief_patch.clone();
        let original_obs = step.observation_id;
        rt.record_step(&sid, step).unwrap();

        let export = rt.export_trace(&sid).unwrap();
        let restored: SessionTrace =
            serde_json::from_value(export["trace"].clone()).unwrap();

        assert_eq!(restored.steps[0].belief_summary_before, original_belief);
        assert_eq!(restored.steps[0].belief_patch, original_patch);
        assert_eq!(restored.steps[0].observation_id, original_obs);
        assert_eq!(restored.steps[0].retrieved_episodes, vec!["ep-001"]);
        assert_eq!(restored.steps[0].retrieved_schemas, vec!["schema-fs"]);
        assert_eq!(restored.steps[0].retrieved_routines, vec!["routine-list-dir"]);
        assert_eq!(restored.steps[0].candidate_skills.len(), 2);
        assert_eq!(restored.steps[0].predicted_scores.len(), 2);
        assert_eq!(restored.steps[0].port_calls.len(), 1);
    }

    #[test]
    fn render_audit_empty_trace() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();

        let audit = rt.render_audit(&sid).unwrap();
        assert!(audit.contains("Total steps: 0"));
        // Should not contain any step headers.
        assert!(!audit.contains("Step 0"));
    }

    #[test]
    fn render_audit_retrieved_memory_context() {
        let mut rt = make_runtime();
        let sid = Uuid::new_v4();
        rt.create_trace(sid).unwrap();
        rt.record_step(&sid, make_step(0)).unwrap();

        let audit = rt.render_audit(&sid).unwrap();
        assert!(audit.contains("1 episodes"));
        assert!(audit.contains("1 schemas"));
        assert!(audit.contains("1 routines"));
    }
}
