use std::sync::{Arc, Mutex};

use tracing::debug;

use crate::runtime::port::{DefaultPortRuntime, PortRuntime};
use crate::runtime::session::BrainFallback;
use crate::types::port::InvocationContext;

/// Implements `BrainFallback` by invoking the `brain` port's `reason` capability.
///
/// When the predictor's top score falls below the confidence threshold,
/// the session controller calls `select_skill` here. We forward the query
/// to the brain port, which returns skill recommendations ranked by the
/// neural reasoning core (LTC + SDM retrieval).
pub struct PortBrainFallback {
    port_runtime: Arc<Mutex<DefaultPortRuntime>>,
}

impl PortBrainFallback {
    pub fn new(port_runtime: Arc<Mutex<DefaultPortRuntime>>) -> Self {
        Self { port_runtime }
    }
}

impl BrainFallback for PortBrainFallback {
    fn select_skill(
        &self,
        goal: &str,
        candidates: &[String],
        belief_summary: &str,
    ) -> crate::errors::Result<String> {
        let query = format!(
            "Goal: {goal}\nBelief: {belief_summary}\nCandidates: {}",
            candidates.join(", ")
        );

        let input = serde_json::json!({
            "query": query,
            "top_k_sources": 5,
        });

        let ctx = InvocationContext {
            session_id: None,
            goal_id: Some(goal.to_string()),
            caller_identity: Some("brain_fallback".to_string()),
            remote_caller: false,
            pack_id: None,
            calling_pack_id: None,
            deadline_ms: Some(10_000),
        };

        let record = self
            .port_runtime
            .lock()
            .map_err(|_| crate::errors::SomaError::Skill("port_runtime lock poisoned".into()))?
            .invoke("brain", "reason", input, &ctx)?;

        let result = &record.structured_result;

        if let Some(recommendations) = result.get("skill_recommendations").and_then(|v| v.as_array())
        {
            for rec in recommendations {
                if let Some(skill_id) = rec.get("skill_id").and_then(|v| v.as_str())
                    && candidates.contains(&skill_id.to_string())
                {
                    let score: f64 = rec
                        .get("score")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    let confidence: f64 = result
                        .get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    debug!(
                        skill_id,
                        score,
                        confidence,
                        "brain selected skill from candidates"
                    );
                    return Ok(skill_id.to_string());
                }
            }
        }

        Err(crate::errors::SomaError::Skill(
            "brain returned no valid candidate".into(),
        ))
    }
}

/// Sends a completed episode to the brain port for consolidation into SDM.
/// Called after `finalize_episode` stores the episode in the body's memory.
pub fn consolidate_episode_to_brain(
    port_runtime: &Arc<Mutex<DefaultPortRuntime>>,
    episode: &serde_json::Value,
) {
    let input = serde_json::json!({ "episode": episode });
    let ctx = InvocationContext {
        session_id: None,
        goal_id: None,
        caller_identity: Some("consolidation".to_string()),
        remote_caller: false,
        pack_id: None,
        calling_pack_id: None,
        deadline_ms: Some(30_000),
    };

    match port_runtime.lock() {
        Ok(rt) => match rt.invoke("brain", "consolidate_episode", input, &ctx) {
            Ok(record) => {
                debug!(
                    result = %record.structured_result,
                    "episode consolidated to brain"
                );
            }
            Err(e) => {
                debug!(error = %e, "brain episode consolidation failed (non-fatal)");
            }
        },
        Err(_) => {
            debug!("port_runtime lock poisoned during brain consolidation");
        }
    }
}

/// Sends a compiled routine to the brain port for SDM storage.
pub fn consolidate_routine_to_brain(
    port_runtime: &Arc<Mutex<DefaultPortRuntime>>,
    routine: &serde_json::Value,
) {
    let input = serde_json::json!({ "routine": routine });
    let ctx = InvocationContext {
        session_id: None,
        goal_id: None,
        caller_identity: Some("consolidation".to_string()),
        remote_caller: false,
        pack_id: None,
        calling_pack_id: None,
        deadline_ms: Some(30_000),
    };

    match port_runtime.lock() {
        Ok(rt) => match rt.invoke("brain", "consolidate_routine", input, &ctx) {
            Ok(record) => {
                debug!(
                    result = %record.structured_result,
                    "routine consolidated to brain"
                );
            }
            Err(e) => {
                debug!(error = %e, "brain routine consolidation failed (non-fatal)");
            }
        },
        Err(_) => {
            debug!("port_runtime lock poisoned during brain consolidation");
        }
    }
}
