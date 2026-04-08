use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::Result;
use crate::memory::episodes::EpisodeStore;
use crate::memory::routines::RoutineStore;
use crate::memory::schemas::SchemaStore;
use crate::runtime::goal::{DefaultGoalRuntime, GoalInput, GoalRuntime};
use crate::runtime::metrics::RuntimeMetrics;
use crate::runtime::port::{DefaultPortRuntime, PortRuntime};
use crate::runtime::session::{SessionController, SessionRuntime, StepResult};
use crate::runtime::skill::{DefaultSkillRuntime, SkillRuntime};
use crate::types::goal::GoalSource;
use crate::types::pack::PackSpec;

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 types
// ---------------------------------------------------------------------------

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    pub id: Value,
}

/// JSON-RPC 2.0 success/error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
    pub id: Value,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// Standard JSON-RPC 2.0 error codes.
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

// ---------------------------------------------------------------------------
// MCP tool metadata
// ---------------------------------------------------------------------------

/// Describes a single MCP tool exposed by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// ---------------------------------------------------------------------------
// McpServer
// ---------------------------------------------------------------------------

/// Bundles the runtime subsystems that the MCP server dispatches to.
///
/// Each field is an `Arc<Mutex<...>>` so the MCP server can be shared across
/// threads while still mutating session state (creating sessions, stepping,
/// pausing, etc.).
pub struct RuntimeHandle {
    pub session_controller: Arc<Mutex<SessionController>>,
    pub goal_runtime: Arc<Mutex<DefaultGoalRuntime>>,
    pub skill_runtime: Arc<Mutex<DefaultSkillRuntime>>,
    pub port_runtime: Arc<Mutex<DefaultPortRuntime>>,
    pub pack_specs: Arc<Mutex<Vec<PackSpec>>>,
    pub metrics: Arc<RuntimeMetrics>,
    pub episode_store: Arc<Mutex<dyn EpisodeStore + Send>>,
    pub schema_store: Arc<Mutex<dyn SchemaStore + Send>>,
    pub routine_store: Arc<Mutex<dyn RoutineStore + Send>>,
    pub start_time: Instant,
}

impl RuntimeHandle {
    /// Build a RuntimeHandle from a bootstrapped Runtime by wrapping each
    /// subsystem in Arc<Mutex<>> for shared ownership.
    pub fn from_runtime(runtime: crate::bootstrap::Runtime) -> Self {
        Self {
            session_controller: Arc::new(Mutex::new(runtime.session_controller)),
            goal_runtime: Arc::new(Mutex::new(runtime.goal_runtime)),
            skill_runtime: Arc::new(Mutex::new(runtime.skill_runtime)),
            port_runtime: runtime.port_runtime,
            pack_specs: Arc::new(Mutex::new(runtime.pack_specs)),
            metrics: runtime.metrics,
            episode_store: runtime.episode_store,
            schema_store: runtime.schema_store,
            routine_store: runtime.routine_store,
            start_time: runtime.start_time,
        }
    }
}

/// MCP (Model Context Protocol) server.
///
/// Exposes the full SOMA interface as JSON-RPC 2.0 tools so that LLMs and
/// other MCP-aware clients can submit goals, inspect sessions, control
/// execution, and query metrics.
///
/// When constructed with a `RuntimeHandle`, tool handlers dispatch to real
/// runtime operations. When constructed without one (`new_stub()`), handlers
/// return static placeholder data — useful for protocol-level testing.
pub struct McpServer {
    tools: Vec<McpTool>,
    runtime: Option<RuntimeHandle>,
}

impl McpServer {
    /// Create an MCP server wired to real runtime subsystems.
    pub fn new(runtime: RuntimeHandle) -> Self {
        Self {
            tools: Self::build_tools(),
            runtime: Some(runtime),
        }
    }

    /// Create a stub MCP server without any runtime backing.
    /// Handlers return placeholder data. Useful for protocol-level tests.
    pub fn new_stub() -> Self {
        Self {
            tools: Self::build_tools(),
            runtime: None,
        }
    }

    /// Return the list of all tools the server exposes.
    pub fn list_tools(&self) -> Vec<McpTool> {
        self.tools.clone()
    }

    /// Dispatch a JSON-RPC 2.0 request to the appropriate tool handler.
    pub fn handle_request(&self, request: McpRequest) -> Result<McpResponse> {
        // Validate JSON-RPC version.
        if request.jsonrpc != "2.0" {
            return Ok(Self::error_response(
                request.id,
                INVALID_REQUEST,
                "jsonrpc must be \"2.0\"".to_string(),
                None,
            ));
        }

        match request.method.as_str() {
            // MCP protocol methods
            "initialize" => self.handle_initialize(request.id, request.params),
            "tools/list" => self.handle_tools_list(request.id),
            "tools/call" => self.handle_tools_call(request.id, request.params),

            // Direct tool methods (for clients that invoke tools as methods)
            "create_goal" => self.handle_create_goal(request.id, request.params),
            "inspect_session" => self.handle_inspect_session(request.id, request.params),
            "inspect_belief" => self.handle_inspect_belief(request.id, request.params),
            "inspect_resources" => self.handle_inspect_resources(request.id, request.params),
            "inspect_packs" => self.handle_inspect_packs(request.id, request.params),
            "inspect_skills" => self.handle_inspect_skills(request.id, request.params),
            "inspect_trace" => self.handle_inspect_trace(request.id, request.params),
            "pause_session" => self.handle_pause_session(request.id, request.params),
            "resume_session" => self.handle_resume_session(request.id, request.params),
            "abort_session" => self.handle_abort_session(request.id, request.params),
            "list_sessions" => self.handle_list_sessions(request.id, request.params),
            "query_metrics" => self.handle_query_metrics(request.id, request.params),
            "query_policy" => self.handle_query_policy(request.id, request.params),
            "dump_state" => self.handle_dump_state(request.id, request.params),

            _ => Ok(Self::error_response(
                request.id,
                METHOD_NOT_FOUND,
                format!("method not found: {}", request.method),
                None,
            )),
        }
    }

    // -----------------------------------------------------------------------
    // MCP protocol handlers
    // -----------------------------------------------------------------------

    fn handle_initialize(&self, id: Value, _params: Option<Value>) -> Result<McpResponse> {
        Ok(Self::success_response(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "soma-next",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ))
    }

    fn handle_tools_list(&self, id: Value) -> Result<McpResponse> {
        let tools_json = serde_json::to_value(&self.tools)?;
        Ok(Self::success_response(
            id,
            serde_json::json!({ "tools": tools_json }),
        ))
    }

    fn handle_tools_call(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required for tools/call".to_string(),
                    None,
                ));
            }
        };

        let tool_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let arguments = params.get("arguments").cloned();

        match tool_name {
            "create_goal" => self.handle_create_goal(id, arguments),
            "inspect_session" => self.handle_inspect_session(id, arguments),
            "inspect_belief" => self.handle_inspect_belief(id, arguments),
            "inspect_resources" => self.handle_inspect_resources(id, arguments),
            "inspect_packs" => self.handle_inspect_packs(id, arguments),
            "inspect_skills" => self.handle_inspect_skills(id, arguments),
            "inspect_trace" => self.handle_inspect_trace(id, arguments),
            "pause_session" => self.handle_pause_session(id, arguments),
            "resume_session" => self.handle_resume_session(id, arguments),
            "abort_session" => self.handle_abort_session(id, arguments),
            "list_sessions" => self.handle_list_sessions(id, arguments),
            "query_metrics" => self.handle_query_metrics(id, arguments),
            "query_policy" => self.handle_query_policy(id, arguments),
            "dump_state" => self.handle_dump_state(id, arguments),
            _ => Ok(Self::error_response(
                id,
                METHOD_NOT_FOUND,
                format!("unknown tool: {}", tool_name),
                None,
            )),
        }
    }

    // -----------------------------------------------------------------------
    // Tool handlers
    // -----------------------------------------------------------------------

    fn handle_create_goal(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: objective (string)".to_string(),
                    None,
                ));
            }
        };

        let objective = params
            .get("objective")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if objective.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "objective must be a non-empty string".to_string(),
                None,
            ));
        }

        let rt = match &self.runtime {
            Some(rt) => rt,
            None => {
                // Stub mode: return placeholder response.
                let session_id = Uuid::new_v4();
                let goal_id = Uuid::new_v4();
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "session_id": session_id.to_string(),
                        "goal_id": goal_id.to_string(),
                        "status": "created",
                        "objective": objective
                    }),
                ));
            }
        };

        // Parse the goal through the GoalRuntime.
        let source = GoalSource {
            source_type: crate::types::goal::GoalSourceType::Mcp,
            identity: None,
            session_id: None,
            peer_id: None,
        };
        let input = GoalInput::NaturalLanguage {
            text: objective.clone(),
            source,
        };

        let mut goal = {
            let goal_rt = rt.goal_runtime.lock().unwrap();
            match goal_rt.parse_goal(input) {
                Ok(g) => g,
                Err(e) => {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("goal parse failed: {e}"),
                        None,
                    ));
                }
            }
        };

        // Normalize and validate.
        {
            let goal_rt = rt.goal_runtime.lock().unwrap();
            goal_rt.normalize_goal(&mut goal);
            if let Err(e) = goal_rt.validate_goal(&goal) {
                return Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("goal validation failed: {e}"),
                    None,
                ));
            }
        }

        let goal_id = goal.goal_id;

        // Create a session and run to completion (or first non-continue state).
        let mut ctrl = rt.session_controller.lock().unwrap();
        let mut session = match ctrl.create_session(goal) {
            Ok(s) => s,
            Err(e) => {
                return Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("session creation failed: {e}"),
                    None,
                ));
            }
        };

        let session_id = session.session_id;

        // Extract filesystem paths from goal text and inject them as
        // working memory bindings so the skill executor finds them
        // during bind_inputs.
        super::goal_utils::inject_path_bindings(&mut session, &objective);

        // Run the control loop until it reaches a non-Continue state.
        let final_status;
        let mut result_data = serde_json::Value::Null;
        loop {
            match ctrl.run_step(&mut session) {
                Ok(StepResult::Continue) => continue,
                Ok(StepResult::Completed) => {
                    final_status = "completed".to_string();
                    // Extract the last observation's structured result if available.
                    if let Some(last_step) = session.trace.steps.last() {
                        result_data = serde_json::json!({
                            "steps": session.trace.steps.len(),
                            "last_skill": last_step.selected_skill,
                        });
                    }
                    break;
                }
                Ok(StepResult::Failed(reason)) => {
                    final_status = "failed".to_string();
                    result_data = serde_json::json!({ "reason": reason });
                    break;
                }
                Ok(StepResult::Aborted) => {
                    final_status = "aborted".to_string();
                    break;
                }
                Ok(StepResult::WaitingForInput(msg)) => {
                    final_status = "waiting_for_input".to_string();
                    result_data = serde_json::json!({ "waiting_for": msg });
                    break;
                }
                Ok(StepResult::WaitingForRemote(msg)) => {
                    final_status = "waiting_for_remote".to_string();
                    result_data = serde_json::json!({ "waiting_for": msg });
                    break;
                }
                Err(e) => {
                    final_status = "error".to_string();
                    result_data = serde_json::json!({ "error": e.to_string() });
                    break;
                }
            }
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "session_id": session_id.to_string(),
                "goal_id": goal_id.to_string(),
                "status": final_status,
                "objective": objective,
                "result": result_data
            }),
        ))
    }

    fn handle_inspect_session(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let session_id_str = match Self::extract_session_id(&params) {
            Some(sid) => sid,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "session_id required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = &self.runtime {
            let uuid = match Uuid::parse_str(&session_id_str) {
                Ok(u) => u,
                Err(_) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        "session_id must be a valid UUID".to_string(),
                        None,
                    ));
                }
            };

            let ctrl = rt.session_controller.lock().unwrap();
            match ctrl.get_session(&uuid) {
                Some(session) => Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "session_id": session.session_id.to_string(),
                        "status": format!("{:?}", session.status),
                        "objective": session.goal.objective.description,
                        "working_memory": {
                            "active_bindings": session.working_memory.active_bindings.len(),
                            "unresolved_slots": &session.working_memory.unresolved_slots,
                            "current_subgoal": &session.working_memory.current_subgoal,
                            "candidate_shortlist": &session.working_memory.candidate_shortlist,
                        },
                        "budget_remaining": {
                            "risk_remaining": session.budget_remaining.risk_remaining,
                            "latency_remaining_ms": session.budget_remaining.latency_remaining_ms,
                            "resource_remaining": session.budget_remaining.resource_remaining,
                            "steps_remaining": session.budget_remaining.steps_remaining
                        },
                        "step_count": session.trace.steps.len(),
                        "created_at": session.created_at.to_rfc3339(),
                        "updated_at": session.updated_at.to_rfc3339()
                    }),
                )),
                None => Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("session not found: {session_id_str}"),
                    None,
                )),
            }
        } else {
            // Stub mode.
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "status": "created",
                    "working_memory": {},
                    "budget_remaining": {
                        "risk_remaining": 0.5,
                        "latency_remaining_ms": 30000,
                        "resource_remaining": 100.0,
                        "steps_remaining": 100
                    }
                }),
            ))
        }
    }

    fn handle_inspect_belief(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let session_id_str = match Self::extract_session_id(&params) {
            Some(sid) => sid,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "session_id required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = &self.runtime {
            let uuid = match Uuid::parse_str(&session_id_str) {
                Ok(u) => u,
                Err(_) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        "session_id must be a valid UUID".to_string(),
                        None,
                    ));
                }
            };

            let ctrl = rt.session_controller.lock().unwrap();
            match ctrl.get_session(&uuid) {
                Some(session) => {
                    let belief = &session.belief;
                    Ok(Self::success_response(
                        id,
                        serde_json::json!({
                            "session_id": session.session_id.to_string(),
                            "belief": {
                                "belief_id": belief.belief_id.to_string(),
                                "resources": belief.resources.len(),
                                "facts": belief.facts.iter().map(|f| {
                                    serde_json::json!({
                                        "fact_id": f.fact_id,
                                        "subject": f.subject,
                                        "predicate": f.predicate,
                                        "confidence": f.confidence,
                                    })
                                }).collect::<Vec<_>>(),
                                "uncertainties": &belief.uncertainties,
                                "active_bindings": belief.active_bindings.len(),
                                "world_hash": &belief.world_hash
                            }
                        }),
                    ))
                }
                None => Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("session not found: {session_id_str}"),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "belief": {
                        "resources": [],
                        "facts": [],
                        "uncertainties": [],
                        "active_bindings": [],
                        "world_hash": ""
                    }
                }),
            ))
        }
    }

    fn handle_inspect_resources(&self, id: Value, _params: Option<Value>) -> Result<McpResponse> {
        // Resources are tracked inside belief state per-session. The global
        // resource listing comes from registered port specs which declare what
        // external resources are available to the runtime.
        if let Some(rt) = &self.runtime {
            let port_rt = rt.port_runtime.lock().unwrap();
            let ports = port_rt.list_ports(None);
            let resources: Vec<Value> = ports
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "port_id": p.port_id,
                        "name": p.name,
                        "kind": format!("{:?}", p.kind),
                        "capabilities": p.capabilities.len(),
                    })
                })
                .collect();
            Ok(Self::success_response(
                id,
                serde_json::json!({ "resources": resources }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({ "resources": [] }),
            ))
        }
    }

    fn handle_inspect_packs(&self, id: Value, _params: Option<Value>) -> Result<McpResponse> {
        if let Some(rt) = &self.runtime {
            let specs = rt.pack_specs.lock().unwrap();
            let packs: Vec<Value> = specs
                .iter()
                .map(|spec| {
                    serde_json::json!({
                        "pack_id": spec.id,
                        "namespace": spec.namespace,
                        "version": spec.version.to_string(),
                        "skills": spec.skills.len(),
                        "ports": spec.ports.len(),
                    })
                })
                .collect();
            Ok(Self::success_response(
                id,
                serde_json::json!({ "packs": packs }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({ "packs": [] }),
            ))
        }
    }

    fn handle_inspect_skills(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        if let Some(rt) = &self.runtime {
            let pack_filter = params
                .as_ref()
                .and_then(|p| p.get("pack"))
                .and_then(|v| v.as_str());

            let skill_rt = rt.skill_runtime.lock().unwrap();
            let all_skills = skill_rt.list_skills(pack_filter);
            let skills: Vec<Value> = all_skills
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "skill_id": s.skill_id,
                        "name": s.name,
                        "namespace": s.namespace,
                        "pack": s.pack,
                        "kind": format!("{:?}", s.kind),
                        "description": s.description,
                        "risk_class": format!("{:?}", s.risk_class),
                        "determinism": format!("{:?}", s.determinism),
                        "inputs": s.inputs.schema,
                        "outputs": s.outputs.schema,
                    })
                })
                .collect();
            Ok(Self::success_response(
                id,
                serde_json::json!({ "skills": skills }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({ "skills": [] }),
            ))
        }
    }

    fn handle_inspect_trace(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let session_id_str = match Self::extract_session_id(&params) {
            Some(sid) => sid,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "session_id required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = &self.runtime {
            let uuid = match Uuid::parse_str(&session_id_str) {
                Ok(u) => u,
                Err(_) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        "session_id must be a valid UUID".to_string(),
                        None,
                    ));
                }
            };

            let from_step = params
                .as_ref()
                .and_then(|p| p.get("from_step"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let limit = params
                .as_ref()
                .and_then(|p| p.get("limit"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let ctrl = rt.session_controller.lock().unwrap();
            match ctrl.get_session(&uuid) {
                Some(session) => {
                    let steps = &session.trace.steps;
                    let end = match limit {
                        Some(l) => (from_step + l).min(steps.len()),
                        None => steps.len(),
                    };
                    let start = from_step.min(steps.len());
                    let trace_steps: Vec<Value> = steps[start..end]
                        .iter()
                        .map(|step| {
                            serde_json::json!({
                                "step_index": step.step_index,
                                "selected_skill": step.selected_skill,
                                "observation_id": step.observation_id.to_string(),
                                "candidate_skills": step.candidate_skills,
                                "predicted_scores": step.predicted_scores.iter().map(|s| {
                                    serde_json::json!({
                                        "skill_id": s.skill_id,
                                        "score": s.score,
                                    })
                                }).collect::<Vec<_>>(),
                                "critic_decision": step.critic_decision,
                                "progress_delta": step.progress_delta,
                                "belief_patch": step.belief_patch,
                                "policy_decisions": step.policy_decisions.iter().map(|p| {
                                    serde_json::json!({
                                        "action": p.action,
                                        "decision": p.decision,
                                        "reason": p.reason,
                                    })
                                }).collect::<Vec<_>>(),
                                "termination_reason": step.termination_reason.as_ref().map(|t| format!("{:?}", t)),
                                "rollback_invoked": step.rollback_invoked,
                                "timestamp": step.timestamp.to_rfc3339(),
                            })
                        })
                        .collect();
                    Ok(Self::success_response(
                        id,
                        serde_json::json!({
                            "session_id": session_id_str,
                            "trace": {
                                "total_steps": steps.len(),
                                "from_step": start,
                                "returned": trace_steps.len(),
                                "steps": trace_steps
                            }
                        }),
                    ))
                }
                None => Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("session not found: {session_id_str}"),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "trace": {
                        "total_steps": 0,
                        "from_step": 0,
                        "returned": 0,
                        "steps": []
                    }
                }),
            ))
        }
    }

    fn handle_pause_session(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let session_id_str = match Self::extract_session_id(&params) {
            Some(sid) => sid,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "session_id required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = &self.runtime {
            let uuid = match Uuid::parse_str(&session_id_str) {
                Ok(u) => u,
                Err(_) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        "session_id must be a valid UUID".to_string(),
                        None,
                    ));
                }
            };

            let mut ctrl = rt.session_controller.lock().unwrap();
            // Get a mutable clone of the session, pause it, then store it back.
            let mut session = match ctrl.get_session(&uuid).cloned() {
                Some(s) => s,
                None => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("session not found: {session_id_str}"),
                        None,
                    ));
                }
            };

            match ctrl.pause(&mut session) {
                Ok(()) => Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "session_id": session_id_str,
                        "status": format!("{:?}", session.status)
                    }),
                )),
                Err(e) => Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("pause failed: {e}"),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "status": "paused"
                }),
            ))
        }
    }

    fn handle_resume_session(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let session_id_str = match Self::extract_session_id(&params) {
            Some(sid) => sid,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "session_id required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = &self.runtime {
            let uuid = match Uuid::parse_str(&session_id_str) {
                Ok(u) => u,
                Err(_) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        "session_id must be a valid UUID".to_string(),
                        None,
                    ));
                }
            };

            let mut ctrl = rt.session_controller.lock().unwrap();
            let mut session = match ctrl.get_session(&uuid).cloned() {
                Some(s) => s,
                None => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("session not found: {session_id_str}"),
                        None,
                    ));
                }
            };

            match ctrl.resume(&mut session) {
                Ok(()) => Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "session_id": session_id_str,
                        "status": format!("{:?}", session.status)
                    }),
                )),
                Err(e) => Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("resume failed: {e}"),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "status": "running"
                }),
            ))
        }
    }

    fn handle_abort_session(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let session_id_str = match Self::extract_session_id(&params) {
            Some(sid) => sid,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "session_id required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = &self.runtime {
            let uuid = match Uuid::parse_str(&session_id_str) {
                Ok(u) => u,
                Err(_) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        "session_id must be a valid UUID".to_string(),
                        None,
                    ));
                }
            };

            let mut ctrl = rt.session_controller.lock().unwrap();
            let mut session = match ctrl.get_session(&uuid).cloned() {
                Some(s) => s,
                None => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("session not found: {session_id_str}"),
                        None,
                    ));
                }
            };

            match ctrl.abort(&mut session) {
                Ok(()) => Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "session_id": session_id_str,
                        "status": format!("{:?}", session.status)
                    }),
                )),
                Err(e) => Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("abort failed: {e}"),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "status": "aborted"
                }),
            ))
        }
    }

    fn handle_list_sessions(&self, id: Value, _params: Option<Value>) -> Result<McpResponse> {
        if let Some(rt) = &self.runtime {
            let ctrl = rt.session_controller.lock().unwrap();
            let sessions: Vec<Value> = ctrl
                .list_sessions()
                .iter()
                .map(|(sid, status)| {
                    serde_json::json!({
                        "session_id": sid.to_string(),
                        "status": status,
                    })
                })
                .collect();
            Ok(Self::success_response(
                id,
                serde_json::json!({ "sessions": sessions }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({ "sessions": [] }),
            ))
        }
    }

    fn handle_query_metrics(&self, id: Value, _params: Option<Value>) -> Result<McpResponse> {
        match &self.runtime {
            Some(rt) => {
                let snap = rt.metrics.snapshot();
                Ok(Self::success_response(
                    id,
                    serde_json::json!({ "metrics": snap.format_json() }),
                ))
            }
            None => {
                // Stub mode: return empty metrics.
                let snap = RuntimeMetrics::new().snapshot();
                Ok(Self::success_response(
                    id,
                    serde_json::json!({ "metrics": snap.format_json() }),
                ))
            }
        }
    }

    fn handle_query_policy(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let action = params
            .as_ref()
            .and_then(|p| p.get("action"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "action": action,
                "decision": {
                    "allowed": true,
                    "effect": "allow",
                    "matched_rules": [],
                    "reason": "no policy rules loaded",
                    "constraints": null
                }
            }),
        ))
    }

    fn handle_dump_state(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let sections: Vec<String> = params
            .as_ref()
            .and_then(|p| p.get("sections"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["full".to_string()]);

        if let Some(rt) = &self.runtime {
            let ctrl = rt.session_controller.lock().unwrap();
            let skill_rt = rt.skill_runtime.lock().unwrap();
            let port_rt_clone = Arc::clone(&rt.port_runtime);
            let specs = rt.pack_specs.lock().unwrap();

            let full = sections.iter().any(|s| s == "full");
            let mut dump = serde_json::Map::new();

            if full || sections.iter().any(|s| s == "belief") {
                let sessions = ctrl.list_sessions();
                let mut beliefs = Vec::new();
                for (sid, _status) in &sessions {
                    if let Some(session) = ctrl.get_session(sid) {
                        let belief = &session.belief;
                        beliefs.push(serde_json::json!({
                            "session_id": sid.to_string(),
                            "belief_id": belief.belief_id.to_string(),
                            "resources": serde_json::to_value(&belief.resources).unwrap_or_default(),
                            "facts": belief.facts.iter().map(|f| {
                                serde_json::json!({
                                    "fact_id": f.fact_id,
                                    "subject": f.subject,
                                    "predicate": f.predicate,
                                    "value": f.value,
                                    "confidence": f.confidence,
                                })
                            }).collect::<Vec<_>>(),
                            "uncertainties": &belief.uncertainties,
                        }));
                    }
                }
                dump.insert("belief".to_string(), serde_json::json!(beliefs));
            }

            if full || sections.iter().any(|s| s == "episodes") {
                let episodes = rt.episode_store.lock()
                    .map(|es| {
                        es.list(1000, 0)
                            .into_iter()
                            .map(|ep| serde_json::to_value(ep).unwrap_or_default())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                dump.insert("episodes".to_string(), serde_json::json!(episodes));
            }

            if full || sections.iter().any(|s| s == "schemas") {
                let schemas = rt.schema_store.lock()
                    .map(|ss| {
                        ss.list_all()
                            .into_iter()
                            .map(|s| serde_json::to_value(s).unwrap_or_default())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                dump.insert("schemas".to_string(), serde_json::json!(schemas));
            }

            if full || sections.iter().any(|s| s == "routines") {
                let routines = rt.routine_store.lock()
                    .map(|rs| {
                        rs.list_all()
                            .into_iter()
                            .map(|r| serde_json::to_value(r).unwrap_or_default())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                dump.insert("routines".to_string(), serde_json::json!(routines));
            }

            if full || sections.iter().any(|s| s == "sessions") {
                let session_list = ctrl.list_sessions();
                let mut session_details = Vec::new();
                for (sid, status) in &session_list {
                    if let Some(session) = ctrl.get_session(sid) {
                        session_details.push(serde_json::json!({
                            "session_id": sid.to_string(),
                            "status": status,
                            "objective": session.goal.objective.description,
                            "budget_remaining": {
                                "risk_remaining": session.budget_remaining.risk_remaining,
                                "latency_remaining_ms": session.budget_remaining.latency_remaining_ms,
                                "resource_remaining": session.budget_remaining.resource_remaining,
                                "steps_remaining": session.budget_remaining.steps_remaining,
                            },
                            "trace_steps": session.trace.steps.len(),
                            "working_memory": {
                                "active_bindings": session.working_memory.active_bindings.len(),
                                "unresolved_slots": &session.working_memory.unresolved_slots,
                                "current_subgoal": &session.working_memory.current_subgoal,
                                "candidate_shortlist": &session.working_memory.candidate_shortlist,
                            },
                            "trace": session.trace.steps.iter().map(|step| {
                                serde_json::json!({
                                    "step_index": step.step_index,
                                    "selected_skill": step.selected_skill,
                                    "observation_id": step.observation_id.to_string(),
                                    "critic_decision": step.critic_decision,
                                    "progress_delta": step.progress_delta,
                                    "timestamp": step.timestamp.to_rfc3339(),
                                })
                            }).collect::<Vec<_>>(),
                            "created_at": session.created_at.to_rfc3339(),
                            "updated_at": session.updated_at.to_rfc3339(),
                        }));
                    } else {
                        session_details.push(serde_json::json!({
                            "session_id": sid.to_string(),
                            "status": status,
                        }));
                    }
                }
                dump.insert("sessions".to_string(), serde_json::json!(session_details));
            }

            if full || sections.iter().any(|s| s == "skills") {
                let skills = skill_rt.list_skills(None);
                let skill_json: Vec<Value> = skills
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "skill_id": s.skill_id,
                            "name": s.name,
                            "namespace": s.namespace,
                            "pack": s.pack,
                            "kind": format!("{:?}", s.kind),
                            "description": s.description,
                            "inputs": s.inputs.schema,
                            "outputs": s.outputs.schema,
                            "risk_class": format!("{:?}", s.risk_class),
                            "determinism": format!("{:?}", s.determinism),
                            "capability_requirements": s.capability_requirements,
                        })
                    })
                    .collect();
                dump.insert("skills".to_string(), serde_json::json!(skill_json));
            }

            if full || sections.iter().any(|s| s == "ports") {
                let ports = port_rt_clone.lock()
                    .map(|pr| {
                        pr.list_ports(None)
                            .iter()
                            .map(|p| {
                                serde_json::json!({
                                    "port_id": p.port_id,
                                    "name": p.name,
                                    "namespace": p.namespace,
                                    "kind": format!("{:?}", p.kind),
                                    "capabilities": p.capabilities.iter().map(|c| {
                                        serde_json::json!({
                                            "capability_id": c.capability_id,
                                            "name": c.name,
                                            "purpose": &c.purpose,
                                        })
                                    }).collect::<Vec<_>>(),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                dump.insert("ports".to_string(), serde_json::json!(ports));
            }

            if full || sections.iter().any(|s| s == "packs") {
                let packs: Vec<Value> = specs
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "pack_id": p.id,
                            "name": p.name,
                            "namespace": p.namespace,
                            "version": p.version.to_string(),
                            "description": p.description,
                            "skills_count": p.skills.len(),
                            "ports_count": p.ports.len(),
                            "schemas_count": p.schemas.len(),
                            "routines_count": p.routines.len(),
                            "policies_count": p.policies.len(),
                        })
                    })
                    .collect();
                dump.insert("packs".to_string(), serde_json::json!(packs));
            }

            if full || sections.iter().any(|s| s == "metrics") {
                use std::sync::atomic::Ordering;

                let ep_count = rt.episode_store.lock().map(|s| s.count()).unwrap_or(0);
                rt.metrics.episodes_stored.store(ep_count as u64, Ordering::Relaxed);

                let snap = rt.metrics.snapshot();

                let counts = crate::runtime::proprioception::RuntimeCounts {
                    active_sessions: rt.metrics.active_sessions.load(Ordering::Relaxed),
                    loaded_packs: specs.len() as u64,
                    registered_skills: skill_rt.list_skills(None).len() as u64,
                    registered_ports: port_rt_clone.lock()
                        .map(|pr| pr.list_ports(None).len() as u64)
                        .unwrap_or(0),
                    peer_connections: 0,
                };
                let self_model = crate::runtime::proprioception::snapshot(rt.start_time, &counts);

                let mut metrics_json = snap.format_json();
                if let Value::Object(ref mut map) = metrics_json {
                    map.insert("self_model".to_string(), self_model.to_json());
                }
                dump.insert("metrics".to_string(), metrics_json);
            }

            Ok(Self::success_response(id, Value::Object(dump)))
        } else {
            // Stub mode: return empty dump with all section keys.
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "belief": [],
                    "episodes": [],
                    "schemas": [],
                    "routines": [],
                    "sessions": [],
                    "skills": [],
                    "ports": [],
                    "packs": [],
                    "metrics": {}
                }),
            ))
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn extract_session_id(params: &Option<Value>) -> Option<String> {
        params
            .as_ref()
            .and_then(|p| p.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn success_response(id: Value, result: Value) -> McpResponse {
        McpResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error_response(id: Value, code: i64, message: String, data: Option<Value>) -> McpResponse {
        McpResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(McpError {
                code,
                message,
                data,
            }),
            id,
        }
    }

    // -----------------------------------------------------------------------
    // Tool definitions
    // -----------------------------------------------------------------------

    fn build_tools() -> Vec<McpTool> {
        vec![
            McpTool {
                name: "create_goal".to_string(),
                description: "Submit a goal to the SOMA runtime. Returns session_id for tracking."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "objective": {
                            "type": "string",
                            "description": "The goal objective description"
                        },
                        "constraints": {
                            "type": "array",
                            "description": "Optional constraints on goal execution",
                            "items": { "type": "object" }
                        },
                        "risk_budget": {
                            "type": "number",
                            "description": "Maximum risk budget (0.0 - 1.0)"
                        },
                        "latency_budget_ms": {
                            "type": "integer",
                            "description": "Maximum latency in milliseconds"
                        },
                        "resource_budget": {
                            "type": "number",
                            "description": "Maximum resource budget"
                        },
                        "priority": {
                            "type": "string",
                            "enum": ["low", "normal", "high", "critical"],
                            "description": "Goal priority level"
                        },
                        "permissions_scope": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Required permission scopes"
                        }
                    },
                    "required": ["objective"]
                }),
            },
            McpTool {
                name: "inspect_session".to_string(),
                description: "Get session status, working memory, and budget for a session."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
            McpTool {
                name: "inspect_belief".to_string(),
                description: "Get the current belief state for a session.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
            McpTool {
                name: "inspect_resources".to_string(),
                description: "List or get resources known to the runtime.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "resource_type": {
                            "type": "string",
                            "description": "Optional filter by resource type"
                        },
                        "resource_id": {
                            "type": "string",
                            "description": "Optional specific resource ID"
                        }
                    }
                }),
            },
            McpTool {
                name: "inspect_packs".to_string(),
                description: "List loaded packs and their lifecycle status.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pack_id": {
                            "type": "string",
                            "description": "Optional specific pack ID"
                        }
                    }
                }),
            },
            McpTool {
                name: "inspect_skills".to_string(),
                description: "List available skills across all loaded packs.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pack": {
                            "type": "string",
                            "description": "Optional filter by pack name"
                        },
                        "kind": {
                            "type": "string",
                            "enum": ["primitive", "composite", "routine", "delegated"],
                            "description": "Optional filter by skill kind"
                        }
                    }
                }),
            },
            McpTool {
                name: "inspect_trace".to_string(),
                description: "Get the session trace (step-by-step execution log).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        },
                        "from_step": {
                            "type": "integer",
                            "description": "Optional starting step index"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Optional maximum number of steps to return"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
            McpTool {
                name: "pause_session".to_string(),
                description: "Pause a running session.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
            McpTool {
                name: "resume_session".to_string(),
                description: "Resume a paused session.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
            McpTool {
                name: "abort_session".to_string(),
                description: "Abort a session (cannot be resumed).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
            McpTool {
                name: "list_sessions".to_string(),
                description: "List all sessions with their current status.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpTool {
                name: "query_metrics".to_string(),
                description: "Get runtime metrics (sessions, skills, ports, uptime).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "metric_names": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of specific metric names"
                        }
                    }
                }),
            },
            McpTool {
                name: "query_policy".to_string(),
                description: "Query policy decisions for a given action.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "The action to check policy for"
                        },
                        "target": {
                            "type": "string",
                            "description": "The target resource or skill"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "Optional session context"
                        }
                    },
                    "required": ["action"]
                }),
            },
            McpTool {
                name: "dump_state".to_string(),
                description: "Dump full runtime state as structured JSON. An LLM can call this to get a complete snapshot of SOMA's belief states, episodes, schemas, routines, sessions, skills, ports, packs, and metrics.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "sections": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["full", "belief", "episodes", "schemas", "routines", "sessions", "skills", "ports", "packs", "metrics"]
                            },
                            "description": "Which sections to include. Omit or pass [\"full\"] for everything."
                        }
                    }
                }),
            },
        ]
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new_stub()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(method: &str, params: Option<Value>) -> McpRequest {
        McpRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Value::Number(1.into()),
        }
    }

    #[test]
    fn test_new_server() {
        let server = McpServer::new_stub();
        assert!(!server.list_tools().is_empty());
    }

    #[test]
    fn test_list_tools_count() {
        let server = McpServer::new_stub();
        let tools = server.list_tools();
        assert_eq!(tools.len(), 14);
    }

    #[test]
    fn test_list_tools_names() {
        let server = McpServer::new_stub();
        let names: Vec<String> = server.list_tools().iter().map(|t| t.name.clone()).collect();
        assert!(names.contains(&"create_goal".to_string()));
        assert!(names.contains(&"inspect_session".to_string()));
        assert!(names.contains(&"inspect_belief".to_string()));
        assert!(names.contains(&"inspect_resources".to_string()));
        assert!(names.contains(&"inspect_packs".to_string()));
        assert!(names.contains(&"inspect_skills".to_string()));
        assert!(names.contains(&"inspect_trace".to_string()));
        assert!(names.contains(&"pause_session".to_string()));
        assert!(names.contains(&"resume_session".to_string()));
        assert!(names.contains(&"abort_session".to_string()));
        assert!(names.contains(&"list_sessions".to_string()));
        assert!(names.contains(&"query_metrics".to_string()));
        assert!(names.contains(&"query_policy".to_string()));
        assert!(names.contains(&"dump_state".to_string()));
    }

    #[test]
    fn test_invalid_jsonrpc_version() {
        let server = McpServer::new_stub();
        let req = McpRequest {
            jsonrpc: "1.0".to_string(),
            method: "create_goal".to_string(),
            params: None,
            id: Value::Number(1.into()),
        };
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_REQUEST);
    }

    #[test]
    fn test_method_not_found() {
        let server = McpServer::new_stub();
        let req = make_request("nonexistent_method", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn test_create_goal_success() {
        let server = McpServer::new_stub();
        let req = make_request(
            "create_goal",
            Some(serde_json::json!({ "objective": "list files in /tmp" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "created");
        assert_eq!(result["objective"], "list files in /tmp");
        assert!(result["session_id"].as_str().is_some());
        assert!(result["goal_id"].as_str().is_some());
    }

    #[test]
    fn test_create_goal_missing_objective() {
        let server = McpServer::new_stub();
        let req = make_request("create_goal", Some(serde_json::json!({})));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_create_goal_empty_objective() {
        let server = McpServer::new_stub();
        let req = make_request(
            "create_goal",
            Some(serde_json::json!({ "objective": "" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_create_goal_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("create_goal", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_inspect_session() {
        let server = McpServer::new_stub();
        let sid = Uuid::new_v4().to_string();
        let req = make_request(
            "inspect_session",
            Some(serde_json::json!({ "session_id": sid })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["session_id"], sid);
        assert_eq!(result["status"], "created");
    }

    #[test]
    fn test_inspect_session_missing_id() {
        let server = McpServer::new_stub();
        let req = make_request("inspect_session", Some(serde_json::json!({})));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_inspect_belief() {
        let server = McpServer::new_stub();
        let sid = Uuid::new_v4().to_string();
        let req = make_request(
            "inspect_belief",
            Some(serde_json::json!({ "session_id": sid })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["session_id"], sid);
        assert!(result["belief"].is_object());
    }

    #[test]
    fn test_inspect_resources() {
        let server = McpServer::new_stub();
        let req = make_request("inspect_resources", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["resources"].is_array());
    }

    #[test]
    fn test_inspect_packs() {
        let server = McpServer::new_stub();
        let req = make_request("inspect_packs", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["packs"].is_array());
    }

    #[test]
    fn test_inspect_skills() {
        let server = McpServer::new_stub();
        let req = make_request("inspect_skills", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["skills"].is_array());
    }

    #[test]
    fn test_inspect_trace() {
        let server = McpServer::new_stub();
        let sid = Uuid::new_v4().to_string();
        let req = make_request(
            "inspect_trace",
            Some(serde_json::json!({ "session_id": sid })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["session_id"], sid);
        assert!(result["trace"]["steps"].is_array());
    }

    #[test]
    fn test_pause_session() {
        let server = McpServer::new_stub();
        let sid = Uuid::new_v4().to_string();
        let req = make_request(
            "pause_session",
            Some(serde_json::json!({ "session_id": sid })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "paused");
    }

    #[test]
    fn test_resume_session() {
        let server = McpServer::new_stub();
        let sid = Uuid::new_v4().to_string();
        let req = make_request(
            "resume_session",
            Some(serde_json::json!({ "session_id": sid })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "running");
    }

    #[test]
    fn test_abort_session() {
        let server = McpServer::new_stub();
        let sid = Uuid::new_v4().to_string();
        let req = make_request(
            "abort_session",
            Some(serde_json::json!({ "session_id": sid })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "aborted");
    }

    #[test]
    fn test_query_metrics() {
        let server = McpServer::new_stub();
        let req = make_request("query_metrics", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["metrics"].is_object());
    }

    #[test]
    fn test_query_policy() {
        let server = McpServer::new_stub();
        let req = make_request(
            "query_policy",
            Some(serde_json::json!({ "action": "execute_skill" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["action"], "execute_skill");
        assert!(result["decision"]["allowed"].as_bool().unwrap());
    }

    #[test]
    fn test_initialize() {
        let server = McpServer::new_stub();
        let req = make_request("initialize", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["serverInfo"]["name"].as_str().is_some());
    }

    #[test]
    fn test_tools_list_method() {
        let server = McpServer::new_stub();
        let req = make_request("tools/list", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["tools"].is_array());
    }

    #[test]
    fn test_tools_call_dispatch() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "create_goal",
                "arguments": { "objective": "test goal" }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "created");
    }

    #[test]
    fn test_tools_call_unknown_tool() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "nonexistent_tool",
                "arguments": {}
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn test_tools_call_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("tools/call", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_response_id_preserved() {
        let server = McpServer::new_stub();
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: "query_metrics".to_string(),
            params: None,
            id: Value::String("req-42".to_string()),
        };
        let resp = server.handle_request(req).unwrap();
        assert_eq!(resp.id, Value::String("req-42".to_string()));
    }

    #[test]
    fn test_response_jsonrpc_version() {
        let server = McpServer::new_stub();
        let req = make_request("query_metrics", None);
        let resp = server.handle_request(req).unwrap();
        assert_eq!(resp.jsonrpc, "2.0");
    }

    #[test]
    fn test_tool_schemas_have_required_fields() {
        let server = McpServer::new_stub();
        for tool in server.list_tools() {
            assert!(!tool.name.is_empty(), "tool name must not be empty");
            assert!(
                !tool.description.is_empty(),
                "tool description must not be empty: {}",
                tool.name
            );
            assert!(
                tool.input_schema.is_object(),
                "tool input_schema must be an object: {}",
                tool.name
            );
            assert_eq!(
                tool.input_schema["type"], "object",
                "tool input_schema type must be 'object': {}",
                tool.name
            );
        }
    }

    #[test]
    fn test_mcp_request_serialization_roundtrip() {
        let req = make_request(
            "create_goal",
            Some(serde_json::json!({ "objective": "test" })),
        );
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: McpRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.method, "create_goal");
        assert_eq!(deserialized.jsonrpc, "2.0");
    }

    #[test]
    fn test_mcp_response_serialization() {
        let resp = McpServer::success_response(
            Value::Number(1.into()),
            serde_json::json!({"ok": true}),
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"result\""));
        // error should be omitted (skip_serializing_if)
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_mcp_error_response_serialization() {
        let resp = McpServer::error_response(
            Value::Number(1.into()),
            INTERNAL_ERROR,
            "something broke".to_string(),
            None,
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));
        assert!(!json.contains("\"result\""));
    }

    #[test]
    fn test_default_impl() {
        let server = McpServer::default();
        assert_eq!(server.list_tools().len(), 14);
    }

    /// Build a real RuntimeHandle from a bootstrapped runtime (no packs).
    fn make_wired_server() -> McpServer {
        let config = crate::config::SomaConfig::default();
        let runtime = crate::bootstrap::bootstrap(&config, &[]).unwrap();
        let handle = RuntimeHandle::from_runtime(runtime);
        McpServer::new(handle)
    }

    #[test]
    fn test_wired_list_sessions_empty() {
        let server = make_wired_server();
        let req = make_request("list_sessions", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["sessions"].is_array());
        assert_eq!(result["sessions"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_wired_inspect_skills_empty_no_packs() {
        let server = make_wired_server();
        let req = make_request("inspect_skills", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["skills"].is_array());
    }

    #[test]
    fn test_wired_inspect_session_not_found() {
        let server = make_wired_server();
        let fake_id = Uuid::new_v4().to_string();
        let req = make_request(
            "inspect_session",
            Some(serde_json::json!({ "session_id": fake_id })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("not found"));
    }

    #[test]
    fn test_wired_inspect_trace_not_found() {
        let server = make_wired_server();
        let fake_id = Uuid::new_v4().to_string();
        let req = make_request(
            "inspect_trace",
            Some(serde_json::json!({ "session_id": fake_id })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("not found"));
    }

    #[test]
    fn test_wired_pause_session_not_found() {
        let server = make_wired_server();
        let fake_id = Uuid::new_v4().to_string();
        let req = make_request(
            "pause_session",
            Some(serde_json::json!({ "session_id": fake_id })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("not found"));
    }

    #[test]
    fn test_wired_abort_session_not_found() {
        let server = make_wired_server();
        let fake_id = Uuid::new_v4().to_string();
        let req = make_request(
            "abort_session",
            Some(serde_json::json!({ "session_id": fake_id })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("not found"));
    }

    #[test]
    fn test_wired_query_metrics() {
        let server = make_wired_server();
        let req = make_request("query_metrics", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["metrics"]["active_sessions"], 0);
    }

    #[test]
    fn test_dump_state_stub_full() {
        let server = McpServer::new_stub();
        let req = make_request("dump_state", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["belief"].is_array());
        assert!(result["episodes"].is_array());
        assert!(result["schemas"].is_array());
        assert!(result["routines"].is_array());
        assert!(result["sessions"].is_array());
        assert!(result["skills"].is_array());
        assert!(result["ports"].is_array());
        assert!(result["packs"].is_array());
        assert!(result["metrics"].is_object());
    }

    #[test]
    fn test_dump_state_stub_specific_sections() {
        let server = McpServer::new_stub();
        let req = make_request(
            "dump_state",
            Some(serde_json::json!({ "sections": ["belief", "metrics"] })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["belief"].is_array());
        assert!(result["metrics"].is_object());
    }

    #[test]
    fn test_dump_state_via_tools_call() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "dump_state",
                "arguments": { "sections": ["full"] }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["belief"].is_array());
    }

    #[test]
    fn test_wired_dump_state_empty_runtime() {
        let server = make_wired_server();
        let req = make_request(
            "dump_state",
            Some(serde_json::json!({ "sections": ["full"] })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["sessions"].is_array());
        assert!(result["skills"].is_array());
        assert!(result["ports"].is_array());
        assert!(result["packs"].is_array());
        assert!(result["episodes"].is_array());
        assert!(result["schemas"].is_array());
        assert!(result["routines"].is_array());
        assert!(result["metrics"].is_object());
        assert!(result["metrics"]["self_model"].is_object());
    }

    #[test]
    fn test_wired_create_goal_runs_session() {
        let server = make_wired_server();
        let req = make_request(
            "create_goal",
            Some(serde_json::json!({ "objective": "list files in /tmp" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        // With no packs loaded, session creation succeeds but execution fails
        // because there are no skill candidates.
        assert!(result["session_id"].is_string());
        assert!(result["goal_id"].is_string());
        assert_eq!(result["objective"], "list files in /tmp");
    }

    #[test]
    fn test_stub_list_sessions() {
        let server = McpServer::new_stub();
        let req = make_request("list_sessions", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["sessions"].is_array());
    }
}
