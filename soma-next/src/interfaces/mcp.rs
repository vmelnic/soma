use std::sync::{Arc, Mutex};

use web_time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::runtime::remote::RemoteExecutor;
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

/// JSON-RPC 2.0 request (or notification when `id` is absent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
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
    #[serde(rename = "inputSchema")]
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
    pub embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync>,
    pub schedule_store: Arc<Mutex<dyn crate::runtime::scheduler::ScheduleStore + Send>>,
    pub world_state: Arc<Mutex<dyn crate::runtime::world_state::WorldStateStore + Send>>,
    pub routine_router: Arc<dyn crate::distributed::routing::RoutineRouter>,
    pub start_time: Instant,
    pub remote_executor: Option<Arc<dyn RemoteExecutor>>,
    /// Live list of peer IDs known to the runtime. Static peers from
    /// `--peer` are pushed in during bootstrap; mDNS-discovered peers
    /// (when `--discover-lan` is active) push into the same list as they
    /// appear and remove themselves on TTL expiry.
    pub peer_ids: Arc<Mutex<Vec<String>>>,
    /// Belief synchronization manager for syncing belief facts with peers.
    pub belief_sync: Arc<Mutex<dyn crate::distributed::sync::BeliefSync + Send>>,
    /// Observation streaming for monitoring remote skill invocations.
    pub observation_streaming: Arc<Mutex<dyn crate::distributed::streaming::ObservationStreaming + Send>>,
    /// Delegation manager for routing skills/sessions to remote peers.
    pub delegation_manager: Option<Arc<dyn crate::distributed::delegation::DelegationManager>>,
    /// Shared session checkpoint store used for mid-run persistence and
    /// resume-on-boot.
    pub checkpoint_store: Arc<crate::memory::checkpoint::SessionCheckpointStore>,
    /// How often (in control-loop steps) to write mid-run checkpoints.
    /// Zero disables mid-run checkpointing.
    pub checkpoint_every_n_steps: u32,
    /// Registry of in-flight asynchronous goals submitted via
    /// `create_goal_async`. Each entry carries a shared session handle and
    /// a cancel flag; the background thread updates status on termination.
    pub goal_registry: Arc<crate::runtime::goal_registry::GoalRegistry>,
    /// Per-skill EMA stats accumulated from completed episodes; updated
    /// by `goal_executor::finalize_episode` and read for `inspect_skills`.
    pub skill_stats: crate::memory::skill_stats::SharedSkillStats,
    /// Plugin search paths for the dynamic dylib loader. Used by
    /// `reload_pack` to rebuild a `DynamicPortLoader` on demand.
    pub plugin_search_paths: Vec<std::path::PathBuf>,
    pub require_port_signatures: bool,
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
            embedder: runtime.embedder,
            schedule_store: runtime.schedule_store,
            world_state: runtime.world_state,
            routine_router: runtime.routine_router,
            start_time: runtime.start_time,
            remote_executor: None,
            peer_ids: Arc::new(Mutex::new(Vec::new())),
            belief_sync: Arc::new(Mutex::new(
                crate::distributed::sync::DefaultBeliefSync::new(),
            )),
            observation_streaming: Arc::new(Mutex::new(
                crate::distributed::streaming::DefaultObservationStreaming::new(),
            )),
            delegation_manager: None,
            checkpoint_store: runtime.checkpoint_store,
            checkpoint_every_n_steps: runtime.checkpoint_every_n_steps,
            goal_registry: Arc::new(crate::runtime::goal_registry::GoalRegistry::new()),
            skill_stats: runtime.skill_stats,
            plugin_search_paths: runtime.plugin_search_paths,
            require_port_signatures: runtime.require_port_signatures,
        }
    }

    /// Attach a remote executor and the list of known peer IDs.
    pub fn with_remote(
        mut self,
        executor: Box<dyn RemoteExecutor>,
        peer_ids: Vec<String>,
    ) -> Self {
        self.remote_executor = Some(Arc::from(executor));
        self.peer_ids = Arc::new(Mutex::new(peer_ids));
        self
    }

    /// Attach a remote executor and a shared mutable peer-id list. Used
    /// when the peer list is managed by a dynamic subsystem such as mDNS
    /// LAN discovery — the discovery task pushes into the same mutex.
    pub fn with_remote_shared(
        mut self,
        executor: Box<dyn RemoteExecutor>,
        peer_ids: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        self.remote_executor = Some(Arc::from(executor));
        self.peer_ids = peer_ids;
        self
    }

    /// Build a closure that launches async goals via this handle. Used by
    /// the webhook listener so incoming POSTs can trigger full-blown
    /// autonomous runs without re-doing the plumbing. The returned
    /// launcher clones the Arcs it needs, so the handle can still be
    /// consumed by `McpServer::new`.
    pub fn build_webhook_launcher(
        &self,
    ) -> crate::distributed::webhook_listener::WebhookGoalLauncher {
        let session_controller = Arc::clone(&self.session_controller);
        let goal_runtime = Arc::clone(&self.goal_runtime);
        let goal_registry = Arc::clone(&self.goal_registry);
        let checkpoint_store = Arc::clone(&self.checkpoint_store);
        let checkpoint_every_n = self.checkpoint_every_n_steps;
        let episode_store = Arc::clone(&self.episode_store);
        let schema_store = Arc::clone(&self.schema_store);
        let routine_store = Arc::clone(&self.routine_store);
        let embedder = Arc::clone(&self.embedder);
        let world_state = Arc::clone(&self.world_state);
        let skill_stats = Arc::clone(&self.skill_stats);

        Arc::new(move |objective: String, max_steps: Option<u32>| {
            let input = crate::runtime::goal::GoalInput::NaturalLanguage {
                text: objective.clone(),
                source: crate::types::goal::GoalSource {
                    source_type: crate::types::goal::GoalSourceType::Api,
                    identity: Some("webhook".into()),
                    session_id: None,
                    peer_id: None,
                },
            };
            let mut goal = {
                let goal_rt = goal_runtime.lock().unwrap();
                goal_rt.parse_goal(input)?
            };
            {
                let goal_rt = goal_runtime.lock().unwrap();
                goal_rt.normalize_goal(&mut goal);
                goal_rt.validate_goal(&goal)?;
            }
            if let Some(ms) = max_steps {
                goal.max_steps = Some(ms);
            }
            let goal_id = goal.goal_id;
            let session = {
                let mut ctrl = session_controller.lock().unwrap();
                ctrl.create_session(goal)?
            };
            let _ = &objective;
            let entry = Arc::new(
                crate::runtime::goal_registry::AsyncGoalEntry::new(goal_id, session),
            );
            goal_registry.insert(Arc::clone(&entry));
            let ctx = crate::runtime::goal_registry::OwnedEpisodeContext {
                episode_store: Arc::clone(&episode_store),
                schema_store: Arc::clone(&schema_store),
                routine_store: Arc::clone(&routine_store),
                embedder: Arc::clone(&embedder),
                world_state: Arc::clone(&world_state),
                skill_stats: Some(Arc::clone(&skill_stats)),
            };
            crate::runtime::goal_registry::spawn_async_goal(
                Arc::clone(&entry),
                Arc::clone(&session_controller),
                Arc::clone(&checkpoint_store),
                checkpoint_every_n,
                ctx,
            );
            Ok(goal_id)
        })
    }
}

// ---------------------------------------------------------------------------
// Implicit sessions — bridge LLM-driven invoke_port calls to the learning
// pipeline by grouping sequential calls into episodes.
// ---------------------------------------------------------------------------

/// Groups sequential `invoke_port` calls within a time window into a single
/// logical session. When the caller goes quiet (no `invoke_port` for
/// `IMPLICIT_SESSION_TIMEOUT`) or switches to a non-invoke_port MCP call,
/// the session is finalized and stored as an episode for schema induction
/// and routine compilation.
struct ImplicitSession {
    /// Ordered sequence of (port_id, capability_id) from invoke_port calls.
    skill_sequence: Vec<(String, String)>,
    /// The `PortCallRecord` from each successful invoke_port call.
    observations: Vec<crate::types::observation::PortCallRecord>,
    /// When the first invoke_port in this session was called.
    started_at: Instant,
    /// When the most recent invoke_port was called.
    last_activity: Instant,
}

/// How long to wait after the last invoke_port before considering the
/// implicit session complete.
const IMPLICIT_SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
    runtime: std::sync::OnceLock<RuntimeHandle>,
    /// Tracks the current implicit session for LLM-driven invoke_port calls.
    /// When finalized, the session becomes an episode fed into the learning
    /// pipeline (schema induction / routine compilation).
    implicit_session: Mutex<Option<ImplicitSession>>,
}

impl McpServer {
    /// Create an MCP server wired to real runtime subsystems.
    pub fn new(runtime: RuntimeHandle) -> Self {
        let lock = std::sync::OnceLock::new();
        let _ = lock.set(runtime);
        Self {
            tools: Self::build_tools(),
            runtime: lock,
            implicit_session: Mutex::new(None),
        }
    }

    /// Create a stub MCP server without any runtime backing.
    /// The runtime can be installed later via `install_runtime`.
    pub fn new_stub() -> Self {
        Self {
            tools: Self::build_tools(),
            runtime: std::sync::OnceLock::new(),
            implicit_session: Mutex::new(None),
        }
    }

    /// Hot-install a runtime into a stub server. Called from the background
    /// bootstrap thread once all packs are loaded.
    pub fn install_runtime(&self, runtime: RuntimeHandle) {
        let n_skills = runtime.skill_runtime.lock().unwrap().list_skills(None).len();
        match self.runtime.set(runtime) {
            Ok(()) => eprintln!("MCP: runtime installed ({n_skills} skills ready)"),
            Err(_) => eprintln!("MCP: warning: runtime already installed"),
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

        // Flush any open implicit session when the caller switches away from
        // invoke_port. For tools/call, check the inner tool name.
        let is_invoke_port = match request.method.as_str() {
            "invoke_port" => true,
            "tools/call" => request
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                == Some("invoke_port"),
            _ => false,
        };
        if !is_invoke_port {
            self.flush_implicit_session();
        }

        match request.method.as_str() {
            // MCP protocol methods
            "initialize" => self.handle_initialize(request.id, request.params),
            "tools/list" => self.handle_tools_list(request.id),
            "tools/call" => self.handle_tools_call(request.id, request.params),

            // Direct tool methods (for clients that invoke tools as methods)
            "create_goal" => self.handle_create_goal(request.id, request.params),
            "create_goal_async" => self.handle_create_goal_async(request.id, request.params),
            "get_goal_status" => self.handle_get_goal_status(request.id, request.params),
            "stream_goal_observations" => self.handle_stream_goal_observations(request.id, request.params),
            "cancel_goal" => self.handle_cancel_goal(request.id, request.params),
            "inspect_session" => self.handle_inspect_session(request.id, request.params),
            "inspect_belief" => self.handle_inspect_belief(request.id, request.params),
            "inspect_belief_projection" => self.handle_inspect_belief_projection(request.id, request.params),
            "provide_session_input" => self.handle_provide_session_input(request.id, request.params),
            "inject_plan" => self.handle_inject_plan(request.id, request.params),
            "find_routines" => self.handle_find_routines(request.id, request.params),
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
            "invoke_port" => self.handle_invoke_port(request.id, request.params),
            "list_ports" => self.handle_list_ports(request.id, request.params),
            "list_capabilities" => self.handle_list_capabilities(request.id, request.params),
            "list_peers" => self.handle_list_peers(request.id, request.params),
            "invoke_remote_skill" => self.handle_invoke_remote_skill(request.id, request.params),
            "transfer_routine" => self.handle_transfer_routine(request.id, request.params),
            "schedule" => self.handle_schedule(request.id, request.params),
            "list_schedules" => self.handle_list_schedules(request.id, request.params),
            "cancel_schedule" => self.handle_cancel_schedule(request.id, request.params),
            "trigger_consolidation" => self.handle_trigger_consolidation(request.id, request.params),
            "execute_routine" => self.handle_execute_routine(request.id, request.params),
            "patch_world_state" => self.handle_patch_world_state(request.id, request.params),
            "expire_world_facts" => self.handle_expire_world_facts(request.id, request.params),
            "reload_pack" => self.handle_reload_pack(request.id, request.params),
            "unload_pack" => self.handle_unload_pack(request.id, request.params),
            "dump_world_state" => self.handle_dump_world_state(request.id, request.params),
            "set_routine_autonomous" => self.handle_set_routine_autonomous(request.id, request.params),
            "replicate_routine" => self.handle_replicate_routine(request.id, request.params),
            "author_routine" => self.handle_author_routine(request.id, request.params),
            "list_routine_versions" => self.handle_list_routine_versions(request.id, request.params),
            "rollback_routine" => self.handle_rollback_routine(request.id, request.params),
            "sync_beliefs" => self.handle_sync_beliefs(request.id, request.params),
            "migrate_session" => self.handle_migrate_session(request.id, request.params),
            "review_routine" => self.handle_review_routine(request.id, request.params),
            "handoff_session" => self.handle_handoff_session(request.id, request.params),
            "claim_session" => self.handle_claim_session(request.id, request.params),

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

        // Use a dummy id for the inner handler; we re-wrap the result with the
        // real id in MCP content format below.
        let inner_id = serde_json::json!(0);
        let inner = match tool_name {
            "create_goal" => self.handle_create_goal(inner_id, arguments),
            "create_goal_async" => self.handle_create_goal_async(inner_id, arguments),
            "get_goal_status" => self.handle_get_goal_status(inner_id, arguments),
            "stream_goal_observations" => self.handle_stream_goal_observations(inner_id, arguments),
            "cancel_goal" => self.handle_cancel_goal(inner_id, arguments),
            "inspect_session" => self.handle_inspect_session(inner_id, arguments),
            "inspect_belief" => self.handle_inspect_belief(inner_id, arguments),
            "inspect_belief_projection" => self.handle_inspect_belief_projection(inner_id, arguments),
            "provide_session_input" => self.handle_provide_session_input(inner_id, arguments),
            "inject_plan" => self.handle_inject_plan(inner_id, arguments),
            "find_routines" => self.handle_find_routines(inner_id, arguments),
            "inspect_resources" => self.handle_inspect_resources(inner_id, arguments),
            "inspect_packs" => self.handle_inspect_packs(inner_id, arguments),
            "inspect_skills" => self.handle_inspect_skills(inner_id, arguments),
            "inspect_trace" => self.handle_inspect_trace(inner_id, arguments),
            "pause_session" => self.handle_pause_session(inner_id, arguments),
            "resume_session" => self.handle_resume_session(inner_id, arguments),
            "abort_session" => self.handle_abort_session(inner_id, arguments),
            "list_sessions" => self.handle_list_sessions(inner_id, arguments),
            "query_metrics" => self.handle_query_metrics(inner_id, arguments),
            "query_policy" => self.handle_query_policy(inner_id, arguments),
            "dump_state" => self.handle_dump_state(inner_id, arguments),
            "invoke_port" => self.handle_invoke_port(inner_id, arguments),
            "list_ports" => self.handle_list_ports(inner_id, arguments),
            "list_capabilities" => self.handle_list_capabilities(inner_id, arguments),
            "list_peers" => self.handle_list_peers(inner_id, arguments),
            "invoke_remote_skill" => self.handle_invoke_remote_skill(inner_id, arguments),
            "transfer_routine" => self.handle_transfer_routine(inner_id, arguments),
            "schedule" => self.handle_schedule(inner_id, arguments),
            "list_schedules" => self.handle_list_schedules(inner_id, arguments),
            "cancel_schedule" => self.handle_cancel_schedule(inner_id, arguments),
            "trigger_consolidation" => self.handle_trigger_consolidation(inner_id, arguments),
            "execute_routine" => self.handle_execute_routine(inner_id, arguments),
            "patch_world_state" => self.handle_patch_world_state(inner_id, arguments),
            "expire_world_facts" => self.handle_expire_world_facts(inner_id, arguments),
            "reload_pack" => self.handle_reload_pack(inner_id, arguments),
            "unload_pack" => self.handle_unload_pack(inner_id, arguments),
            "dump_world_state" => self.handle_dump_world_state(inner_id, arguments),
            "set_routine_autonomous" => self.handle_set_routine_autonomous(inner_id, arguments),
            "replicate_routine" => self.handle_replicate_routine(inner_id, arguments),
            "author_routine" => self.handle_author_routine(inner_id, arguments),
            "list_routine_versions" => self.handle_list_routine_versions(inner_id, arguments),
            "rollback_routine" => self.handle_rollback_routine(inner_id, arguments),
            "sync_beliefs" => self.handle_sync_beliefs(inner_id, arguments),
            "migrate_session" => self.handle_migrate_session(inner_id, arguments),
            "review_routine" => self.handle_review_routine(inner_id, arguments),
            "handoff_session" => self.handle_handoff_session(inner_id, arguments),
            "claim_session" => self.handle_claim_session(inner_id, arguments),
            _ => {
                return Ok(Self::error_response(
                    id,
                    METHOD_NOT_FOUND,
                    format!("unknown tool: {}", tool_name),
                    None,
                ));
            }
        }?;

        // Wrap in MCP content array format for tools/call responses.
        if let Some(result) = inner.result {
            Ok(Self::tool_success_response(id, result))
        } else if let Some(err) = inner.error {
            Ok(Self::error_response(id, err.code, err.message, err.data))
        } else {
            Ok(Self::tool_success_response(id, Value::Null))
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

        let max_steps_override = params
            .get("max_steps")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok());

        let rt = match self.runtime.get() {
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

        if let Some(ms) = max_steps_override {
            goal.max_steps = Some(ms);
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
        let _ = &objective;

        // Run the control loop until it reaches a non-Continue state, then
        // finalize (store episode + attempt learning) for every terminal
        // outcome including errors. Mid-run checkpoints (when enabled via
        // `runtime.checkpoint_every_n_steps`) let interrupted sessions be
        // resumed on a later boot.
        let checkpoint_store_ref = rt.checkpoint_store.as_ref();
        let (final_status, result_data) =
            match crate::runtime::goal_executor::run_loop_with_checkpoint(
                &mut ctrl,
                &mut session,
                Some(checkpoint_store_ref),
                rt.checkpoint_every_n_steps,
            ) {
                Ok(StepResult::Completed) => {
                    let data = if let Some(last_step) = session.trace.steps.last() {
                        serde_json::json!({
                            "steps": session.trace.steps.len(),
                            "last_skill": last_step.selected_skill,
                            "last_selection_reason": last_step.selection_reason,
                        })
                    } else {
                        serde_json::Value::Null
                    };
                    ("completed".to_string(), data)
                }
                Ok(StepResult::Failed(reason)) => (
                    "failed".to_string(),
                    serde_json::json!({ "reason": reason }),
                ),
                Ok(StepResult::Aborted) => ("aborted".to_string(), serde_json::Value::Null),
                Ok(StepResult::WaitingForInput(msg)) => (
                    "waiting_for_input".to_string(),
                    serde_json::json!({ "waiting_for": msg }),
                ),
                Ok(StepResult::WaitingForRemote(msg)) => (
                    "waiting_for_remote".to_string(),
                    serde_json::json!({ "waiting_for": msg }),
                ),
                Ok(StepResult::Continue) => {
                    // run_loop never returns Continue; treat as error for safety.
                    (
                        "error".to_string(),
                        serde_json::json!({ "error": "run_loop returned Continue" }),
                    )
                }
                Err(e) => (
                    "error".to_string(),
                    serde_json::json!({ "error": e.to_string() }),
                ),
            };

        let is_terminal = matches!(
            final_status.as_str(),
            "completed" | "failed" | "aborted" | "error"
        );
        if is_terminal {
            let ctx = crate::runtime::goal_executor::EpisodeContext {
                episode_store: &rt.episode_store,
                schema_store: &rt.schema_store,
                routine_store: &rt.routine_store,
                embedder: &rt.embedder,
                world_state: &rt.world_state,
                skill_stats: Some(&rt.skill_stats),
            };
            crate::runtime::goal_executor::finalize_episode(&session, &ctx);
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

    fn handle_create_goal_async(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
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
        let max_steps_override = params
            .get("max_steps")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok());

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                let goal_id = Uuid::new_v4();
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "goal_id": goal_id.to_string(),
                        "status": "pending",
                        "objective": objective,
                    }),
                ));
            }
        };

        let input = GoalInput::NaturalLanguage {
            text: objective.clone(),
            source: GoalSource {
                source_type: crate::types::goal::GoalSourceType::Mcp,
                identity: None,
                session_id: None,
                peer_id: None,
            },
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
        if let Some(ms) = max_steps_override {
            goal.max_steps = Some(ms);
        }
        if let Some(inputs) = params.get("inputs").cloned()
            && inputs.is_object()
        {
            goal.objective.structured = Some(inputs);
        }
        if let Some(ms) = params
            .get("latency_budget_ms")
            .and_then(|v| v.as_u64())
        {
            goal.latency_budget_ms = ms;
        }
        if let Some(expl) = params.get("exploration") {
            match serde_json::from_value::<crate::types::goal::ExplorationStrategy>(
                expl.clone(),
            ) {
                Ok(s) => goal.exploration = s,
                Err(e) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("invalid exploration strategy: {e}"),
                        None,
                    ));
                }
            }
        }
        let goal_id = goal.goal_id;

        // Create the session inside a short critical section, then release
        // the controller lock so the background thread can re-acquire it
        // per step.
        let session = {
            let mut ctrl = rt.session_controller.lock().unwrap();
            match ctrl.create_session(goal) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("session creation failed: {e}"),
                        None,
                    ));
                }
            }
        };
        let session_id = session.session_id;

        let entry = std::sync::Arc::new(
            crate::runtime::goal_registry::AsyncGoalEntry::new(goal_id, session),
        );
        rt.goal_registry.insert(std::sync::Arc::clone(&entry));

        let ctx = crate::runtime::goal_registry::OwnedEpisodeContext {
            episode_store: std::sync::Arc::clone(&rt.episode_store),
            schema_store: std::sync::Arc::clone(&rt.schema_store),
            routine_store: std::sync::Arc::clone(&rt.routine_store),
            embedder: std::sync::Arc::clone(&rt.embedder),
            world_state: std::sync::Arc::clone(&rt.world_state),
            skill_stats: Some(std::sync::Arc::clone(&rt.skill_stats)),
        };

        crate::runtime::goal_registry::spawn_async_goal(
            std::sync::Arc::clone(&entry),
            std::sync::Arc::clone(&rt.session_controller),
            std::sync::Arc::clone(&rt.checkpoint_store),
            rt.checkpoint_every_n_steps,
            ctx,
        );

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "goal_id": goal_id.to_string(),
                "session_id": session_id.to_string(),
                "status": "pending",
                "objective": objective,
            }),
        ))
    }

    fn handle_get_goal_status(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let goal_id = match Self::extract_uuid(&params, "goal_id") {
            Some(v) => v,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "goal_id required (uuid string)".to_string(),
                    None,
                ));
            }
        };
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({ "goal_id": goal_id.to_string(), "status": "unknown" }),
                ));
            }
        };
        let entry = match rt.goal_registry.get(&goal_id) {
            Some(e) => e,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("no async goal with id {}", goal_id),
                    None,
                ));
            }
        };
        let status = entry.current_status();
        let error = entry.last_error();
        let (session_id, step_count, last_skill) = {
            let s = entry.session.lock().unwrap();
            (
                s.session_id.to_string(),
                s.trace.steps.len(),
                s.trace.steps.last().map(|ts| ts.selected_skill.clone()),
            )
        };
        Ok(Self::success_response(
            id,
            serde_json::json!({
                "goal_id": goal_id.to_string(),
                "session_id": session_id,
                "status": status,
                "steps": step_count,
                "last_skill": last_skill,
                "error": error,
            }),
        ))
    }

    /// Pull observations produced by an async goal since `after_step`.
    ///
    /// Each event mirrors a TraceStep but only the brain-relevant fields:
    /// step_index, selected_skill, selection_reason, success, latency_ms,
    /// observation summary, critic_decision. The brain polls this with the
    /// last seen step_index to get a near-real-time view of long-running
    /// async goals — proprioception over what would otherwise be a
    /// poll-the-final-status black box.
    fn handle_stream_goal_observations(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let goal_id = match Self::extract_uuid(&params, "goal_id") {
            Some(v) => v,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "goal_id required (uuid string)".to_string(),
                    None,
                ));
            }
        };
        let after_step = params
            .as_ref()
            .and_then(|p| p.get("after_step"))
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        let limit = params
            .as_ref()
            .and_then(|p| p.get("limit"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(100);

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "goal_id": goal_id.to_string(),
                        "events": [],
                        "terminal": true
                    }),
                ));
            }
        };
        let entry = match rt.goal_registry.get(&goal_id) {
            Some(e) => e,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("no async goal with id {}", goal_id),
                    None,
                ));
            }
        };

        let status = entry.current_status();
        let terminal = matches!(
            status,
            crate::runtime::goal_registry::AsyncGoalStatus::Completed
                | crate::runtime::goal_registry::AsyncGoalStatus::Failed
                | crate::runtime::goal_registry::AsyncGoalStatus::Aborted
                | crate::runtime::goal_registry::AsyncGoalStatus::Error
        );

        let session = entry.session.lock().unwrap();
        let events: Vec<Value> = session
            .trace
            .steps
            .iter()
            .filter(|s| (s.step_index as i64) > after_step)
            .take(limit)
            .map(|s| {
                serde_json::json!({
                    "step_index": s.step_index,
                    "selected_skill": s.selected_skill,
                    "selection_reason": s.selection_reason,
                    "candidate_skills": s.candidate_skills,
                    "predicted_scores": s.predicted_scores.iter().map(|c| {
                        serde_json::json!({"skill_id": c.skill_id, "score": c.score})
                    }).collect::<Vec<_>>(),
                    "critic_decision": s.critic_decision,
                    "progress_delta": s.progress_delta,
                    "termination_reason": s.termination_reason.as_ref().map(|t| format!("{:?}", t)),
                    "rollback_invoked": s.rollback_invoked,
                    "observation_success": s.port_calls.iter().all(|p| p.success),
                    "failure_detail": s.failure_detail,
                    "timestamp": s.timestamp.to_rfc3339(),
                    "port_calls": s.port_calls.iter().map(|p| {
                        serde_json::json!({
                            "port_id": p.port_id,
                            "capability_id": p.capability_id,
                            "success": p.success,
                            "latency_ms": p.latency_ms
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();

        let last_step = session
            .trace
            .steps
            .last()
            .map(|s| s.step_index as i64)
            .unwrap_or(-1);

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "goal_id": goal_id.to_string(),
                "status": status,
                "events": events,
                "last_step": last_step,
                "terminal": terminal,
            }),
        ))
    }

    fn handle_cancel_goal(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let goal_id = match Self::extract_uuid(&params, "goal_id") {
            Some(v) => v,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "goal_id required (uuid string)".to_string(),
                    None,
                ));
            }
        };
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({ "goal_id": goal_id.to_string(), "cancelled": true }),
                ));
            }
        };
        match rt.goal_registry.get(&goal_id) {
            Some(entry) => {
                entry.request_cancel();
                Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "goal_id": goal_id.to_string(),
                        "cancel_requested": true,
                    }),
                ))
            }
            None => Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                format!("no async goal with id {}", goal_id),
                None,
            )),
        }
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

        if let Some(rt) = self.runtime.get() {
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

            // Check async goal entries first (their session is the live copy).
            let async_entry = rt
                .goal_registry
                .list()
                .into_iter()
                .filter_map(|gid| rt.goal_registry.get(&gid))
                .find(|e| e.session_id == uuid);

            let build_response = |session: &crate::types::session::ControlSession| {
                serde_json::json!({
                    "session_id": session.session_id.to_string(),
                    "status": format!("{:?}", session.status),
                    "objective": session.goal.objective.description,
                    "working_memory": {
                        "active_bindings": session.working_memory.active_bindings.len(),
                        "unresolved_slots": &session.working_memory.unresolved_slots,
                        "current_subgoal": &session.working_memory.current_subgoal,
                        "candidate_shortlist": &session.working_memory.candidate_shortlist,
                        "plan_step": session.working_memory.plan_step,
                        "has_active_steps": session.working_memory.active_steps.is_some(),
                        "active_steps_len": session.working_memory.active_steps.as_ref().map(|s| s.len()).unwrap_or(0),
                        "has_active_plan": session.working_memory.active_plan.is_some(),
                        "plan_stack_depth": session.working_memory.plan_stack.len(),
                        "used_plan_following": session.working_memory.used_plan_following,
                        "pending_input_request": session.working_memory.pending_input_request.as_ref().map(|r| {
                            serde_json::json!({
                                "skill_id": r.skill_id,
                                "missing_slots": r.missing_slots.iter().map(|s| {
                                    serde_json::json!({"name": s.name, "schema": s.schema})
                                }).collect::<Vec<_>>()
                            })
                        }),
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
                })
            };

            if let Some(entry) = async_entry.as_ref() {
                let session = entry.session.lock().unwrap();
                return Ok(Self::success_response(id, build_response(&session)));
            }

            let ctrl = rt.session_controller.lock().unwrap();
            match ctrl.get_session(&uuid) {
                Some(session) => Ok(Self::success_response(id, build_response(session))),
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

        if let Some(rt) = self.runtime.get() {
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

    fn handle_inspect_belief_projection(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
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

        if let Some(rt) = self.runtime.get() {
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
            match ctrl.project_belief(&uuid) {
                Some((full_belief, projected, toon_encoded)) => {
                    let full_size = serde_json::to_string(&full_belief)
                        .map(|s| s.len())
                        .unwrap_or(0);
                    let projected_size = serde_json::to_string(&projected)
                        .map(|s| s.len())
                        .unwrap_or(0);
                    let toon_size = toon_encoded.len();

                    Ok(Self::success_response(
                        id,
                        serde_json::json!({
                            "session_id": session_id_str,
                            "full_belief": full_belief,
                            "projected": projected,
                            "toon_encoded": toon_encoded,
                            "size_comparison": {
                                "full_json_bytes": full_size,
                                "projected_json_bytes": projected_size,
                                "toon_bytes": toon_size,
                                "projection_reduction_pct": if full_size > 0 {
                                    ((1.0 - projected_size as f64 / full_size as f64) * 100.0).round()
                                } else { 0.0 },
                                "toon_reduction_pct": if full_size > 0 {
                                    ((1.0 - toon_size as f64 / full_size as f64) * 100.0).round()
                                } else { 0.0 },
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
            Ok(Self::error_response(
                id,
                INTERNAL_ERROR,
                "runtime not initialized".to_string(),
                None,
            ))
        }
    }

    fn handle_provide_session_input(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
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
        let bindings = match params.as_ref().and_then(|p| p.get("bindings")) {
            Some(b) if b.is_object() => b.clone(),
            _ => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "bindings required (JSON object of name → value)".to_string(),
                    None,
                ));
            }
        };
        let redirect_skill_id = params
            .as_ref()
            .and_then(|p| p.get("redirect_skill_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if let Some(rt) = self.runtime.get() {
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

            let async_entry = rt
                .goal_registry
                .list()
                .into_iter()
                .filter_map(|gid| rt.goal_registry.get(&gid))
                .find(|e| e.session_id == uuid);

            let injected_count;
            if let Some(entry) = async_entry.as_ref() {
                let mut session = entry.session.lock().unwrap();
                if session.status != crate::types::session::SessionStatus::WaitingForInput {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!(
                            "session is {:?}, not WaitingForInput",
                            session.status
                        ),
                        None,
                    ));
                }
                injected_count = match crate::runtime::session::SessionController::inject_brain_input(
                    &mut session,
                    &bindings,
                ) {
                    Ok(n) => n,
                    Err(e) => {
                        return Ok(Self::error_response(
                            id,
                            INVALID_PARAMS,
                            format!("invalid bindings: {e}"),
                            None,
                        ));
                    }
                };
                if let Some(ref skill_id) = redirect_skill_id {
                    session.working_memory.active_steps = Some(vec![
                        crate::types::routine::CompiledStep::Skill {
                            skill_id: skill_id.clone(),
                            on_success: crate::types::routine::NextStep::Continue,
                            on_failure: crate::types::routine::NextStep::Continue,
                            conditions: vec![],
                        },
                    ]);
                    session.working_memory.plan_step = 0;
                    session.working_memory.active_plan = None;
                    session.working_memory.used_plan_following = true;
                }
                let mut ctrl = rt.session_controller.lock().unwrap();
                if let Err(e) = ctrl.resume(&mut session) {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("resume failed: {e}"),
                        None,
                    ));
                }
                drop(session);
                drop(ctrl);

                let ctx = crate::runtime::goal_registry::OwnedEpisodeContext {
                    episode_store: Arc::clone(&rt.episode_store),
                    schema_store: Arc::clone(&rt.schema_store),
                    routine_store: Arc::clone(&rt.routine_store),
                    embedder: Arc::clone(&rt.embedder),
                    world_state: Arc::clone(&rt.world_state),
                    skill_stats: Some(Arc::clone(&rt.skill_stats)),
                };
                crate::runtime::goal_registry::spawn_async_goal(
                    Arc::clone(entry),
                    Arc::clone(&rt.session_controller),
                    Arc::clone(&rt.checkpoint_store),
                    rt.checkpoint_every_n_steps,
                    ctx,
                );
            } else {
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
                if session.status != crate::types::session::SessionStatus::WaitingForInput {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!(
                            "session is {:?}, not WaitingForInput",
                            session.status
                        ),
                        None,
                    ));
                }
                injected_count = match crate::runtime::session::SessionController::inject_brain_input(
                    &mut session,
                    &bindings,
                ) {
                    Ok(n) => n,
                    Err(e) => {
                        return Ok(Self::error_response(
                            id,
                            INVALID_PARAMS,
                            format!("invalid bindings: {e}"),
                            None,
                        ));
                    }
                };
                if let Some(ref skill_id) = redirect_skill_id {
                    session.working_memory.active_steps = Some(vec![
                        crate::types::routine::CompiledStep::Skill {
                            skill_id: skill_id.clone(),
                            on_success: crate::types::routine::NextStep::Continue,
                            on_failure: crate::types::routine::NextStep::Continue,
                            conditions: vec![],
                        },
                    ]);
                    session.working_memory.plan_step = 0;
                    session.working_memory.active_plan = None;
                    session.working_memory.used_plan_following = true;
                }
                if let Err(e) = ctrl.resume(&mut session) {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("resume failed: {e}"),
                        None,
                    ));
                }
            }

            let mut resp = serde_json::json!({
                "session_id": session_id_str,
                "bindings_injected": injected_count,
                "status": "resumed",
            });
            if let Some(ref skill_id) = redirect_skill_id {
                resp["redirected_to"] = serde_json::json!(skill_id);
            }
            Ok(Self::success_response(id, resp))
        } else {
            Ok(Self::error_response(
                id,
                INTERNAL_ERROR,
                "runtime not initialized".to_string(),
                None,
            ))
        }
    }

    fn handle_inject_plan(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
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
        let steps_val = match params.as_ref().and_then(|p| p.get("steps")) {
            Some(s) if s.is_array() => s.clone(),
            _ => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "steps required (array of CompiledStep)".to_string(),
                    None,
                ));
            }
        };
        let steps: Vec<crate::types::routine::CompiledStep> = match serde_json::from_value(steps_val) {
            Ok(s) => s,
            Err(e) => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("invalid steps: {e}"),
                    None,
                ));
            }
        };
        if steps.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "steps must not be empty".to_string(),
                None,
            ));
        }

        if let Some(rt) = self.runtime.get() {
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

            let step_count = steps.len();

            let async_entry = rt
                .goal_registry
                .list()
                .into_iter()
                .filter_map(|gid| rt.goal_registry.get(&gid))
                .find(|e| e.session_id == uuid);

            if let Some(entry) = async_entry.as_ref() {
                let mut session = entry.session.lock().unwrap();
                session.working_memory.active_steps = Some(steps);
                session.working_memory.plan_step = 0;
                session.working_memory.active_plan = None;
                session.working_memory.used_plan_following = true;
            } else {
                let mut ctrl = rt.session_controller.lock().unwrap();
                let session = match ctrl.get_session_by_id_mut(&uuid) {
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
                session.working_memory.active_steps = Some(steps);
                session.working_memory.plan_step = 0;
                session.working_memory.active_plan = None;
                session.working_memory.used_plan_following = true;
            }

            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "steps_injected": step_count,
                }),
            ))
        } else {
            Ok(Self::error_response(
                id,
                INTERNAL_ERROR,
                "runtime not initialized".to_string(),
                None,
            ))
        }
    }

    fn handle_find_routines(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let query = params
            .as_ref()
            .and_then(|p| p.get("query"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let limit = params
            .as_ref()
            .and_then(|p| p.get("limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        if let Some(rt) = self.runtime.get() {
            let routine_store = rt.routine_store.lock().unwrap();
            let all_routines = routine_store.list_all();

            let mut results: Vec<serde_json::Value> = Vec::new();

            if query.is_empty() {
                for r in all_routines.iter().take(limit) {
                    results.push(Self::routine_summary(r));
                }
            } else {
                use crate::memory::embedder::GoalEmbedder;
                let embedder = crate::memory::embedder::HashEmbedder::new();
                let query_emb = embedder.embed(query);
                let mut scored: Vec<(f64, &crate::types::routine::Routine)> = all_routines
                    .iter()
                    .map(|&r| {
                        let goal_fp = r.match_conditions.iter()
                            .find(|c| c.condition_type == "goal_fingerprint")
                            .and_then(|c| c.expression.get("goal_fingerprint"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let r_text = format!(
                            "{} {}",
                            goal_fp,
                            r.compiled_skill_path.join(" ")
                        );
                        let r_emb = embedder.embed(&r_text);
                        let sim = embedder.similarity(&query_emb, &r_emb);
                        (sim, r)
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                for (score, r) in scored.into_iter().take(limit) {
                    let mut summary = Self::routine_summary(r);
                    summary["similarity"] = serde_json::json!(score);
                    results.push(summary);
                }
            }

            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "routines": results,
                    "total": all_routines.len(),
                }),
            ))
        } else {
            Ok(Self::error_response(
                id,
                INTERNAL_ERROR,
                "runtime not initialized".to_string(),
                None,
            ))
        }
    }

    fn routine_summary(r: &crate::types::routine::Routine) -> serde_json::Value {
        let goal_fp = r.match_conditions.iter()
            .find(|c| c.condition_type == "goal_fingerprint")
            .and_then(|c| c.expression.get("goal_fingerprint"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        serde_json::json!({
            "routine_id": r.routine_id,
            "goal_fingerprint": goal_fp,
            "skill_path": r.compiled_skill_path,
            "steps": r.effective_steps().len(),
            "confidence": r.confidence,
            "version": r.version,
        })
    }

    fn handle_inspect_resources(&self, id: Value, _params: Option<Value>) -> Result<McpResponse> {
        // Resources are tracked inside belief state per-session. The global
        // resource listing comes from registered port specs which declare what
        // external resources are available to the runtime.
        if let Some(rt) = self.runtime.get() {
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
        if let Some(rt) = self.runtime.get() {
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
        if let Some(rt) = self.runtime.get() {
            let pack_filter = params
                .as_ref()
                .and_then(|p| p.get("pack"))
                .and_then(|v| v.as_str());

            let skill_rt = rt.skill_runtime.lock().unwrap();
            let all_skills = skill_rt.list_skills(pack_filter);
            let skills: Vec<Value> = all_skills
                .iter()
                .map(|s| {
                    let stats = rt.skill_stats.get(&s.skill_id);
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
                        "capability_requirements": s.capability_requirements,
                        "tags": s.tags,
                        "observed": stats.map(|st| serde_json::json!({
                            "n": st.n_observed,
                            "ema_latency_ms": st.ema_latency_ms,
                            "ema_resource_cost": st.ema_resource_cost,
                            "ema_success_rate": st.ema_success_rate,
                            "calibrated": st.is_calibrated(),
                        })),
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

        if let Some(rt) = self.runtime.get() {
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
                                "selection_reason": step.selection_reason,
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
                                "failure_detail": step.failure_detail,
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

        if let Some(rt) = self.runtime.get() {
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
        let payload = params.as_ref().and_then(|p| p.get("payload")).cloned();

        if let Some(rt) = self.runtime.get() {
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

            // Async-goal sessions live inside an `AsyncGoalEntry` whose
            // own Mutex<ControlSession> is the one we need to mutate.
            let async_entry = rt
                .goal_registry
                .list()
                .into_iter()
                .filter_map(|gid| rt.goal_registry.get(&gid))
                .find(|e| e.session_id == uuid);

            let injected_count;
            let new_status_str;
            if let Some(entry) = async_entry.as_ref() {
                let mut session = entry.session.lock().unwrap();
                injected_count = if let Some(ref p) = payload {
                    match crate::runtime::session::SessionController::inject_resume_payload(
                        &mut session,
                        p,
                    ) {
                        Ok(n) => n,
                        Err(e) => {
                            return Ok(Self::error_response(
                                id,
                                INVALID_PARAMS,
                                format!("invalid payload: {e}"),
                                None,
                            ));
                        }
                    }
                } else {
                    0
                };
                let mut ctrl = rt.session_controller.lock().unwrap();
                if let Err(e) = ctrl.resume(&mut session) {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("resume failed: {e}"),
                        None,
                    ));
                }
                drop(session);
                drop(ctrl);

                // The background thread for the async goal exited when the
                // session entered WaitingFor*; respawn it now that the
                // payload has been delivered.
                let ctx = crate::runtime::goal_registry::OwnedEpisodeContext {
                    episode_store: Arc::clone(&rt.episode_store),
                    schema_store: Arc::clone(&rt.schema_store),
                    routine_store: Arc::clone(&rt.routine_store),
                    embedder: Arc::clone(&rt.embedder),
                    world_state: Arc::clone(&rt.world_state),
                    skill_stats: Some(Arc::clone(&rt.skill_stats)),
                };
                crate::runtime::goal_registry::spawn_async_goal(
                    Arc::clone(entry),
                    Arc::clone(&rt.session_controller),
                    Arc::clone(&rt.checkpoint_store),
                    rt.checkpoint_every_n_steps,
                    ctx,
                );
                new_status_str = "Running".to_string();
            } else {
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
                injected_count = if let Some(ref p) = payload {
                    match crate::runtime::session::SessionController::inject_resume_payload(
                        &mut session,
                        p,
                    ) {
                        Ok(n) => n,
                        Err(e) => {
                            return Ok(Self::error_response(
                                id,
                                INVALID_PARAMS,
                                format!("invalid payload: {e}"),
                                None,
                            ));
                        }
                    }
                } else {
                    0
                };
                if let Err(e) = ctrl.resume(&mut session) {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("resume failed: {e}"),
                        None,
                    ));
                }
                new_status_str = format!("{:?}", session.status);
            }

            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "session_id": session_id_str,
                    "status": new_status_str,
                    "injected_bindings": injected_count,
                }),
            ))
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

        if let Some(rt) = self.runtime.get() {
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
        if let Some(rt) = self.runtime.get() {
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
        match self.runtime.get() {
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

        if let Some(rt) = self.runtime.get() {
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
                                "plan_step": session.working_memory.plan_step,
                                "has_active_steps": session.working_memory.active_steps.is_some(),
                                "active_steps_len": session.working_memory.active_steps.as_ref().map(|s| s.len()).unwrap_or(0),
                                "has_active_plan": session.working_memory.active_plan.is_some(),
                                "plan_stack_depth": session.working_memory.plan_stack.len(),
                                "used_plan_following": session.working_memory.used_plan_following,
                            },
                            "trace": session.trace.steps.iter().map(|step| {
                                serde_json::json!({
                                    "step_index": step.step_index,
                                    "selected_skill": step.selected_skill,
                                    "selection_reason": step.selection_reason,
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
    // Port invocation and discovery
    // -----------------------------------------------------------------------

    fn handle_invoke_port(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: port_id, capability_id, input".to_string(),
                    None,
                ));
            }
        };

        let port_id = match params.get("port_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "port_id must be a non-empty string".to_string(),
                    None,
                ));
            }
        };

        let capability_id = match params.get("capability_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "capability_id must be a non-empty string".to_string(),
                    None,
                ));
            }
        };

        let input = params
            .get("input")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        if let Some(rt) = self.runtime.get() {
            let ctx = crate::types::port::InvocationContext {
                caller_identity: Some("mcp".to_string()),
                ..Default::default()
            };

            let port_rt = rt.port_runtime.lock().unwrap();
            match port_rt.invoke(&port_id, &capability_id, input, &ctx) {
                Ok(record) => {
                    // Track this call in the implicit session for episode
                    // creation. Clone what we need before serializing.
                    let record_clone = record.clone();
                    let record_json = serde_json::to_value(&record).unwrap_or_default();

                    self.record_implicit_call(
                        port_id,
                        capability_id,
                        record_clone,
                    );

                    Ok(Self::success_response(id, record_json))
                }
                Err(e) => Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("port invocation failed: {e}"),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "port_id": port_id,
                    "capability_id": capability_id,
                    "success": false,
                    "raw_result": null,
                    "structured_result": null,
                    "failure_class": "DependencyUnavailable",
                    "latency_ms": 0
                }),
            ))
        }
    }

    fn handle_list_ports(&self, id: Value, params: Option<Value>) -> Result<McpResponse> {
        let namespace = params
            .as_ref()
            .and_then(|p| p.get("namespace"))
            .and_then(|v| v.as_str())
            .map(String::from);

        if let Some(rt) = self.runtime.get() {
            let port_rt = rt.port_runtime.lock().unwrap();
            let ports = port_rt.list_ports(namespace.as_deref());
            let port_json: Vec<Value> = ports
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
                                "effect_class": format!("{:?}", c.effect_class),
                                "risk_class": format!("{:?}", c.risk_class),
                                "input_schema": c.input_schema,
                                "output_schema": c.output_schema,
                            })
                        }).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(Self::success_response(id, serde_json::json!({ "ports": port_json })))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({ "ports": [] }),
            ))
        }
    }

    /// Capability catalog: what the brain is allowed to invoke right now.
    ///
    /// Inputs (all optional):
    ///   - `goal_id` / `session_id`: scope by an active goal/session so the
    ///     answer reflects its `permissions_scope` and remaining budget.
    ///
    /// Output groups skills + ports into `allowed` and `denied` based on the
    /// goal's `permissions_scope`. When no goal is supplied, every loaded
    /// skill/port is reported as allowed and budget is null.
    fn handle_list_capabilities(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let Some(rt) = self.runtime.get() else {
            return Ok(Self::success_response(
                id,
                serde_json::json!({
                    "allowed": {"skills": [], "ports": []},
                    "denied": {"skills": [], "ports": []},
                    "remaining_budget": null,
                    "permissions_scope": []
                }),
            ));
        };

        let goal_id = params
            .as_ref()
            .and_then(|p| p.get("goal_id"))
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        let session_id = params
            .as_ref()
            .and_then(|p| p.get("session_id"))
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());

        let mut scope: Vec<String> = Vec::new();
        let mut budget: Option<Value> = None;

        if let Some(sid) = session_id {
            let ctrl = rt.session_controller.lock().unwrap();
            if let Some(s) = ctrl.get_session(&sid) {
                scope = s.goal.permissions_scope.clone();
                budget = Some(serde_json::json!({
                    "risk_remaining": s.budget_remaining.risk_remaining,
                    "latency_remaining_ms": s.budget_remaining.latency_remaining_ms,
                    "resource_remaining": s.budget_remaining.resource_remaining,
                    "steps_remaining": s.budget_remaining.steps_remaining
                }));
            }
        } else if let Some(gid) = goal_id
            && let Some(entry) = rt.goal_registry.get(&gid)
        {
            let s = entry.session.lock().unwrap();
            scope = s.goal.permissions_scope.clone();
            budget = Some(serde_json::json!({
                "risk_remaining": s.budget_remaining.risk_remaining,
                "latency_remaining_ms": s.budget_remaining.latency_remaining_ms,
                "resource_remaining": s.budget_remaining.resource_remaining,
                "steps_remaining": s.budget_remaining.steps_remaining
            }));
        }

        // A capability is allowed when the goal's permissions_scope is empty
        // (== unrestricted) or contains the skill/port pack-or-namespace tag.
        let scope_allows = |tags: &[&str]| -> bool {
            if scope.is_empty() {
                return true;
            }
            tags.iter().any(|t| scope.iter().any(|s| s == t))
        };

        let mut allowed_skills: Vec<Value> = Vec::new();
        let mut denied_skills: Vec<Value> = Vec::new();
        {
            let skill_rt = rt.skill_runtime.lock().unwrap();
            for s in skill_rt.list_skills(None) {
                let entry = serde_json::json!({
                    "skill_id": s.skill_id,
                    "pack": s.pack,
                    "namespace": s.namespace,
                    "risk_class": format!("{:?}", s.risk_class),
                    "capability_requirements": s.capability_requirements,
                });
                if scope_allows(&[s.pack.as_str(), s.namespace.as_str()]) {
                    allowed_skills.push(entry);
                } else {
                    denied_skills.push(entry);
                }
            }
        }

        let mut allowed_ports: Vec<Value> = Vec::new();
        let mut denied_ports: Vec<Value> = Vec::new();
        {
            let port_rt = rt.port_runtime.lock().unwrap();
            for p in port_rt.list_ports(None) {
                let entry = serde_json::json!({
                    "port_id": p.port_id,
                    "namespace": p.namespace,
                    "kind": format!("{:?}", p.kind),
                    "capabilities": p.capabilities.iter()
                        .map(|c| c.capability_id.clone())
                        .collect::<Vec<_>>(),
                });
                if scope_allows(&[p.port_id.as_str(), p.namespace.as_str()]) {
                    allowed_ports.push(entry);
                } else {
                    denied_ports.push(entry);
                }
            }
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "allowed": {"skills": allowed_skills, "ports": allowed_ports},
                "denied": {"skills": denied_skills, "ports": denied_ports},
                "remaining_budget": budget,
                "permissions_scope": scope
            }),
        ))
    }

    // -----------------------------------------------------------------------
    // Distributed tool handlers
    // -----------------------------------------------------------------------

    fn handle_list_peers(&self, id: Value, _params: Option<Value>) -> Result<McpResponse> {
        if let Some(rt) = self.runtime.get() {
            let ids = rt.peer_ids.lock().unwrap();
            let peers: Vec<Value> = ids
                .iter()
                .map(|pid| {
                    serde_json::json!({
                        "peer_id": pid,
                        "registered": true,
                        "has_executor": rt.remote_executor.is_some(),
                    })
                })
                .collect();
            Ok(Self::success_response(
                id,
                serde_json::json!({ "peers": peers, "count": peers.len() }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({ "peers": [], "count": 0 }),
            ))
        }
    }

    fn handle_invoke_remote_skill(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: peer_id, skill_id".to_string(),
                    None,
                ));
            }
        };

        let peer_id = match params.get("peer_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "peer_id is required".to_string(),
                    None,
                ));
            }
        };

        let skill_id = match params.get("skill_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "skill_id is required".to_string(),
                    None,
                ));
            }
        };

        let input = params
            .get("input")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        if let Some(rt) = self.runtime.get() {
            if let Some(ref exec) = rt.remote_executor {
                match exec.invoke_skill(&peer_id, &skill_id, input) {
                    Ok(resp) => {
                        // Track the observation through the streaming subsystem
                        // for monitoring and delivery status tracking.
                        let mut stream_info = serde_json::json!(null);
                        if let Ok(mut streaming) = rt.observation_streaming.lock() {
                            let session_id = resp.trace_id;
                            if let Ok(stream_id) = streaming.open_stream(
                                session_id,
                                &peer_id,
                                false,
                            ) {
                                let obs = crate::types::peer::StreamedObservation {
                                    session_id,
                                    step_id: format!("remote-{}", skill_id),
                                    source_peer: peer_id.clone(),
                                    skill_or_resource_ref: skill_id.clone(),
                                    raw_result: resp.observation.clone(),
                                    structured_result: resp.observation.clone(),
                                    effect_patch: None,
                                    success: resp.success,
                                    latency_ms: resp.latency_ms,
                                    timestamp: resp.timestamp,
                                    sequence: 1,
                                };
                                let delivery = streaming
                                    .receive_observation(&stream_id, &obs)
                                    .ok();
                                let _ = streaming.close_stream(&stream_id);
                                stream_info = serde_json::json!({
                                    "stream_id": stream_id.0,
                                    "delivery_status": delivery.map(|d| format!("{:?}", d)),
                                });
                            }
                        }
                        Ok(Self::success_response(
                            id,
                            serde_json::json!({
                                "skill_id": resp.skill_id,
                                "peer_id": resp.peer_id,
                                "success": resp.success,
                                "observation": resp.observation,
                                "latency_ms": resp.latency_ms,
                                "trace_id": resp.trace_id.to_string(),
                                "timestamp": resp.timestamp.to_rfc3339(),
                                "stream_info": stream_info,
                            }),
                        ))
                    }
                    Err(e) => Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("remote skill invocation failed: {}", e),
                        None,
                    )),
                }
            } else {
                Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    "no remote executor configured (start with --peer flag)".to_string(),
                    None,
                ))
            }
        } else {
            Ok(Self::error_response(
                id,
                INTERNAL_ERROR,
                "runtime not available".to_string(),
                None,
            ))
        }
    }

    fn handle_transfer_routine(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: peer_id, routine_id".to_string(),
                    None,
                ));
            }
        };

        let peer_id = match params.get("peer_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "peer_id is required".to_string(),
                    None,
                ));
            }
        };

        let routine_id = match params.get("routine_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "routine_id is required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = self.runtime.get() {
            // Look up the routine locally.
            let routine = {
                let store = rt.routine_store.lock().unwrap();
                store.list_all().into_iter().find(|r| r.routine_id == routine_id).cloned()
            };

            let routine = match routine {
                Some(r) => r,
                None => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("routine '{}' not found locally", routine_id),
                        None,
                    ));
                }
            };

            if let Some(ref exec) = rt.remote_executor {
                // Convert to transfer format.
                let transfer = crate::types::peer::RoutineTransfer {
                    routine_id: routine.routine_id.clone(),
                    match_conditions: routine.match_conditions.clone(),
                    compiled_skill_path: routine.compiled_skill_path.clone(),
                    compiled_steps: routine.compiled_steps.clone(),
                    guard_conditions: routine.guard_conditions.clone(),
                    expected_cost: routine.expected_cost,
                    expected_effect: routine.expected_effect.clone(),
                    confidence: routine.confidence,
                    autonomous: routine.autonomous,
                    priority: routine.priority,
                    exclusive: routine.exclusive,
                    policy_scope: routine.policy_scope.clone(),
                    version: routine.version,
                };

                match exec.transfer_routine(&peer_id, &transfer) {
                    Ok(()) => Ok(Self::success_response(
                        id,
                        serde_json::json!({
                            "transferred": true,
                            "routine_id": routine_id,
                            "peer_id": peer_id,
                            "compiled_skill_path": routine.compiled_skill_path,
                        }),
                    )),
                    Err(e) => Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("routine transfer failed: {}", e),
                        None,
                    )),
                }
            } else {
                Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    "no remote executor configured (start with --peer flag)".to_string(),
                    None,
                ))
            }
        } else {
            Ok(Self::error_response(
                id,
                INTERNAL_ERROR,
                "runtime not available".to_string(),
                None,
            ))
        }
    }

    fn handle_execute_routine(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: routine_id (string)".to_string(),
                    None,
                ));
            }
        };

        let routine_id = params
            .get("routine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if routine_id.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "routine_id must be a non-empty string".to_string(),
                None,
            ));
        }

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                // Stub mode: return placeholder response.
                let session_id = Uuid::new_v4();
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "session_id": session_id.to_string(),
                        "routine_id": routine_id,
                        "status": "completed",
                        "result": { "note": "stub mode" }
                    }),
                ));
            }
        };

        // Look up the routine.
        let routine = {
            let rs = rt.routine_store.lock().unwrap();
            match rs.get(&routine_id) {
                Some(r) => r.clone(),
                None => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("routine not found: {routine_id}"),
                        None,
                    ));
                }
            }
        };

        // Build a GoalSpec using the routine_id as objective.
        let objective = format!("execute routine: {routine_id}");
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

        let goal = {
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

        let goal_id = goal.goal_id;

        // Consult the routine router to decide where to execute.
        // The MCP server has a peer ID list and a remote executor but not
        // a full PeerRegistry with load/skill data. When peers are available
        // and local load exceeds the router's threshold, delegate to the
        // first known peer. Full PeerRegistry-based routing is used by the
        // distributed listener layer (--listen mode) where peer specs are
        // populated via heartbeat and mDNS.
        if let Some(ref exec) = rt.remote_executor {
            let local_load = rt.metrics.active_sessions
                .load(std::sync::atomic::Ordering::Relaxed) as f64 / 100.0;
            if local_load >= rt.routine_router.local_load_threshold() {
                let peers = rt.peer_ids.lock().unwrap();
                if let Some(peer_id) = peers.first().cloned() {
                    drop(peers);
                    // Transfer the routine first so the peer has it, then
                    // submit the goal for execution.
                    let transfer = crate::types::peer::RoutineTransfer {
                        routine_id: routine.routine_id.clone(),
                        match_conditions: routine.match_conditions.clone(),
                        compiled_skill_path: routine.compiled_skill_path.clone(),
                        compiled_steps: routine.compiled_steps.clone(),
                        guard_conditions: routine.guard_conditions.clone(),
                        expected_cost: routine.expected_cost,
                        expected_effect: routine.expected_effect.clone(),
                        confidence: routine.confidence,
                        autonomous: routine.autonomous,
                        priority: routine.priority,
                        exclusive: routine.exclusive,
                        policy_scope: routine.policy_scope.clone(),
                        version: routine.version,
                    };
                    let _ = exec.transfer_routine(&peer_id, &transfer);
                    let skill_ids: Vec<String> = routine.effective_steps().iter().filter_map(|s| {
                        if let crate::types::routine::CompiledStep::Skill { skill_id, .. } = s {
                            Some(skill_id.clone())
                        } else { None }
                    }).collect();
                    let first_skill = skill_ids.first().cloned().unwrap_or_default();
                    match exec.invoke_skill(&peer_id, &first_skill, serde_json::json!({})) {
                        Ok(resp) => {
                            return Ok(Self::success_response(
                                id,
                                serde_json::json!({
                                    "routine_id": routine_id,
                                    "routed_to": peer_id,
                                    "remote_result": resp.observation,
                                }),
                            ));
                        }
                        Err(e) => {
                            tracing::debug!("remote execution failed, falling back to local: {e}");
                        }
                    }
                }
            }
        }

        // Create a session and pre-load the routine's plan.
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

        // Inject optional input bindings into active_bindings.
        if let Some(input_obj) = params.get("input").and_then(|v| v.as_object()) {
            for (key, value) in input_obj {
                session.belief.active_bindings.push(
                    crate::types::belief::Binding {
                        name: key.clone(),
                        value: value.clone(),
                        source: "goal".to_string(),
                        confidence: 1.0,
                    },
                );
            }
        }

        // Pre-load the routine's steps as the session's active plan.
        let steps = routine.effective_steps();
        if !steps.is_empty() {
            session.working_memory.active_steps = Some(steps);
        } else {
            session.working_memory.active_plan = Some(routine.compiled_skill_path.clone());
        }
        session.working_memory.plan_step = 0;
        session.working_memory.used_plan_following = true;
        session.working_memory.active_policy_scope = routine.policy_scope.clone();

        // Run the control loop until it reaches a non-Continue state.
        let final_status;
        let mut result_data = serde_json::Value::Null;
        loop {
            match ctrl.run_step(&mut session) {
                Ok(StepResult::Continue) => continue,
                Ok(StepResult::Completed) => {
                    final_status = "completed".to_string();
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

        // Routine executions that succeed don't need episode storage —
        // the routine already captures the behavior.
        let is_terminal = matches!(
            final_status.as_str(),
            "completed" | "failed" | "aborted" | "error"
        );
        let succeeded = final_status == "completed";
        if is_terminal && !succeeded {
            let mut episode = crate::interfaces::cli::build_episode_from_session(
                &session,
                Some(&*rt.embedder),
            );
            episode.world_state_context = rt.world_state.lock().ok()
                .map(|ws| ws.snapshot())
                .unwrap_or(serde_json::json!({}));
            let fingerprint = episode.goal_fingerprint.clone();
            let adapter = crate::adapters::EpisodeMemoryAdapter::new(
                Arc::clone(&rt.episode_store),
                Arc::clone(&rt.embedder),
            );
            if let Err(e) = adapter.store(episode) {
                tracing::warn!(error = %e, "failed to store episode from execute_routine");
            } else {
                crate::interfaces::cli::attempt_learning(
                    &rt.episode_store,
                    &rt.schema_store,
                    &rt.routine_store,
                    &fingerprint,
                    &*rt.embedder,
                );
            }
        }

        // Record routine completion (or failure) as a WorldState fact so the
        // reactive monitor can chain routines or trigger follow-up logic.
        if is_terminal
            && let Ok(mut ws) = rt.world_state.lock()
        {
            let fact = crate::types::belief::Fact {
                fact_id: format!("routine_completed.{}", routine_id),
                subject: "routine".to_string(),
                predicate: format!("{}_completed", routine_id),
                value: serde_json::json!(succeeded),
                confidence: if succeeded { 1.0 } else { 0.5 },
                provenance: crate::types::common::FactProvenance::Observed,
                timestamp: chrono::Utc::now(),
                            ttl_ms: None,
            };
            let _ = ws.add_fact(fact);
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "session_id": session_id.to_string(),
                "goal_id": goal_id.to_string(),
                "routine_id": routine_id,
                "status": final_status,
                "result": result_data
            }),
        ))
    }

    fn handle_schedule(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: label (string)".to_string(),
                    None,
                ));
            }
        };

        let label = match params.get("label").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "label is required".to_string(),
                    None,
                ));
            }
        };

        // Resolve short port names (e.g. "smtp") to full port IDs
        // (e.g. "soma.smtp") so the scheduler can invoke them directly.
        let port_id = params.get("port_id").and_then(|v| v.as_str()).map(|s| {
            if let Some(rt) = self.runtime.get()
                && let Ok(pr) = rt.port_runtime.lock()
            {
                if pr.get_port(s).is_some() {
                    return s.to_string();
                }
                for p in pr.list_ports(None) {
                    if p.port_id.ends_with(s) || p.port_id.ends_with(&format!(".{}", s)) {
                        return p.port_id.clone();
                    }
                }
            }
            s.to_string()
        });
        let capability_id = params.get("capability_id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let input = params.get("input").cloned().unwrap_or(serde_json::json!({}));
        let interval_ms = params.get("interval_ms").and_then(|v| v.as_u64());
        let delay_ms = params.get("delay_ms").and_then(|v| v.as_u64());
        let cron_expr = params.get("cron_expr").and_then(|v| v.as_str()).map(|s| s.to_string());
        let message = params.get("message").and_then(|v| v.as_str()).map(|s| s.to_string());
        let max_fires = params.get("max_fires").and_then(|v| v.as_u64());
        let brain = params.get("brain").and_then(|v| v.as_bool()).unwrap_or(false);

        // At least one timing mode must be present.
        if interval_ms.is_none() && delay_ms.is_none() && cron_expr.is_none() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "at least one of interval_ms, delay_ms, or cron_expr must be provided".to_string(),
                None,
            ));
        }

        // Build action if port_id and capability_id are both present.
        let action = match (port_id.as_deref(), capability_id.as_deref()) {
            (Some(p), Some(c)) if !p.is_empty() && !c.is_empty() => {
                Some(crate::runtime::scheduler::ScheduleAction {
                    port_id: p.to_string(),
                    capability_id: c.to_string(),
                    input: input.clone(),
                })
            }
            _ => None,
        };

        // Must have either an action or a message.
        if action.is_none() && message.is_none() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "either port_id+capability_id or message must be provided".to_string(),
                None,
            ));
        }

        let now = crate::runtime::scheduler::now_epoch_ms();
        let next_fire_epoch_ms = if let Some(delay) = delay_ms {
            now + delay
        } else if let Some(interval) = interval_ms {
            now + interval
        } else {
            // cron_expr present — store but don't compute next fire (deferred).
            now
        };

        let schedule_id = Uuid::new_v4();
        let schedule = crate::runtime::scheduler::Schedule {
            id: schedule_id,
            label: label.clone(),
            delay_ms,
            interval_ms,
            cron_expr,
            action,
            goal_trigger: None,
            message,
            max_fires,
            fire_count: 0,
            brain,
            next_fire_epoch_ms,
            created_at_epoch_ms: now,
            enabled: true,
        };

        if let Some(rt) = self.runtime.get() {
            let mut store = rt.schedule_store.lock().unwrap();
            store.add(schedule)?;
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "created": true,
                "schedule_id": schedule_id.to_string(),
                "label": label
            }),
        ))
    }

    fn handle_list_schedules(
        &self,
        id: Value,
        _params: Option<Value>,
    ) -> Result<McpResponse> {
        if let Some(rt) = self.runtime.get() {
            let store = rt.schedule_store.lock().unwrap();
            let all = store.list_all();
            let schedules: Vec<Value> = all
                .iter()
                .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
                .collect();
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "count": schedules.len(),
                    "schedules": schedules
                }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "count": 0,
                    "schedules": []
                }),
            ))
        }
    }

    fn handle_cancel_schedule(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: schedule_id (string)".to_string(),
                    None,
                ));
            }
        };

        let schedule_id_str = match params.get("schedule_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "schedule_id is required".to_string(),
                    None,
                ));
            }
        };

        let schedule_uuid = match Uuid::parse_str(&schedule_id_str) {
            Ok(u) => u,
            Err(_) => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("invalid UUID: {}", schedule_id_str),
                    None,
                ));
            }
        };

        if let Some(rt) = self.runtime.get() {
            let mut store = rt.schedule_store.lock().unwrap();
            let cancelled = store.remove(&schedule_uuid)?;
            Ok(Self::success_response(
                id,
                serde_json::json!({ "cancelled": cancelled }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({ "cancelled": false }),
            ))
        }
    }

    fn handle_trigger_consolidation(
        &self,
        id: Value,
        _params: Option<Value>,
    ) -> Result<McpResponse> {
        if let Some(rt) = self.runtime.get() {
            let (schemas_induced, routines_compiled) =
                crate::memory::schemas::run_consolidation_cycle(
                    &rt.episode_store,
                    &rt.schema_store,
                    &rt.routine_store,
                    &*rt.embedder,
                );
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "schemas_induced": schemas_induced,
                    "routines_compiled": routines_compiled,
                }),
            ))
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "schemas_induced": 0,
                    "routines_compiled": 0,
                    "note": "stub mode — no runtime available"
                }),
            ))
        }
    }

    // -----------------------------------------------------------------------
    // World-state and autonomous-routine handlers
    // -----------------------------------------------------------------------

    fn handle_patch_world_state(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: add_facts and/or remove_fact_ids".to_string(),
                    None,
                ));
            }
        };

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "added": 0,
                        "removed": 0,
                        "note": "stub mode"
                    }),
                ));
            }
        };

        let mut ws = rt.world_state.lock().unwrap();
        let mut added = 0u64;
        let mut removed = 0u64;

        // Process removals first so adds can replace removed facts.
        if let Some(ids) = params.get("remove_fact_ids").and_then(|v| v.as_array()) {
            for val in ids {
                if let Some(fact_id) = val.as_str()
                    && let Ok(true) = ws.remove_fact(fact_id)
                {
                    removed += 1;
                }
            }
        }

        // Process additions.
        if let Some(facts) = params.get("add_facts").and_then(|v| v.as_array()) {
            for fact_val in facts {
                let fact: crate::types::belief::Fact = match serde_json::from_value(fact_val.clone()) {
                    Ok(f) => f,
                    Err(e) => {
                        return Ok(Self::error_response(
                            id,
                            INVALID_PARAMS,
                            format!("invalid fact: {e}"),
                            None,
                        ));
                    }
                };
                if let Err(e) = ws.add_fact(fact) {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        format!("add_fact failed: {e}"),
                        None,
                    ));
                }
                added += 1;
            }
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "added": added,
                "removed": removed,
                "snapshot_hash": ws.snapshot_hash(),
            }),
        ))
    }

    /// Unload a pack: drop every skill it registered (by `pack` field) and
    /// every port whose `port_id` appears in its manifest. Returns counts.
    fn handle_unload_pack(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let pack_id = match params
            .as_ref()
            .and_then(|p| p.get("pack_id"))
            .and_then(|v| v.as_str())
        {
            Some(p) => p.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "pack_id required".to_string(),
                    None,
                ));
            }
        };
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({"removed_skills": 0, "removed_ports": 0, "note": "stub mode"}),
                ));
            }
        };
        let (removed_skills, removed_ports, removed_pack) =
            Self::do_unload_pack(rt, &pack_id)?;
        Ok(Self::success_response(
            id,
            serde_json::json!({
                "pack_id": pack_id,
                "removed_pack": removed_pack,
                "removed_skills": removed_skills,
                "removed_ports": removed_ports,
            }),
        ))
    }

    /// Reload a pack from a manifest path. Drops the previous registration
    /// (if any) and re-registers ports + skills from the fresh spec. Lets
    /// the brain swap a port adapter or update a skill manifest without
    /// restarting the runtime.
    fn handle_reload_pack(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let manifest_path = match params
            .as_ref()
            .and_then(|p| p.get("manifest_path"))
            .and_then(|v| v.as_str())
        {
            Some(p) => p.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "manifest_path required".to_string(),
                    None,
                ));
            }
        };
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({"loaded": false, "note": "stub mode"}),
                ));
            }
        };

        // Read + parse the new manifest.
        let manifest_content = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(e) => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("failed to read manifest '{manifest_path}': {e}"),
                    None,
                ));
            }
        };
        let new_spec: crate::types::pack::PackSpec =
            match serde_json::from_str(&manifest_content) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("manifest parse failed: {e}"),
                        None,
                    ));
                }
            };

        let pack_id = new_spec.id.clone();
        let (removed_skills, removed_ports, _removed_pack) =
            Self::do_unload_pack(rt, &pack_id)?;

        // Register new ports via the real adapter factory used by
        // bootstrap. Builtin ports (filesystem/http) load directly;
        // dylib-backed ports go through DynamicPortLoader using the
        // search paths captured at bootstrap time.
        let mut added_ports = 0u64;
        let mut added_skills = 0u64;
        let mut errors: Vec<String> = Vec::new();
        {
            #[cfg(feature = "dylib-ports")]
            let mut loader = {
                let mut paths = rt.plugin_search_paths.clone();
                // Add the manifest's parent directory so the loader can find
                // dylibs co-located with the manifest (packs/<port>/ layout).
                if let Some(parent) = std::path::Path::new(&manifest_path).parent() {
                    let parent = parent.to_path_buf();
                    if !paths.contains(&parent) {
                        paths.push(parent);
                    }
                }
                crate::runtime::dynamic_port::DynamicPortLoader::with_signature_policy(
                    paths,
                    rt.require_port_signatures,
                )
            };
            let mut port_rt = rt.port_runtime.lock().unwrap();
            for port_spec in &new_spec.ports {
                let result = crate::bootstrap::create_port_adapter(
                    port_spec,
                    #[cfg(feature = "dylib-ports")]
                    &mut loader,
                );
                match result {
                    Ok((adapter, effective_spec)) => {
                        let spec_to_register =
                            effective_spec.unwrap_or_else(|| port_spec.clone());
                        let port_id = spec_to_register.port_id.clone();
                        if let Err(e) =
                            port_rt.register_port(spec_to_register, adapter)
                        {
                            errors.push(format!(
                                "port {} register failed: {e}",
                                port_id
                            ));
                            continue;
                        }
                        if let Err(e) = port_rt.activate(&port_id) {
                            errors.push(format!(
                                "port {} activate failed: {e}",
                                port_id
                            ));
                            continue;
                        }
                        added_ports += 1;
                    }
                    Err(e) => {
                        errors.push(format!(
                            "port {} adapter creation failed: {e}",
                            port_spec.port_id
                        ));
                    }
                }
            }
        }
        {
            let mut skill_rt = rt.skill_runtime.lock().unwrap();
            for skill_spec in &new_spec.skills {
                match skill_rt.register_skill(skill_spec.clone()) {
                    Ok(_) => added_skills += 1,
                    Err(e) => errors.push(format!(
                        "skill {} register failed: {e}",
                        skill_spec.skill_id
                    )),
                }
            }
        }
        {
            let mut packs = rt.pack_specs.lock().unwrap();
            packs.retain(|p| p.id != pack_id);
            packs.push(new_spec);
        }

        // Rebuild the session controller's skill registry so newly-loaded
        // skills become visible to subsequent run_step calls. The
        // controller's SkillRegistryAdapter snapshots skills at
        // construction; without this, hot-reloaded skills would be invisible
        // until restart.
        {
            let skill_rt = rt.skill_runtime.lock().unwrap();
            let new_registry =
                Box::new(crate::adapters::SkillRegistryAdapter::new(&skill_rt));
            let mut ctrl = rt.session_controller.lock().unwrap();
            ctrl.replace_skill_registry(new_registry);
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "pack_id": pack_id,
                "removed_skills": removed_skills,
                "removed_ports": removed_ports,
                "added_skills": added_skills,
                "added_ports": added_ports,
                "warnings": errors,
            }),
        ))
    }

    /// Shared unload helper used by both `unload_pack` and `reload_pack`.
    /// Walks the registered pack's skill_ids and port_ids and removes
    /// them from the live runtimes. Returns (skills_removed, ports_removed,
    /// pack_was_present).
    fn do_unload_pack(
        rt: &RuntimeHandle,
        pack_id: &str,
    ) -> Result<(u64, u64, bool)> {
        let (skill_ids, port_ids, was_present) = {
            let packs = rt.pack_specs.lock().unwrap();
            match packs.iter().find(|p| p.id == pack_id) {
                Some(p) => (
                    p.skills.iter().map(|s| s.skill_id.clone()).collect::<Vec<_>>(),
                    p.ports.iter().map(|pt| pt.port_id.clone()).collect::<Vec<_>>(),
                    true,
                ),
                None => (Vec::new(), Vec::new(), false),
            }
        };
        let mut removed_skills = 0u64;
        let mut removed_ports = 0u64;
        {
            let mut skill_rt = rt.skill_runtime.lock().unwrap();
            for sid in &skill_ids {
                if matches!(skill_rt.unregister_skill(sid), Ok(true)) {
                    removed_skills += 1;
                }
            }
        }
        {
            let mut port_rt = rt.port_runtime.lock().unwrap();
            for pid in &port_ids {
                if matches!(port_rt.remove_port(pid), Ok(true)) {
                    removed_ports += 1;
                }
            }
        }
        if was_present {
            let mut packs = rt.pack_specs.lock().unwrap();
            packs.retain(|p| p.id != pack_id);
        }
        Ok((removed_skills, removed_ports, was_present))
    }

    /// Force-evict expired facts from world state. Facts with TTL set are
    /// already filtered out of `snapshot()` automatically — this tool
    /// physically removes them so the underlying store doesn't grow
    /// unbounded between reactive ticks.
    fn handle_expire_world_facts(
        &self,
        id: Value,
        _params: Option<Value>,
    ) -> Result<McpResponse> {
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({"removed": 0, "note": "stub mode"}),
                ));
            }
        };
        let removed = {
            let mut ws = rt.world_state.lock().unwrap();
            ws.prune_expired_facts() as u64
        };
        Ok(Self::success_response(
            id,
            serde_json::json!({"removed": removed}),
        ))
    }

    fn handle_dump_world_state(
        &self,
        id: Value,
        _params: Option<Value>,
    ) -> Result<McpResponse> {
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "snapshot": {},
                        "facts": [],
                        "note": "stub mode"
                    }),
                ));
            }
        };

        let ws = rt.world_state.lock().unwrap();
        let snapshot = ws.snapshot();
        let facts: Vec<Value> = ws
            .list_facts()
            .into_iter()
            .map(|f| serde_json::to_value(f).unwrap_or(Value::Null))
            .collect();

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "snapshot": snapshot,
                "facts": facts,
                "snapshot_hash": ws.snapshot_hash(),
            }),
        ))
    }

    fn handle_set_routine_autonomous(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: routine_id (string), autonomous (bool)".to_string(),
                    None,
                ));
            }
        };

        let routine_id = params
            .get("routine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if routine_id.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "routine_id must be a non-empty string".to_string(),
                None,
            ));
        }

        let autonomous = params
            .get("autonomous")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "routine_id": routine_id,
                        "autonomous": autonomous,
                        "found": false,
                        "note": "stub mode"
                    }),
                ));
            }
        };

        let mut rs = rt.routine_store.lock().unwrap();
        match rs.set_autonomous(&routine_id, autonomous) {
            Ok(found) => Ok(Self::success_response(
                id,
                serde_json::json!({
                    "routine_id": routine_id,
                    "autonomous": autonomous,
                    "found": found,
                }),
            )),
            Err(e) => Ok(Self::error_response(
                id,
                INTERNAL_ERROR,
                format!("set_autonomous failed: {e}"),
                None,
            )),
        }
    }

    fn handle_replicate_routine(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: routine_id (string), optional peer_ids (string[])".to_string(),
                    None,
                ));
            }
        };

        let routine_id = params
            .get("routine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if routine_id.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "routine_id must be a non-empty string".to_string(),
                None,
            ));
        }

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "routine_id": routine_id,
                        "replicated_to": [],
                        "note": "stub mode"
                    }),
                ));
            }
        };

        // Look up the routine.
        let routine = {
            let rs = rt.routine_store.lock().unwrap();
            rs.get(&routine_id).cloned()
        };

        let routine = match routine {
            Some(r) => r,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("routine not found: {routine_id}"),
                    None,
                ));
            }
        };

        // Determine target peers.
        let target_peers: Vec<String> = if let Some(arr) = params.get("peer_ids").and_then(|v| v.as_array()) {
            arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
        } else {
            // Default: replicate to all known peers.
            rt.peer_ids.lock().unwrap().clone()
        };

        if target_peers.is_empty() {
            return Ok(Self::success_response(
                id,
                serde_json::json!({
                    "routine_id": routine_id,
                    "replicated_to": [],
                    "note": "no peers available"
                }),
            ));
        }

        let exec = match &rt.remote_executor {
            Some(e) => e,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "routine_id": routine_id,
                        "replicated_to": [],
                        "note": "no remote executor configured"
                    }),
                ));
            }
        };

        let transfer = crate::types::peer::RoutineTransfer {
            routine_id: routine.routine_id.clone(),
            match_conditions: routine.match_conditions.clone(),
            compiled_skill_path: routine.compiled_skill_path.clone(),
            compiled_steps: routine.compiled_steps.clone(),
            guard_conditions: routine.guard_conditions.clone(),
            expected_cost: routine.expected_cost,
            expected_effect: routine.expected_effect.clone(),
            confidence: routine.confidence,
            autonomous: routine.autonomous,
            priority: routine.priority,
            exclusive: routine.exclusive,
            policy_scope: routine.policy_scope.clone(),
            version: routine.version,
        };

        let mut successes = Vec::new();
        let mut failures = Vec::new();
        for peer_id in &target_peers {
            match exec.transfer_routine(peer_id, &transfer) {
                Ok(()) => successes.push(peer_id.clone()),
                Err(e) => failures.push(serde_json::json!({
                    "peer_id": peer_id,
                    "error": e.to_string()
                })),
            }
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "routine_id": routine_id,
                "replicated_to": successes,
                "failures": failures,
            }),
        ))
    }

    fn handle_author_routine(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: routine_id (string), match_conditions (array), steps (array)".to_string(),
                    None,
                ));
            }
        };

        let routine_id = params
            .get("routine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if routine_id.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "routine_id must be a non-empty string".to_string(),
                None,
            ));
        }

        let match_conditions_val = match params.get("match_conditions") {
            Some(v) if v.is_array() && !v.as_array().unwrap().is_empty() => v.clone(),
            _ => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "match_conditions must be a non-empty array".to_string(),
                    None,
                ));
            }
        };

        let steps_val = match params.get("steps") {
            Some(v) if v.is_array() && !v.as_array().unwrap().is_empty() => v.clone(),
            _ => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "steps must be a non-empty array".to_string(),
                    None,
                ));
            }
        };

        let match_conditions: Vec<crate::types::common::Precondition> =
            match serde_json::from_value(match_conditions_val) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("invalid match_conditions: {e}"),
                        None,
                    ));
                }
            };

        let steps_arr = steps_val.as_array().unwrap();
        let mut compiled_steps: Vec<crate::types::routine::CompiledStep> =
            Vec::with_capacity(steps_arr.len());
        for (i, step) in steps_arr.iter().enumerate() {
            let step_type = step
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match step_type {
                "skill" => {
                    let skill_id = match step.get("skill_id").and_then(|v| v.as_str()) {
                        Some(s) if !s.is_empty() => s.to_string(),
                        _ => {
                            return Ok(Self::error_response(
                                id,
                                INVALID_PARAMS,
                                format!("step {i}: skill step requires non-empty skill_id"),
                                None,
                            ));
                        }
                    };
                    let conditions: Vec<crate::types::routine::DataCondition> = step
                        .get("conditions")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|c| serde_json::from_value(c.clone()).ok())
                                .collect()
                        })
                        .unwrap_or_default();
                    compiled_steps.push(crate::types::routine::CompiledStep::Skill {
                        skill_id,
                        on_success: Self::parse_authored_next_step(step.get("on_success")),
                        on_failure: Self::parse_authored_next_step_or(
                            step.get("on_failure"),
                            crate::types::routine::NextStep::Abandon,
                        ),
                        conditions,
                    });
                }
                "sub_routine" => {
                    let sub_routine_id = match step.get("routine_id").and_then(|v| v.as_str()) {
                        Some(s) if !s.is_empty() => s.to_string(),
                        _ => {
                            return Ok(Self::error_response(
                                id,
                                INVALID_PARAMS,
                                format!("step {i}: sub_routine step requires non-empty routine_id"),
                                None,
                            ));
                        }
                    };
                    let conditions: Vec<crate::types::routine::DataCondition> = step
                        .get("conditions")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|c| serde_json::from_value(c.clone()).ok())
                                .collect()
                        })
                        .unwrap_or_default();
                    compiled_steps.push(crate::types::routine::CompiledStep::SubRoutine {
                        routine_id: sub_routine_id,
                        on_success: Self::parse_authored_next_step(step.get("on_success")),
                        on_failure: Self::parse_authored_next_step_or(
                            step.get("on_failure"),
                            crate::types::routine::NextStep::Abandon,
                        ),
                        conditions,
                    });
                }
                _ => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("step {i}: type must be \"skill\" or \"sub_routine\""),
                        None,
                    ));
                }
            }
        }

        // Extract skill_ids for the legacy compiled_skill_path field.
        let compiled_skill_path: Vec<String> = compiled_steps
            .iter()
            .filter_map(|s| match s {
                crate::types::routine::CompiledStep::Skill { skill_id, .. } => {
                    Some(skill_id.clone())
                }
                _ => None,
            })
            .collect();

        // Parse optional fields.
        let guard_conditions: Vec<crate::types::common::Precondition> = params
            .get("guard_conditions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let priority = params
            .get("priority")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let exclusive = params
            .get("exclusive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let policy_scope = params
            .get("policy_scope")
            .and_then(|v| v.as_str())
            .map(String::from);

        let autonomous = params
            .get("autonomous")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let routine = crate::types::routine::Routine {
            routine_id: routine_id.clone(),
            namespace: "llm_authored".to_string(),
            origin: crate::types::routine::RoutineOrigin::PackAuthored,
            match_conditions,
            compiled_skill_path,
            compiled_steps,
            guard_conditions,
            expected_cost: 0.0,
            expected_effect: Vec::new(),
            confidence: 1.0,
            autonomous,
            priority,
            exclusive,
            policy_scope,
            version: 0,
        };

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "created": true,
                        "routine_id": routine_id,
                        "note": "stub mode"
                    }),
                ));
            }
        };

        {
            let mut rs = rt.routine_store.lock().unwrap();
            if let Err(e) = rs.register(routine) {
                return Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("failed to register routine: {e}"),
                    None,
                ));
            }
        }

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "created": true,
                "routine_id": routine_id,
            }),
        ))
    }

    fn handle_list_routine_versions(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: routine_id (string)".to_string(),
                    None,
                ));
            }
        };

        let routine_id = params
            .get("routine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if routine_id.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "routine_id must be a non-empty string".to_string(),
                None,
            ));
        }

        if let Some(rt) = self.runtime.get() {
            let store = rt.routine_store.lock().unwrap();
            let versions = store.list_versions(&routine_id);
            let entries: Vec<Value> = versions
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "routine_id": r.routine_id,
                        "version": r.version,
                        "confidence": r.confidence,
                        "origin": r.origin,
                        "autonomous": r.autonomous,
                    })
                })
                .collect();
            Ok(Self::success_response(
                id,
                serde_json::json!({ "versions": entries }),
            ))
        } else {
            // Stub mode
            Ok(Self::success_response(
                id,
                serde_json::json!({ "versions": [] }),
            ))
        }
    }

    fn handle_rollback_routine(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: routine_id (string), target_version (integer)".to_string(),
                    None,
                ));
            }
        };

        let routine_id = params
            .get("routine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if routine_id.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "routine_id must be a non-empty string".to_string(),
                None,
            ));
        }

        let target_version = match params.get("target_version").and_then(|v| v.as_u64()) {
            Some(v) => v as u32,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "target_version must be a non-negative integer".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = self.runtime.get() {
            let mut store = rt.routine_store.lock().unwrap();
            match store.rollback(&routine_id, target_version) {
                Ok(()) => Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "rolled_back": true,
                        "routine_id": routine_id,
                        "version": target_version,
                    }),
                )),
                Err(e) => Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("rollback failed: {e}"),
                    None,
                )),
            }
        } else {
            // Stub mode
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "rolled_back": true,
                    "routine_id": routine_id,
                    "version": target_version,
                    "note": "stub mode"
                }),
            ))
        }
    }

    fn handle_sync_beliefs(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: peer_id (string)".to_string(),
                    None,
                ));
            }
        };

        let peer_id = match params.get("peer_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "peer_id is required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = self.runtime.get() {
            // Gather current world state facts and convert to BeliefFactEntry.
            let ws = rt.world_state.lock().unwrap();
            let facts: Vec<crate::distributed::sync::BeliefFactEntry> = ws
                .list_facts()
                .into_iter()
                .map(|f| crate::distributed::sync::BeliefFactEntry {
                    fact_id: f.fact_id.clone(),
                    subject: f.subject.clone(),
                    predicate: f.predicate.clone(),
                    value: f.value.clone(),
                    provenance: f.provenance,
                    confidence: f.confidence,
                    version: 1,
                    timestamp: f.timestamp,
                })
                .collect();
            drop(ws);

            let mut sync = rt.belief_sync.lock().unwrap();
            match sync.sync_belief(&peer_id, &facts) {
                Ok(result) => Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "outcome": serde_json::to_value(result.outcome).unwrap_or(Value::Null),
                        "peer_id": result.peer_id,
                        "local_version": result.local_version,
                        "remote_version": result.remote_version,
                        "freshness_ms": result.freshness_ms,
                        "stale": result.stale,
                        "details": result.details,
                    }),
                )),
                Err(e) => Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("belief sync failed: {}", e),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "outcome": "merged",
                    "peer_id": peer_id,
                    "local_version": 0,
                    "remote_version": 0,
                    "freshness_ms": 0,
                    "stale": false,
                    "details": {},
                    "note": "stub mode"
                }),
            ))
        }
    }

    fn handle_migrate_session(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: session_id (string), peer_id (string)".to_string(),
                    None,
                ));
            }
        };

        let session_id_str = match params.get("session_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "session_id is required".to_string(),
                    None,
                ));
            }
        };

        let session_id = match Uuid::parse_str(&session_id_str) {
            Ok(id) => id,
            Err(_) => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("invalid session_id UUID: {}", session_id_str),
                    None,
                ));
            }
        };

        let peer_id = match params.get("peer_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "peer_id is required".to_string(),
                    None,
                ));
            }
        };

        if let Some(rt) = self.runtime.get() {
            let dm = match &rt.delegation_manager {
                Some(dm) => Arc::clone(dm),
                None => {
                    return Ok(Self::error_response(
                        id,
                        INTERNAL_ERROR,
                        "no delegation manager configured".to_string(),
                        None,
                    ));
                }
            };

            // Look up the session and build migration data.
            let ctrl = rt.session_controller.lock().unwrap();
            let session = match ctrl.get_session_by_id(&session_id) {
                Some(s) => s.clone(),
                None => {
                    return Ok(Self::error_response(
                        id,
                        INVALID_PARAMS,
                        format!("session not found: {}", session_id),
                        None,
                    ));
                }
            };
            drop(ctrl);

            let migration_data = crate::types::peer::SessionMigrationData {
                session_id: session.session_id,
                goal: serde_json::to_value(&session.goal).unwrap_or(Value::Null),
                working_memory: serde_json::to_value(&session.working_memory)
                    .unwrap_or(Value::Null),
                belief_summary: serde_json::to_value(&session.belief).unwrap_or(Value::Null),
                pending_observations: vec![],
                current_budget: crate::types::peer::RemoteBudget {
                    risk_limit: session.budget_remaining.risk_remaining,
                    latency_limit_ms: session.budget_remaining.latency_remaining_ms,
                    resource_limit: session.budget_remaining.resource_remaining,
                    step_limit: session.budget_remaining.steps_remaining,
                },
                trace_cursor: session.trace.steps.len() as u64,
                policy_context: serde_json::json!({}),
            };

            match dm.migrate_session(&peer_id, &migration_data) {
                Ok(result) => Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "outcome": serde_json::to_value(result.outcome).unwrap_or(Value::Null),
                        "reason": result.reason,
                        "new_session_id": result.new_session_id.map(|id| id.to_string()),
                    }),
                )),
                Err(e) => Ok(Self::error_response(
                    id,
                    INTERNAL_ERROR,
                    format!("session migration failed: {}", e),
                    None,
                )),
            }
        } else {
            Ok(Self::success_response(
                id,
                serde_json::json!({
                    "outcome": "failure",
                    "reason": "stub mode",
                    "new_session_id": null,
                    "note": "stub mode"
                }),
            ))
        }
    }

    fn handle_review_routine(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    "params required: routine_id (string)".to_string(),
                    None,
                ));
            }
        };

        let routine_id = params
            .get("routine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if routine_id.is_empty() {
            return Ok(Self::error_response(
                id,
                INVALID_PARAMS,
                "routine_id must be a non-empty string".to_string(),
                None,
            ));
        }

        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => {
                return Ok(Self::success_response(
                    id,
                    serde_json::json!({
                        "routine_id": routine_id,
                        "safety": "unknown",
                        "recommendation": "needs_review",
                        "note": "stub mode"
                    }),
                ));
            }
        };

        // Look up the routine.
        let rs = rt.routine_store.lock().unwrap();
        let routine = match rs.get(&routine_id) {
            Some(r) => r.clone(),
            None => {
                return Ok(Self::error_response(
                    id,
                    INVALID_PARAMS,
                    format!("routine not found: {routine_id}"),
                    None,
                ));
            }
        };
        drop(rs);

        // Collect all skill IDs and sub-routine IDs from the effective steps.
        let effective_steps = routine.effective_steps();
        let mut all_skill_ids: Vec<String> = Vec::new();
        let mut sub_routine_ids: Vec<String> = Vec::new();

        for step in &effective_steps {
            match step {
                crate::types::routine::CompiledStep::Skill { skill_id, .. } => {
                    if !all_skill_ids.contains(skill_id) {
                        all_skill_ids.push(skill_id.clone());
                    }
                }
                crate::types::routine::CompiledStep::SubRoutine { routine_id: sub_id, .. } => {
                    if !sub_routine_ids.contains(sub_id) {
                        sub_routine_ids.push(sub_id.clone());
                    }
                }
            }
        }

        // Resolve sub-routine skill IDs.
        let rs = rt.routine_store.lock().unwrap();
        for sub_id in &sub_routine_ids {
            if let Some(sub_routine) = rs.get(sub_id) {
                for step in sub_routine.effective_steps() {
                    if let crate::types::routine::CompiledStep::Skill { skill_id, .. } = &step
                        && !all_skill_ids.contains(skill_id)
                    {
                        all_skill_ids.push(skill_id.clone());
                    }
                }
            }
        }
        drop(rs);

        // Check each skill's side effect class via the skill runtime.
        let skill_rt = rt.skill_runtime.lock().unwrap();
        let mut destructive_skills: Vec<String> = Vec::new();
        let mut unknown_skills: Vec<String> = Vec::new();
        let mut skill_safety_details: Vec<Value> = Vec::new();

        for sid in &all_skill_ids {
            if let Some(spec) = skill_rt.get_skill(sid) {
                let side_effect = crate::adapters::PolicyEngineAdapter::derive_side_effect_class(spec);
                let class_str = format!("{:?}", side_effect);
                let is_destructive = matches!(
                    side_effect,
                    crate::types::common::SideEffectClass::Destructive
                        | crate::types::common::SideEffectClass::Irreversible
                );
                if is_destructive {
                    destructive_skills.push(sid.clone());
                }
                skill_safety_details.push(serde_json::json!({
                    "skill_id": sid,
                    "side_effect_class": class_str,
                    "destructive": is_destructive,
                }));
            } else {
                unknown_skills.push(sid.clone());
                skill_safety_details.push(serde_json::json!({
                    "skill_id": sid,
                    "side_effect_class": "unknown",
                    "destructive": false,
                    "note": "skill not found in registry"
                }));
            }
        }
        drop(skill_rt);

        // Build step summaries.
        let step_summaries: Vec<Value> = effective_steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let format_next = |ns: &crate::types::routine::NextStep| -> String {
                    match ns {
                        crate::types::routine::NextStep::Continue => "continue".to_string(),
                        crate::types::routine::NextStep::Goto { step_index, max_iterations } => {
                            if let Some(max) = max_iterations {
                                format!("goto step {step_index} (max {max} iterations)")
                            } else {
                                format!("goto step {step_index}")
                            }
                        }
                        crate::types::routine::NextStep::CallRoutine { routine_id } => {
                            format!("call routine {routine_id}")
                        }
                        crate::types::routine::NextStep::Complete => "complete".to_string(),
                        crate::types::routine::NextStep::Abandon => "abandon".to_string(),
                    }
                };

                let format_conditions = |conds: &[crate::types::routine::DataCondition]| -> String {
                    if conds.is_empty() {
                        return String::new();
                    }
                    let parts: Vec<String> = conds.iter().map(|c| {
                        format!("if {} → {}", c.expression, format_next(&c.next_step))
                    }).collect();
                    format!(", conditions: [{}]", parts.join("; "))
                };

                match step {
                    crate::types::routine::CompiledStep::Skill {
                        skill_id,
                        on_success,
                        on_failure,
                        conditions,
                    } => serde_json::json!({
                        "step": i,
                        "type": "skill",
                        "skill_id": skill_id,
                        "conditions_count": conditions.len(),
                        "summary": format!(
                            "Step {i}: invoke skill {skill_id}, on success → {}, on failure → {}{}",
                            format_next(on_success),
                            format_next(on_failure),
                            format_conditions(conditions)
                        ),
                    }),
                    crate::types::routine::CompiledStep::SubRoutine {
                        routine_id: sub_id,
                        on_success,
                        on_failure,
                        conditions,
                    } => serde_json::json!({
                        "step": i,
                        "type": "sub_routine",
                        "routine_id": sub_id,
                        "conditions_count": conditions.len(),
                        "summary": format!(
                            "Step {i}: call sub-routine {sub_id}, on success → {}, on failure → {}{}",
                            format_next(on_success),
                            format_next(on_failure),
                            format_conditions(conditions)
                        ),
                    }),
                }
            })
            .collect();

        // Summarize match conditions.
        let match_summaries: Vec<String> = routine
            .match_conditions
            .iter()
            .map(|c| {
                format!(
                    "[{}] {}",
                    c.condition_type, c.description
                )
            })
            .collect();

        // Summarize guard conditions.
        let guard_summaries: Vec<String> = routine
            .guard_conditions
            .iter()
            .map(|c| {
                format!(
                    "[{}] {}",
                    c.condition_type, c.description
                )
            })
            .collect();

        // Determine safety.
        let safety = if !unknown_skills.is_empty() {
            "unknown"
        } else if !destructive_skills.is_empty() {
            "review_required"
        } else {
            "safe"
        };

        // Determine recommendation.
        let recommendation = if routine.confidence > 0.7 && safety == "safe" {
            "ready_for_autonomous"
        } else {
            "needs_review"
        };

        let origin_str = format!("{:?}", routine.origin);

        Ok(Self::success_response(
            id,
            serde_json::json!({
                "routine_id": routine.routine_id,
                "version": routine.version,
                "origin": origin_str,
                "confidence": routine.confidence,
                "autonomous": routine.autonomous,
                "priority": routine.priority,
                "exclusive": routine.exclusive,
                "policy_scope": routine.policy_scope,
                "match_conditions": match_summaries,
                "guard_conditions": guard_summaries,
                "steps": step_summaries,
                "sub_routines": sub_routine_ids,
                "all_skill_ids": all_skill_ids,
                "skill_safety": skill_safety_details,
                "destructive_skills": destructive_skills,
                "unknown_skills": unknown_skills,
                "safety": safety,
                "recommendation": recommendation,
            }),
        ))
    }

    fn handle_handoff_session(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id, INVALID_PARAMS,
                    "params required: session_id, from_device".to_string(), None,
                ));
            }
        };
        let session_id = params.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let from_device = params.get("from_device").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let to_device = params.get("to_device").and_then(|v| v.as_str()).map(|s| s.to_string());
        let objective = params.get("objective").and_then(|v| v.as_str()).map(|s| s.to_string());
        if session_id.is_empty() || from_device.is_empty() {
            return Ok(Self::error_response(
                id, INVALID_PARAMS,
                "session_id and from_device are required".to_string(), None,
            ));
        }
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => return Ok(Self::success_response(id, serde_json::json!({"success": true, "note": "stub mode"}))),
        };
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let fact = crate::types::belief::Fact {
            fact_id: format!("handoff-{}-{}", session_id, ts),
            subject: format!("session:{}", session_id),
            predicate: "handoff".to_string(),
            value: serde_json::json!({
                "from_device": from_device, "to_device": to_device,
                "session_id": session_id, "objective": objective, "ts": ts,
            }),
            confidence: 1.0,
            provenance: crate::types::common::FactProvenance::Asserted,
            timestamp: now, ttl_ms: None,
        };
        let mut ws = rt.world_state.lock().unwrap();
        ws.add_fact(fact)?;
        let hash = ws.snapshot_hash();
        drop(ws);
        Ok(Self::success_response(id, serde_json::json!({
            "success": true, "session_id": session_id, "snapshot_hash": hash,
        })))
    }

    fn handle_claim_session(
        &self,
        id: Value,
        params: Option<Value>,
    ) -> Result<McpResponse> {
        let params = match params {
            Some(p) => p,
            None => {
                return Ok(Self::error_response(
                    id, INVALID_PARAMS,
                    "params required: session_id, device_id".to_string(), None,
                ));
            }
        };
        let session_id = params.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let device_id = params.get("device_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if session_id.is_empty() || device_id.is_empty() {
            return Ok(Self::error_response(
                id, INVALID_PARAMS,
                "session_id and device_id are required".to_string(), None,
            ));
        }
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => return Ok(Self::success_response(id, serde_json::json!({"success": true, "note": "stub mode"}))),
        };
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let fact = crate::types::belief::Fact {
            fact_id: format!("claim-{}-{}", session_id, ts),
            subject: format!("session:{}", session_id),
            predicate: "claimed_by".to_string(),
            value: serde_json::json!({"device_id": device_id, "ts": ts}),
            confidence: 1.0,
            provenance: crate::types::common::FactProvenance::Asserted,
            timestamp: now, ttl_ms: None,
        };
        let mut ws = rt.world_state.lock().unwrap();
        ws.add_fact(fact)?;
        let hash = ws.snapshot_hash();
        drop(ws);
        Ok(Self::success_response(id, serde_json::json!({
            "success": true, "session_id": session_id, "device_id": device_id, "snapshot_hash": hash,
        })))
    }

    fn parse_authored_next_step(
        val: Option<&serde_json::Value>,
    ) -> crate::types::routine::NextStep {
        Self::parse_authored_next_step_or(val, crate::types::routine::NextStep::Continue)
    }

    /// Parse an action object into a `NextStep`, returning `default` when the
    /// value is absent or unparseable.
    fn parse_authored_next_step_or(
        val: Option<&serde_json::Value>,
        default: crate::types::routine::NextStep,
    ) -> crate::types::routine::NextStep {
        let val = match val {
            Some(v) => v,
            None => return default,
        };
        serde_json::from_value(val.clone()).unwrap_or(default)
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

    fn extract_uuid(params: &Option<Value>, field: &str) -> Option<Uuid> {
        params
            .as_ref()
            .and_then(|p| p.get(field))
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
    }

    fn success_response(id: Value, result: Value) -> McpResponse {
        McpResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Wrap a tool result in the MCP content array format required by tools/call.
    fn tool_success_response(id: Value, result: Value) -> McpResponse {
        let text = serde_json::to_string_pretty(&result).unwrap_or_default();
        Self::success_response(
            id,
            serde_json::json!({
                "content": [{ "type": "text", "text": text }]
            }),
        )
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
    // Implicit session management
    // -----------------------------------------------------------------------

    /// Record a successful invoke_port call into the current implicit session.
    /// If a stale session exists (last activity > timeout), finalize it first
    /// and start a new one.
    fn record_implicit_call(
        &self,
        port_id: String,
        capability_id: String,
        record: crate::types::observation::PortCallRecord,
    ) {
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => return,
        };

        let now = Instant::now();

        // Take the old session out if it's stale.
        let stale = {
            let mut guard = self.implicit_session.lock().unwrap();
            if let Some(ref sess) = *guard {
                if now.duration_since(sess.last_activity) > IMPLICIT_SESSION_TIMEOUT {
                    guard.take()
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Finalize the stale session outside the lock.
        if let Some(old) = stale {
            self.finalize_implicit_session(old, rt);
        }

        // Append to the current session or create a new one.
        let mut guard = self.implicit_session.lock().unwrap();
        match guard.as_mut() {
            Some(sess) => {
                sess.skill_sequence.push((port_id, capability_id));
                sess.observations.push(record);
                sess.last_activity = now;
            }
            None => {
                *guard = Some(ImplicitSession {
                    skill_sequence: vec![(port_id, capability_id)],
                    observations: vec![record],
                    started_at: now,
                    last_activity: now,
                });
            }
        }
    }

    /// Flush any open implicit session, finalizing it as an episode.
    /// Called when the caller switches to a non-invoke_port MCP method,
    /// signaling the end of a logical "turn".
    fn flush_implicit_session(&self) {
        let rt = match self.runtime.get() {
            Some(rt) => rt,
            None => return,
        };

        let session = {
            let mut guard = self.implicit_session.lock().unwrap();
            guard.take()
        };

        if let Some(sess) = session {
            self.finalize_implicit_session(sess, rt);
        }
    }

    /// Convert a completed implicit session into an episode and store it.
    /// Only creates episodes from sessions with 2+ steps — single
    /// invoke_port calls are not patterns worth learning from.
    fn finalize_implicit_session(&self, session: ImplicitSession, rt: &RuntimeHandle) {
        // Patch WorldState with last-called facts from this session.
        // This runs for ALL sessions (including single-call) so the reactive
        // monitor can match routines against recent port activity.
        if let Ok(mut ws) = rt.world_state.lock() {
            for ((port_id, cap_id), obs) in session.skill_sequence.iter()
                .zip(session.observations.iter())
            {
                let fact = crate::types::belief::Fact {
                    fact_id: format!("last_call.{}.{}", port_id, cap_id),
                    subject: port_id.clone(),
                    predicate: format!("last_{}", cap_id),
                    value: if obs.success {
                        serde_json::json!(true)
                    } else {
                        serde_json::json!(false)
                    },
                    confidence: if obs.success { 1.0 } else { 0.5 },
                    provenance: crate::types::common::FactProvenance::Observed,
                    timestamp: chrono::Utc::now(),
                                    ttl_ms: None,
                };
                let _ = ws.add_fact(fact);
            }
        }

        if session.skill_sequence.len() < 2 {
            return;
        }

        // Build a goal fingerprint from the skill sequence so episodes with
        // the same port.capability pattern cluster together for PrefixSpan.
        let fingerprint = session
            .skill_sequence
            .iter()
            .map(|(p, c)| format!("{}.{}", p, c))
            .collect::<Vec<_>>()
            .join("\u{2192}"); // → arrow character

        let embedding = Some(rt.embedder.embed(&fingerprint));

        let outcome = if session.observations.iter().all(|o| o.success) {
            crate::types::episode::EpisodeOutcome::Success
        } else if session.observations.iter().any(|o| o.success) {
            crate::types::episode::EpisodeOutcome::PartialSuccess
        } else {
            crate::types::episode::EpisodeOutcome::Failure
        };
        let success = outcome == crate::types::episode::EpisodeOutcome::Success;

        let session_id = Uuid::new_v4();

        let steps: Vec<crate::types::episode::EpisodeStep> = session
            .skill_sequence
            .iter()
            .zip(session.observations.iter())
            .enumerate()
            .map(|(i, ((port_id, cap_id), pcr))| {
                let skill_id = format!("{}.{}", port_id, cap_id);
                let observation = crate::types::observation::Observation {
                    observation_id: pcr.observation_id,
                    session_id,
                    skill_id: Some(skill_id.clone()),
                    port_calls: vec![pcr.clone()],
                    raw_result: pcr.raw_result.clone(),
                    structured_result: pcr.structured_result.clone(),
                    effect_patch: pcr.effect_patch.clone(),
                    success: pcr.success,
                    failure_class: None,
                    failure_detail: None,
                    latency_ms: pcr.latency_ms,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: pcr.confidence,
                    timestamp: pcr.timestamp,
                };
                crate::types::episode::EpisodeStep {
                    step_index: i as u32,
                    belief_summary: serde_json::json!({}),
                    candidates_considered: vec![skill_id.clone()],
                    predicted_scores: vec![1.0],
                    selected_skill: skill_id,
                    observation,
                    belief_patch: serde_json::json!({}),
                    progress_delta: 1.0 / session.skill_sequence.len() as f64,
                    critic_decision: "Continue".to_string(),
                    timestamp: pcr.timestamp,
                }
            })
            .collect();

        let observations: Vec<crate::types::observation::Observation> =
            steps.iter().map(|s| s.observation.clone()).collect();

        let total_cost: f64 = session
            .observations
            .iter()
            .map(|o| o.resource_cost)
            .sum();

        let salience = {
            let outcome_weight = match outcome {
                crate::types::episode::EpisodeOutcome::Success => 1.0_f64,
                crate::types::episode::EpisodeOutcome::PartialSuccess => 0.5,
                crate::types::episode::EpisodeOutcome::Failure => 0.2,
                _ => 0.1,
            };
            let efficiency = if total_cost > 0.0 {
                (1.0 - (total_cost / 100.0).min(1.0)).max(0.0)
            } else {
                0.5
            };
            (outcome_weight * 0.7 + efficiency * 0.3).clamp(0.0, 1.0)
        };

        let ws_context = rt.world_state.lock().ok()
            .map(|ws| ws.snapshot())
            .unwrap_or(serde_json::json!({}));

        let episode = crate::types::episode::Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: fingerprint.clone(),
            initial_belief_summary: serde_json::json!({}),
            steps,
            observations,
            outcome,
            total_cost,
            success,
            tags: vec!["implicit-session".to_string()],
            embedding,
            created_at: chrono::Utc::now(),
            salience,
            world_state_context: ws_context,
        };

        let step_count = episode.steps.len();
        let duration_secs = session
            .last_activity
            .duration_since(session.started_at)
            .as_secs_f64();
        let sequence_display = session
            .skill_sequence
            .iter()
            .map(|(p, c)| format!("{}.{}", p, c))
            .collect::<Vec<_>>()
            .join(" \u{2192} ");

        let adapter = crate::adapters::EpisodeMemoryAdapter::new(
            Arc::clone(&rt.episode_store),
            Arc::clone(&rt.embedder),
        );
        if let Err(e) = adapter.store(episode) {
            tracing::warn!(error = %e, "failed to store implicit session episode");
        } else {
            eprintln!(
                "[implicit-session] stored episode: {} steps, {:.1}s, sequence: {}",
                step_count, duration_secs, sequence_display
            );
            crate::interfaces::cli::attempt_learning(
                &rt.episode_store,
                &rt.schema_store,
                &rt.routine_store,
                &fingerprint,
                &*rt.embedder,
            );
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
                        },
                        "max_steps": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Per-goal override for the session step budget. Defaults to the runtime's configured max_steps (100)."
                        }
                    },
                    "required": ["objective"]
                }),
            },
            McpTool {
                name: "create_goal_async".to_string(),
                description: "Submit a goal and return immediately with a goal_id. The runtime drives the control loop on a background thread; poll get_goal_status to watch it finish.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "objective": {
                            "type": "string",
                            "description": "The goal objective description"
                        },
                        "inputs": {
                            "type": "object",
                            "description": "Structured key-value inputs seeded into belief bindings before execution. Keys must match the target skill's input schema fields."
                        },
                        "max_steps": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Per-goal override for the session step budget."
                        }
                    },
                    "required": ["objective"]
                }),
            },
            McpTool {
                name: "get_goal_status".to_string(),
                description: "Return the live status of a goal started with create_goal_async.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "goal_id": {
                            "type": "string",
                            "description": "The goal_id returned by create_goal_async"
                        }
                    },
                    "required": ["goal_id"]
                }),
            },
            McpTool {
                name: "stream_goal_observations".to_string(),
                description: "Pull observations produced by an async goal since a given step. Brain polls this with the last seen step_index for near-real-time visibility into long-running async goals (proprioception). Returns events plus a `terminal` flag — once true, no further events will arrive.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "goal_id": {
                            "type": "string",
                            "description": "The goal_id returned by create_goal_async"
                        },
                        "after_step": {
                            "type": "integer",
                            "description": "Return events with step_index > this value. Pass -1 (or omit) for all events.",
                            "default": -1
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of events to return. Defaults to 100.",
                            "default": 100
                        }
                    },
                    "required": ["goal_id"]
                }),
            },
            McpTool {
                name: "cancel_goal".to_string(),
                description: "Request cancellation of an async goal. The background thread checks the cancel flag between control-loop steps and transitions the session to Aborted.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "goal_id": {
                            "type": "string",
                            "description": "The goal_id returned by create_goal_async"
                        }
                    },
                    "required": ["goal_id"]
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
                name: "inspect_belief_projection".to_string(),
                description: "Show full belief vs JMESPath-projected vs TOON-encoded for a session. Compares sizes and reduction percentages.".to_string(),
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
                name: "provide_session_input".to_string(),
                description: "Provide missing input bindings to a WaitingForInput session. The external brain calls this after inspecting the session's pending_input_request to fill slots the body could not resolve. Optionally override the skill selection with redirect_skill_id. Auto-resumes the session.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID (must be in WaitingForInput status)"
                        },
                        "bindings": {
                            "type": "object",
                            "description": "JSON object of slot_name → value for each missing input"
                        },
                        "redirect_skill_id": {
                            "type": "string",
                            "description": "Optional: override the body's skill selection. The next step will execute this skill instead of the originally selected one."
                        }
                    },
                    "required": ["session_id", "bindings"]
                }),
            },
            McpTool {
                name: "inject_plan".to_string(),
                description: "Inject a compiled execution plan into a session. The body will follow the plan steps in order using its existing plan-following engine. Used by the brain orchestrator to compose multi-step workflows from known routines and skills.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        },
                        "steps": {
                            "type": "array",
                            "description": "Array of CompiledStep objects. Each is either {\"Skill\":{\"skill_id\":\"...\"}} or {\"SubRoutine\":{\"routine_id\":\"...\"}}.",
                            "items": { "type": "object" }
                        }
                    },
                    "required": ["session_id", "steps"]
                }),
            },
            McpTool {
                name: "find_routines".to_string(),
                description: "Search compiled routines by similarity to a goal description. Returns matching routines with their skill paths and confidence. Used by the brain to check if a sub-goal already has a learned routine before composing a plan.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Goal description to search for. If empty, lists all routines."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results (default: 5)"
                        }
                    }
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
                description: "Resume a paused/waiting session. If the session is in WaitingForInput or WaitingForRemote, supply `payload` to deliver the requested data — each key becomes a belief binding. For async goals (started via create_goal_async), the background worker is automatically respawned.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID"
                        },
                        "payload": {
                            "type": "object",
                            "description": "Optional JSON object whose top-level keys are merged into belief bindings before resume. Use this to satisfy a WaitingForInput or WaitingForRemote request."
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
            McpTool {
                name: "invoke_port".to_string(),
                description: "Invoke a capability on a loaded port. Returns a PortCallRecord with the result, latency, success status, and tracing metadata.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "port_id": {
                            "type": "string",
                            "description": "The port identifier (e.g. \"smtp\", \"postgres\", \"s3\")"
                        },
                        "capability_id": {
                            "type": "string",
                            "description": "The capability to invoke (e.g. \"send_plain\", \"query\", \"put_object\")"
                        },
                        "input": {
                            "type": "object",
                            "description": "Input payload for the capability"
                        }
                    },
                    "required": ["port_id", "capability_id"]
                }),
            },
            McpTool {
                name: "list_ports".to_string(),
                description: "List all loaded ports and their capabilities. Use this to discover available ports before invoking them.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "namespace": {
                            "type": "string",
                            "description": "Optional namespace filter"
                        }
                    }
                }),
            },
            McpTool {
                name: "list_capabilities".to_string(),
                description: "List skills and ports the brain can invoke right now, partitioned into allowed/denied by permissions_scope. Optionally pass goal_id or session_id to scope the answer and include remaining budget. Without either, returns the full unfiltered catalog.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "goal_id": {
                            "type": "string",
                            "description": "Optional async goal id to scope the catalog"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "Optional session id to scope the catalog"
                        }
                    }
                }),
            },
            McpTool {
                name: "list_peers".to_string(),
                description: "List connected remote SOMA peers. Each peer is another SOMA instance reachable over TCP, WebSocket, or Unix socket.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpTool {
                name: "invoke_remote_skill".to_string(),
                description: "Invoke a skill on a remote SOMA peer. The remote peer executes the skill using its own loaded packs and ports, and returns the observation.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "peer_id": {
                            "type": "string",
                            "description": "The peer identifier (e.g. \"peer-0\", \"unix-peer-0\")"
                        },
                        "skill_id": {
                            "type": "string",
                            "description": "The skill to invoke on the remote peer"
                        },
                        "input": {
                            "type": "object",
                            "description": "Input payload for the skill"
                        }
                    },
                    "required": ["peer_id", "skill_id"]
                }),
            },
            McpTool {
                name: "transfer_routine".to_string(),
                description: "Transfer a locally compiled routine to a remote peer. The peer stores the routine and can use it for plan-following without re-learning.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "peer_id": {
                            "type": "string",
                            "description": "The target peer identifier"
                        },
                        "routine_id": {
                            "type": "string",
                            "description": "The local routine ID to transfer"
                        }
                    },
                    "required": ["peer_id", "routine_id"]
                }),
            },
            McpTool {
                name: "schedule".to_string(),
                description: "Create a scheduled task. Either a one-shot delay, a recurring interval, or a cron expression. The task can invoke a port capability, emit a plain message, or both.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "Human-readable label"
                        },
                        "delay_ms": {
                            "type": "integer",
                            "description": "Fire once after N milliseconds (one-shot)"
                        },
                        "interval_ms": {
                            "type": "integer",
                            "description": "Fire every N milliseconds (recurring)"
                        },
                        "cron_expr": {
                            "type": "string",
                            "description": "Cron expression (stored for future support)"
                        },
                        "message": {
                            "type": "string",
                            "description": "Plain text message to show in chat (no port call needed)"
                        },
                        "port_id": {
                            "type": "string",
                            "description": "Port to invoke (for port-call mode)"
                        },
                        "capability_id": {
                            "type": "string",
                            "description": "Capability on the port"
                        },
                        "input": {
                            "type": "object",
                            "description": "Payload for the port call"
                        },
                        "max_fires": {
                            "type": "integer",
                            "description": "Stop after this many fires (omit for unlimited)"
                        },
                        "brain": {
                            "type": "boolean",
                            "description": "Route result through LLM brain for interpretation"
                        }
                    },
                    "required": ["label"]
                }),
            },
            McpTool {
                name: "list_schedules".to_string(),
                description: "List all active schedules.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpTool {
                name: "cancel_schedule".to_string(),
                description: "Cancel a scheduled task by ID.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "schedule_id": {
                            "type": "string",
                            "description": "The schedule UUID to cancel"
                        }
                    },
                    "required": ["schedule_id"]
                }),
            },
            McpTool {
                name: "trigger_consolidation".to_string(),
                description: "Trigger a consolidation cycle (the 'sleep' equivalent). Replays all stored episodes, induces schemas via PrefixSpan, and compiles routines from high-confidence schemas. Returns the number of schemas induced and routines compiled.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpTool {
                name: "execute_routine".to_string(),
                description: "Execute a compiled routine by ID. Runs the routine's pre-learned skill sequence to completion without full deliberation.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "routine_id": {
                            "type": "string",
                            "description": "The routine ID to execute"
                        },
                        "input": {
                            "type": "object",
                            "description": "Optional input bindings for the routine's skills"
                        }
                    },
                    "required": ["routine_id"]
                }),
            },
            McpTool {
                name: "expire_world_facts".to_string(),
                description: "Force-evict world-state facts whose TTL has elapsed. Snapshot reads already filter expired facts; call this to physically free memory.".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
            },
            McpTool {
                name: "reload_pack".to_string(),
                description: "Re-register a pack from a manifest file. Drops the previous pack's skills (and any ports the runtime can free), then re-registers from the fresh spec. Lets the brain hot-swap skill metadata without restarting. Port adapter swap is limited: dynamic dylib reload requires the dylib loader path which is not yet wired into this tool.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "manifest_path": {
                            "type": "string",
                            "description": "Filesystem path to the pack manifest JSON"
                        }
                    },
                    "required": ["manifest_path"]
                }),
            },
            McpTool {
                name: "unload_pack".to_string(),
                description: "Drop every skill and port that the named pack registered. Used to prune obsolete packs at runtime.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pack_id": {
                            "type": "string",
                            "description": "Pack id (matches the `id` field in the manifest)"
                        }
                    },
                    "required": ["pack_id"]
                }),
            },
            McpTool {
                name: "patch_world_state".to_string(),
                description: "Add or remove facts in the world state. Each fact may carry an optional `ttl_ms` after which it is auto-expired. Changes may trigger autonomous routines on the next monitor tick.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "add_facts": {
                            "type": "array",
                            "description": "Facts to add or upsert",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "fact_id": { "type": "string" },
                                    "subject": { "type": "string" },
                                    "predicate": { "type": "string" },
                                    "value": { "description": "Any JSON value" },
                                    "confidence": { "type": "number" }
                                },
                                "required": ["fact_id", "subject", "predicate", "value", "confidence"]
                            }
                        },
                        "remove_fact_ids": {
                            "type": "array",
                            "description": "Fact IDs to remove",
                            "items": { "type": "string" }
                        }
                    }
                }),
            },
            McpTool {
                name: "dump_world_state".to_string(),
                description: "Return the full world state snapshot including all facts and the current snapshot hash.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            McpTool {
                name: "set_routine_autonomous".to_string(),
                description: "Set or clear the autonomous flag on a routine. Autonomous routines are automatically executed by the reactive monitor when their match conditions are satisfied by the world state.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "routine_id": {
                            "type": "string",
                            "description": "The routine ID to modify"
                        },
                        "autonomous": {
                            "type": "boolean",
                            "description": "Whether the routine should run autonomously"
                        }
                    },
                    "required": ["routine_id", "autonomous"]
                }),
            },
            McpTool {
                name: "replicate_routine".to_string(),
                description: "Replicate a compiled routine to remote peers. Transfers the routine (including compiled steps, match conditions, and guard conditions) so the peer can execute it locally. If peer_ids is omitted, replicates to all known peers.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "routine_id": {
                            "type": "string",
                            "description": "The routine ID to replicate"
                        },
                        "peer_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Target peer IDs (optional, defaults to all known peers)"
                        }
                    },
                    "required": ["routine_id"]
                }),
            },
            McpTool {
                name: "author_routine".to_string(),
                description: "Create or update a routine from a structured definition. The LLM translates natural language behavioral intent into a compiled routine that the runtime can execute, match against world state, and fire autonomously.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "routine_id": { "type": "string", "description": "Unique identifier for the routine" },
                        "match_conditions": {
                            "type": "array",
                            "description": "Conditions that trigger this routine (goal_fingerprint or world_state)",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "condition_type": { "type": "string" },
                                    "expression": { "description": "JSON expression to match against context" },
                                    "description": { "type": "string" }
                                },
                                "required": ["condition_type", "expression", "description"]
                            }
                        },
                        "steps": {
                            "type": "array",
                            "description": "Ordered execution steps with branching",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string", "enum": ["skill", "sub_routine"] },
                                    "skill_id": { "type": "string", "description": "For skill steps" },
                                    "routine_id": { "type": "string", "description": "For sub_routine steps" },
                                    "on_success": { "type": "object", "description": "Action on success (default: continue)" },
                                    "on_failure": { "type": "object", "description": "Action on failure (default: abandon)" },
                                    "conditions": {
                                        "type": "array",
                                        "description": "Data conditions evaluated against the observation's structured_result on success. First match wins, overriding on_success.",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "expression": { "description": "JSON object to match against structured_result. Key with value true = existence check; other values = exact match." },
                                                "description": { "type": "string" },
                                                "next_step": { "type": "object", "description": "NextStep action if condition matches (same format as on_success/on_failure)" }
                                            },
                                            "required": ["expression", "description", "next_step"]
                                        }
                                    }
                                },
                                "required": ["type"]
                            }
                        },
                        "guard_conditions": {
                            "type": "array",
                            "description": "Optional conditions that must ALL pass",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "condition_type": { "type": "string" },
                                    "expression": { "description": "JSON expression to match against context" },
                                    "description": { "type": "string" }
                                },
                                "required": ["condition_type", "expression", "description"]
                            }
                        },
                        "priority": { "type": "integer", "description": "Higher fires first (default 0)" },
                        "exclusive": { "type": "boolean", "description": "If true, blocks lower-priority matches (default false)" },
                        "policy_scope": { "type": "string", "description": "Optional policy namespace override" },
                        "autonomous": { "type": "boolean", "description": "If true, reactive monitor fires this automatically (default false)" }
                    },
                    "required": ["routine_id", "match_conditions", "steps"]
                }),
            },
            McpTool {
                name: "list_routine_versions".to_string(),
                description: "List all versions of a routine, including history and current."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "routine_id": {
                            "type": "string",
                            "description": "The routine ID to list versions for"
                        }
                    },
                    "required": ["routine_id"]
                }),
            },
            McpTool {
                name: "rollback_routine".to_string(),
                description: "Roll back a routine to a previous version from its history."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "routine_id": {
                            "type": "string",
                            "description": "The routine ID to roll back"
                        },
                        "target_version": {
                            "type": "integer",
                            "description": "The version number to roll back to"
                        }
                    },
                    "required": ["routine_id", "target_version"]
                }),
            },
            McpTool {
                name: "sync_beliefs".to_string(),
                description: "Synchronize belief facts with a remote peer. Sends current world state facts to the peer and merges the result.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "peer_id": {
                            "type": "string",
                            "description": "The peer identifier to sync beliefs with"
                        }
                    },
                    "required": ["peer_id"]
                }),
            },
            McpTool {
                name: "migrate_session".to_string(),
                description: "Migrate an active session to a remote peer. Transfers goal, working memory, belief, observations, budget, trace, and policy context atomically.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session UUID to migrate"
                        },
                        "peer_id": {
                            "type": "string",
                            "description": "The target peer identifier"
                        }
                    },
                    "required": ["session_id", "peer_id"]
                }),
            },
            McpTool {
                name: "review_routine".to_string(),
                description: "Review a routine's safety profile before marking it autonomous. Returns a human-readable summary of what the routine does, which ports/skills it touches, side effect assessment, and a recommendation.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "routine_id": {
                            "type": "string",
                            "description": "The routine to review"
                        }
                    },
                    "required": ["routine_id"]
                }),
            },
            McpTool {
                name: "handoff_session".to_string(),
                description: "Hand off a session to another device by writing a handoff fact to world state.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "The session to hand off" },
                        "from_device": { "type": "string", "description": "The device handing off" },
                        "to_device": { "type": "string", "description": "Optional target device ID" },
                        "objective": { "type": "string", "description": "Optional objective for the receiver" }
                    },
                    "required": ["session_id", "from_device"]
                }),
            },
            McpTool {
                name: "claim_session".to_string(),
                description: "Claim a handed-off session by writing a claim fact to world state.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "The session to claim" },
                        "device_id": { "type": "string", "description": "The device claiming" }
                    },
                    "required": ["session_id", "device_id"]
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
    #[cfg(feature = "native-filesystem")]
    use crate::runtime::port::Port;

    fn make_request(method: &str, params: Option<Value>) -> McpRequest {
        McpRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Value::Number(1.into()),
        }
    }

    /// Extract the inner JSON from an MCP content-wrapped tools/call response.
    fn unwrap_tool_result(result: &Value) -> Value {
        let text = result["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
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
        assert_eq!(tools.len(), 48);
    }

    #[test]
    fn test_list_tools_names() {
        let server = McpServer::new_stub();
        let names: Vec<String> = server.list_tools().iter().map(|t| t.name.clone()).collect();
        assert!(names.contains(&"create_goal".to_string()));
        assert!(names.contains(&"inspect_session".to_string()));
        assert!(names.contains(&"inspect_belief".to_string()));
        assert!(names.contains(&"provide_session_input".to_string()));
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
        assert!(names.contains(&"invoke_port".to_string()));
        assert!(names.contains(&"list_ports".to_string()));
        assert!(names.contains(&"trigger_consolidation".to_string()));
        assert!(names.contains(&"execute_routine".to_string()));
        assert!(names.contains(&"patch_world_state".to_string()));
        assert!(names.contains(&"dump_world_state".to_string()));
        assert!(names.contains(&"set_routine_autonomous".to_string()));
        assert!(names.contains(&"replicate_routine".to_string()));
        assert!(names.contains(&"author_routine".to_string()));
        assert!(names.contains(&"list_routine_versions".to_string()));
        assert!(names.contains(&"rollback_routine".to_string()));
        assert!(names.contains(&"sync_beliefs".to_string()));
        assert!(names.contains(&"migrate_session".to_string()));
        assert!(names.contains(&"review_routine".to_string()));
        assert!(names.contains(&"handoff_session".to_string()));
        assert!(names.contains(&"claim_session".to_string()));
        assert!(names.contains(&"inject_plan".to_string()));
        assert!(names.contains(&"find_routines".to_string()));
    }

    #[test]
    fn test_inject_plan_missing_session() {
        let server = McpServer::new_stub();
        let resp = server.handle_request(McpRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "inject_plan",
                "arguments": {
                    "session_id": "00000000-0000-0000-0000-000000000001",
                    "steps": [{"type": "skill", "skill_id": "test.skill"}]
                }
            })),
        }).unwrap();
        let text = resp.result
            .and_then(|r| r.get("content").cloned())
            .and_then(|c| c.as_array().cloned())
            .and_then(|a| a.first().cloned())
            .and_then(|c| c.get("text").cloned())
            .and_then(|t| t.as_str().map(|s| s.to_string()));
        let has_error = resp.error.is_some()
            || text.as_deref().map(|t| t.contains("not found")).unwrap_or(false)
            || text.as_deref().map(|t| t.contains("error")).unwrap_or(false);
        assert!(has_error);
    }

    #[test]
    fn test_inject_plan_empty_steps_rejected() {
        let server = McpServer::new_stub();
        let resp = server.handle_request(McpRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "inject_plan",
                "arguments": {
                    "session_id": "00000000-0000-0000-0000-000000000001",
                    "steps": []
                }
            })),
        }).unwrap();
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_find_routines_stub_returns_error() {
        let server = McpServer::new_stub();
        let resp = server.handle_request(McpRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "find_routines",
                "arguments": {
                    "query": "compute sha256 hash",
                    "limit": 5
                }
            })),
        }).unwrap();
        assert!(resp.error.is_some());
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
    fn test_execute_routine_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "execute_routine",
            Some(serde_json::json!({ "routine_id": "test-routine-1" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["routine_id"], "test-routine-1");
        assert_eq!(result["status"], "completed");
        assert!(result["session_id"].as_str().is_some());
    }

    #[test]
    fn test_execute_routine_missing_id() {
        let server = McpServer::new_stub();
        let req = make_request("execute_routine", Some(serde_json::json!({})));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_execute_routine_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("execute_routine", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_replicate_routine_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "replicate_routine",
            Some(serde_json::json!({ "routine_id": "test-routine-1" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["routine_id"], "test-routine-1");
        assert!(result["replicated_to"].is_array());
        assert_eq!(result["note"], "stub mode");
    }

    #[test]
    fn test_replicate_routine_missing_id() {
        let server = McpServer::new_stub();
        let req = make_request("replicate_routine", Some(serde_json::json!({})));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_replicate_routine_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("replicate_routine", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_author_routine_stub() {
        let server = McpServer::new_stub();
        let req = make_request("author_routine", Some(serde_json::json!({
            "routine_id": "test_authored",
            "match_conditions": [{"condition_type": "world_state", "expression": {"event": true}, "description": "test"}],
            "steps": [{"type": "skill", "skill_id": "do_thing"}]
        })));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["routine_id"], "test_authored");
        assert_eq!(result["created"], true);
    }

    #[test]
    fn test_author_routine_missing_id() {
        let server = McpServer::new_stub();
        let req = make_request("author_routine", Some(serde_json::json!({
            "match_conditions": [{"condition_type": "world_state", "expression": {}, "description": "test"}],
            "steps": [{"type": "skill", "skill_id": "x"}]
        })));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_author_routine_empty_steps() {
        let server = McpServer::new_stub();
        let req = make_request("author_routine", Some(serde_json::json!({
            "routine_id": "test",
            "match_conditions": [{"condition_type": "world_state", "expression": {}, "description": "test"}],
            "steps": []
        })));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_author_routine_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("author_routine", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_list_routine_versions_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "list_routine_versions",
            Some(serde_json::json!({ "routine_id": "test_routine" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["versions"].is_array());
    }

    #[test]
    fn test_rollback_routine_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "rollback_routine",
            Some(serde_json::json!({ "routine_id": "test_routine", "target_version": 0 })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["rolled_back"], true);
        assert_eq!(result["routine_id"], "test_routine");
        assert_eq!(result["version"], 0);
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
        let inner = unwrap_tool_result(&result);
        assert_eq!(inner["status"], "created");
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
        assert_eq!(server.list_tools().len(), 48);
    }

    /// Build a real RuntimeHandle from a bootstrapped runtime (no packs).
    fn make_wired_server() -> McpServer {
        let mut config = crate::config::SomaConfig::default();
        // Use in-memory stores so tests don't share disk state.
        config.soma.data_dir = String::new();
        let runtime = crate::bootstrap::bootstrap(&config, &[]).unwrap();
        let handle = RuntimeHandle::from_runtime(runtime);
        McpServer::new(handle)
    }

    /// Build a wired server with the built-in filesystem port registered,
    /// so invoke_port("filesystem", ...) calls produce real success records.
    #[cfg(feature = "native-filesystem")]
    fn make_wired_server_with_filesystem() -> McpServer {
        let mut config = crate::config::SomaConfig::default();
        config.soma.data_dir = String::new();
        let runtime = crate::bootstrap::bootstrap(&config, &[]).unwrap();
        let handle = RuntimeHandle::from_runtime(runtime);
        McpServer::new(handle)
    }

    #[test]
    fn test_async_goal_returns_goal_id_without_running_to_completion() {
        let server = make_wired_server();
        let req = make_request(
            "create_goal_async",
            Some(serde_json::json!({
                "objective": "list files in /tmp",
                "max_steps": 3,
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        let result = resp.result.unwrap();
        let goal_id = result["goal_id"].as_str().unwrap().to_string();
        assert_eq!(result["status"], "pending");

        // Poll briefly — the stub runtime has no skills so the background
        // thread terminates quickly but the MCP response came back first.
        let poll = make_request(
            "get_goal_status",
            Some(serde_json::json!({ "goal_id": goal_id })),
        );
        let poll_resp = server.handle_request(poll).unwrap();
        assert!(poll_resp.error.is_none());
        let poll_result = poll_resp.result.unwrap();
        assert_eq!(poll_result["goal_id"].as_str().unwrap(), goal_id);
        assert!(poll_result["session_id"].is_string());
    }

    #[test]
    fn test_cancel_nonexistent_goal_errors() {
        let server = make_wired_server();
        let req = make_request(
            "cancel_goal",
            Some(serde_json::json!({ "goal_id": Uuid::new_v4().to_string() })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
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
        let inner = unwrap_tool_result(&result);
        assert!(inner["belief"].is_array());
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

    #[test]
    fn test_invoke_port_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({
                "port_id": "smtp",
                "capability_id": "send_plain",
                "input": { "to": "test@example.com", "subject": "test", "body": "hello" }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["port_id"], "smtp");
        assert_eq!(result["capability_id"], "send_plain");
        assert_eq!(result["success"], false);
    }

    #[test]
    fn test_invoke_port_missing_port_id() {
        let server = McpServer::new_stub();
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({ "capability_id": "send_plain" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_invoke_port_missing_capability_id() {
        let server = McpServer::new_stub();
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({ "port_id": "smtp" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_invoke_port_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("invoke_port", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_list_ports_stub() {
        let server = McpServer::new_stub();
        let req = make_request("list_ports", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["ports"].is_array());
    }

    #[test]
    fn test_unload_pack_unknown_returns_zero() {
        let server = McpServer::new_stub();
        let req = make_request(
            "unload_pack",
            Some(serde_json::json!({"pack_id": "does-not-exist"})),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        // Stub mode short-circuits to 0/0.
        assert_eq!(result["removed_skills"], serde_json::json!(0));
        assert_eq!(result["removed_ports"], serde_json::json!(0));
    }

    #[test]
    fn test_reload_pack_missing_manifest_errors() {
        let server = McpServer::new_stub();
        let req = make_request(
            "reload_pack",
            Some(serde_json::json!({"manifest_path": "/no/such/path.json"})),
        );
        let resp = server.handle_request(req).unwrap();
        // Stub mode bypasses the read; returns success with note.
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_stream_goal_observations_unknown_goal_in_stub() {
        let server = McpServer::new_stub();
        let goal_id = Uuid::new_v4().to_string();
        let req = make_request(
            "stream_goal_observations",
            Some(serde_json::json!({ "goal_id": goal_id })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["events"].is_array());
        assert_eq!(result["terminal"], serde_json::Value::Bool(true));
    }

    #[test]
    fn test_list_capabilities_stub() {
        let server = McpServer::new_stub();
        let req = make_request("list_capabilities", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["allowed"]["skills"].is_array());
        assert!(result["allowed"]["ports"].is_array());
        assert!(result["denied"]["skills"].is_array());
        assert!(result["denied"]["ports"].is_array());
    }

    #[test]
    fn test_invoke_port_via_tools_call() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "invoke_port",
                "arguments": {
                    "port_id": "smtp",
                    "capability_id": "send_plain",
                    "input": { "to": "test@example.com" }
                }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let inner = unwrap_tool_result(&result);
        assert_eq!(inner["port_id"], "smtp");
    }

    #[test]
    fn test_list_ports_via_tools_call() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "list_ports",
                "arguments": {}
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let inner = unwrap_tool_result(&result);
        assert!(inner["ports"].is_array());
    }

    #[test]
    fn test_wired_invoke_port_on_filesystem() {
        let config = crate::config::SomaConfig::default();
        let runtime = crate::bootstrap::bootstrap(&config, &[]).unwrap();
        let handle = RuntimeHandle::from_runtime(runtime);
        let server = McpServer::new(handle);
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({
                "port_id": "nonexistent",
                "capability_id": "read",
                "input": {}
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["success"], false);
    }

    #[test]
    fn test_wired_list_ports_empty() {
        let config = crate::config::SomaConfig::default();
        let runtime = crate::bootstrap::bootstrap(&config, &[]).unwrap();
        let handle = RuntimeHandle::from_runtime(runtime);
        let server = McpServer::new(handle);
        let req = make_request("list_ports", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["ports"].is_array());
    }

    // -----------------------------------------------------------------------
    // Implicit session tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_implicit_session_not_created_for_stub() {
        let server = McpServer::new_stub();
        // invoke_port on a stub doesn't create implicit sessions (no runtime).
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({
                "port_id": "smtp",
                "capability_id": "send_plain",
                "input": {}
            })),
        );
        let _ = server.handle_request(req).unwrap();
        let guard = server.implicit_session.lock().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn test_implicit_session_created_on_wired_invoke_port() {
        // Uses make_wired_server (no ports). The port call returns a failure
        // record, but the implicit session still tracks it.
        let server = make_wired_server();
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({
                "port_id": "filesystem",
                "capability_id": "stat",
                "input": { "path": "/tmp" }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let guard = server.implicit_session.lock().unwrap();
        assert!(guard.is_some());
        let sess = guard.as_ref().unwrap();
        assert_eq!(sess.skill_sequence.len(), 1);
        assert_eq!(sess.skill_sequence[0], ("filesystem".to_string(), "stat".to_string()));
    }

    #[test]
    fn test_implicit_session_accumulates_calls() {
        let server = make_wired_server();
        for cap in &["stat", "readdir"] {
            let req = make_request(
                "invoke_port",
                Some(serde_json::json!({
                    "port_id": "filesystem",
                    "capability_id": cap,
                    "input": { "path": "/tmp" }
                })),
            );
            let resp = server.handle_request(req).unwrap();
            assert!(resp.error.is_none());
        }
        let guard = server.implicit_session.lock().unwrap();
        let sess = guard.as_ref().unwrap();
        assert_eq!(sess.skill_sequence.len(), 2);
        assert_eq!(sess.observations.len(), 2);
    }

    #[test]
    fn test_implicit_session_flushed_on_non_invoke_port() {
        let server = make_wired_server();
        for cap in &["stat", "readdir"] {
            let req = make_request(
                "invoke_port",
                Some(serde_json::json!({
                    "port_id": "filesystem",
                    "capability_id": cap,
                    "input": { "path": "/tmp" }
                })),
            );
            server.handle_request(req).unwrap();
        }
        {
            let guard = server.implicit_session.lock().unwrap();
            assert!(guard.is_some());
            assert_eq!(guard.as_ref().unwrap().skill_sequence.len(), 2);
        }
        // A non-invoke_port call flushes the session.
        let req = make_request("list_ports", None);
        server.handle_request(req).unwrap();
        let guard = server.implicit_session.lock().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn test_implicit_session_episode_stored_after_flush() {
        let server = make_wired_server();
        for cap in &["stat", "readdir"] {
            let req = make_request(
                "invoke_port",
                Some(serde_json::json!({
                    "port_id": "filesystem",
                    "capability_id": cap,
                    "input": { "path": "/tmp" }
                })),
            );
            server.handle_request(req).unwrap();
        }
        server.handle_request(make_request("list_ports", None)).unwrap();
        let rt = server.runtime.get().unwrap();
        let store = rt.episode_store.lock().unwrap();
        let episodes = store.retrieve_nearest("filesystem.stat\u{2192}filesystem.readdir", 10);
        assert!(!episodes.is_empty(), "episode should be stored after flush");
        let ep = &episodes[0];
        assert_eq!(ep.steps.len(), 2);
        assert!(ep.tags.contains(&"implicit-session".to_string()));
        assert_eq!(
            ep.goal_fingerprint,
            "filesystem.stat\u{2192}filesystem.readdir"
        );
    }

    #[test]
    fn test_implicit_session_single_call_not_stored() {
        let server = make_wired_server();
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({
                "port_id": "filesystem",
                "capability_id": "stat",
                "input": { "path": "/tmp" }
            })),
        );
        server.handle_request(req).unwrap();
        server.handle_request(make_request("list_ports", None)).unwrap();
        let rt = server.runtime.get().unwrap();
        let store = rt.episode_store.lock().unwrap();
        let episodes = store.retrieve_nearest("filesystem.stat", 10);
        assert!(episodes.is_empty(), "single-call sessions should not produce episodes");
    }

    #[test]
    fn test_implicit_session_flushed_via_tools_call_non_invoke() {
        let server = make_wired_server();
        for cap in &["stat", "readdir"] {
            let req = make_request(
                "tools/call",
                Some(serde_json::json!({
                    "name": "invoke_port",
                    "arguments": {
                        "port_id": "filesystem",
                        "capability_id": cap,
                        "input": { "path": "/tmp" }
                    }
                })),
            );
            server.handle_request(req).unwrap();
        }
        {
            let guard = server.implicit_session.lock().unwrap();
            assert!(guard.is_some());
            assert_eq!(guard.as_ref().unwrap().skill_sequence.len(), 2);
        }
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "list_ports",
                "arguments": {}
            })),
        );
        server.handle_request(req).unwrap();
        let guard = server.implicit_session.lock().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn test_implicit_session_not_flushed_by_invoke_port() {
        let server = make_wired_server();
        let req = make_request(
            "invoke_port",
            Some(serde_json::json!({
                "port_id": "filesystem",
                "capability_id": "stat",
                "input": { "path": "/tmp" }
            })),
        );
        server.handle_request(req).unwrap();
        let req2 = make_request(
            "invoke_port",
            Some(serde_json::json!({
                "port_id": "filesystem",
                "capability_id": "readdir",
                "input": { "path": "/tmp" }
            })),
        );
        server.handle_request(req2).unwrap();
        let guard = server.implicit_session.lock().unwrap();
        assert!(guard.is_some());
        assert_eq!(guard.as_ref().unwrap().skill_sequence.len(), 2);
    }

    /// Full structural validation of an implicit session episode with a real
    /// filesystem port that produces success=true records.
    #[test]
    #[cfg(feature = "native-filesystem")]
    fn test_implicit_session_episode_has_correct_structure() {
        let server = make_wired_server_with_filesystem();
        for cap in &["stat", "readdir", "stat"] {
            let req = make_request(
                "invoke_port",
                Some(serde_json::json!({
                    "port_id": "filesystem",
                    "capability_id": cap,
                    "input": { "path": "/tmp" }
                })),
            );
            server.handle_request(req).unwrap();
        }
        // Flush.
        server.handle_request(make_request("query_metrics", None)).unwrap();

        let rt = server.runtime.get().unwrap();
        let store = rt.episode_store.lock().unwrap();
        let fp = "filesystem.stat\u{2192}filesystem.readdir\u{2192}filesystem.stat";
        let episodes = store.retrieve_nearest(fp, 10);
        assert_eq!(episodes.len(), 1);
        let ep = &episodes[0];
        assert_eq!(ep.steps.len(), 3);
        assert!(ep.success);
        assert_eq!(ep.outcome, crate::types::episode::EpisodeOutcome::Success);
        assert_eq!(ep.steps[0].selected_skill, "filesystem.stat");
        assert_eq!(ep.steps[1].selected_skill, "filesystem.readdir");
        assert_eq!(ep.steps[2].selected_skill, "filesystem.stat");
        assert!(ep.embedding.is_some());
        assert_eq!(ep.goal_fingerprint, fp);
    }

    #[test]
    fn test_sync_beliefs_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "sync_beliefs",
            Some(serde_json::json!({ "peer_id": "peer-0" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["outcome"], "merged");
        assert_eq!(result["peer_id"], "peer-0");
        assert!(result.get("note").is_some());
    }

    #[test]
    fn test_sync_beliefs_missing_peer_id() {
        let server = McpServer::new_stub();
        let req = make_request("sync_beliefs", Some(serde_json::json!({})));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_sync_beliefs_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("sync_beliefs", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_migrate_session_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "migrate_session",
            Some(serde_json::json!({
                "session_id": "550e8400-e29b-41d4-a716-446655440000",
                "peer_id": "peer-0"
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["outcome"], "failure");
        assert!(result.get("note").is_some());
    }

    #[test]
    fn test_migrate_session_missing_session_id() {
        let server = McpServer::new_stub();
        let req = make_request(
            "migrate_session",
            Some(serde_json::json!({ "peer_id": "peer-0" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_migrate_session_missing_peer_id() {
        let server = McpServer::new_stub();
        let req = make_request(
            "migrate_session",
            Some(serde_json::json!({
                "session_id": "550e8400-e29b-41d4-a716-446655440000"
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_migrate_session_invalid_uuid() {
        let server = McpServer::new_stub();
        let req = make_request(
            "migrate_session",
            Some(serde_json::json!({
                "session_id": "not-a-uuid",
                "peer_id": "peer-0"
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_migrate_session_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("migrate_session", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_sync_beliefs_tools_call() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "sync_beliefs",
                "arguments": { "peer_id": "peer-1" }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let inner = unwrap_tool_result(&result);
        assert_eq!(inner["outcome"], "merged");
        assert_eq!(inner["peer_id"], "peer-1");
    }

    #[test]
    fn test_migrate_session_tools_call() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "migrate_session",
                "arguments": {
                    "session_id": "550e8400-e29b-41d4-a716-446655440000",
                    "peer_id": "peer-1"
                }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let inner = unwrap_tool_result(&result);
        assert_eq!(inner["outcome"], "failure");
    }

    #[test]
    fn test_review_routine_stub() {
        let server = McpServer::new_stub();
        let req = make_request(
            "review_routine",
            Some(serde_json::json!({ "routine_id": "test-routine-1" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["routine_id"], "test-routine-1");
        assert_eq!(result["safety"], "unknown");
        assert_eq!(result["recommendation"], "needs_review");
    }

    #[test]
    fn test_review_routine_missing_id() {
        let server = McpServer::new_stub();
        let req = make_request("review_routine", Some(serde_json::json!({})));
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_review_routine_no_params() {
        let server = McpServer::new_stub();
        let req = make_request("review_routine", None);
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn test_review_routine_via_tools_call() {
        let server = McpServer::new_stub();
        let req = make_request(
            "tools/call",
            Some(serde_json::json!({
                "name": "review_routine",
                "arguments": { "routine_id": "test-routine-1" }
            })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let inner = unwrap_tool_result(&result);
        assert_eq!(inner["routine_id"], "test-routine-1");
        assert_eq!(inner["safety"], "unknown");
        assert_eq!(inner["recommendation"], "needs_review");
    }

    #[test]
    fn test_review_routine_wired_not_found() {
        let server = make_wired_server();
        let req = make_request(
            "review_routine",
            Some(serde_json::json!({ "routine_id": "nonexistent" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("routine not found"));
    }

    #[test]
    fn test_review_routine_wired_with_routine() {
        let server = make_wired_server();
        // Register a routine into the store.
        {
            let rt = server.runtime.get().unwrap();
            let mut rs = rt.routine_store.lock().unwrap();
            rs.register(crate::types::routine::Routine {
                routine_id: "review-test".to_string(),
                namespace: "test".to_string(),
                origin: crate::types::routine::RoutineOrigin::PackAuthored,
                match_conditions: vec![crate::types::common::Precondition {
                    condition_type: "goal_fingerprint".to_string(),
                    expression: serde_json::json!({"goal": "test"}),
                    description: "matches test goals".to_string(),
                }],
                compiled_skill_path: vec!["skill_a".to_string(), "skill_b".to_string()],
                compiled_steps: vec![],
                guard_conditions: vec![],
                expected_cost: 0.5,
                expected_effect: vec![],
                confidence: 0.85,
                autonomous: false,
                priority: 1,
                exclusive: false,
                policy_scope: None,
                version: 0,
            })
            .unwrap();
        }

        let req = make_request(
            "review_routine",
            Some(serde_json::json!({ "routine_id": "review-test" })),
        );
        let resp = server.handle_request(req).unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["routine_id"], "review-test");
        assert_eq!(result["version"], 0);
        assert_eq!(result["origin"], "PackAuthored");
        assert_eq!(result["confidence"], 0.85);
        assert_eq!(result["autonomous"], false);
        assert_eq!(result["priority"], 1);
        assert!(result["steps"].is_array());
        assert_eq!(result["steps"].as_array().unwrap().len(), 2);
        assert!(result["all_skill_ids"].is_array());
        assert_eq!(result["all_skill_ids"].as_array().unwrap().len(), 2);
        // Skills not in registry, so they show as unknown.
        assert_eq!(result["safety"], "unknown");
        assert_eq!(result["recommendation"], "needs_review");
        assert!(result["unknown_skills"].as_array().unwrap().len() > 0);
    }
}
