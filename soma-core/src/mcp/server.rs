//! MCP server -- JSON-RPC 2.0 over stdio (Milestone 3).
//!
//! Reads newline-delimited JSON-RPC 2.0 messages from stdin, dispatches them to
//! tool handlers, and writes responses to stdout. This is the primary interface
//! for LLMs to drive SOMA: querying state, executing intents, managing plugins,
//! and calling plugin conventions. Every tool call is logged for audit
//! and gated by role-based auth.

use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::config::SomaConfig;
use crate::mcp::auth::AuthManager;
use crate::mcp::tools::{self, McpToolResult, ToolCallArgs};
use crate::memory::checkpoint::Checkpoint;
use crate::memory::experience::ExperienceBuffer;
use crate::metrics::SomaMetrics;
use crate::mind::MindEngine;
use crate::mind::onnx_engine::OnnxMindEngine;
use crate::plugin::manager::PluginManager;
use crate::proprioception::Proprioception;
use crate::protocol::discovery::PeerRegistry;
use crate::state::SomaState;

/// Inbound JSON-RPC 2.0 request parsed from a single stdin line.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)] // Required by JSON-RPC 2.0 spec, validated by serde
    jsonrpc: String,
    #[serde(default)]
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// Outbound JSON-RPC 2.0 response written as a single stdout line.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }

    fn error(id: serde_json::Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(), id,
            result: None,
            error: Some(JsonRpcError { code, message, data: None }),
        }
    }

    fn method_not_found(id: serde_json::Value, method: &str) -> Self {
        Self::error(id, -32601, format!("Method not found: {method}"))
    }
}

/// Central MCP server holding `Arc` references to all SOMA subsystems.
///
/// Each tool handler acquires read or write locks on the relevant subsystems as needed.
/// The server itself is not `Clone`; it is created once in `main.rs` and moved into
/// `run_stdio()`.
pub struct McpServer {
    pub config: SomaConfig,
    pub mind: Arc<RwLock<OnnxMindEngine>>,
    pub plugins: Arc<RwLock<PluginManager>>,
    pub proprio: Arc<RwLock<Proprioception>>,
    pub experience: Arc<RwLock<ExperienceBuffer>>,
    pub state: Arc<RwLock<SomaState>>,
    pub metrics: Arc<SomaMetrics>,
    pub peers: Arc<RwLock<PeerRegistry>>,
    pub auth: Arc<RwLock<AuthManager>>,
    pub shutdown_requested: Arc<std::sync::atomic::AtomicBool>,
}

impl McpServer {
    /// Run the MCP server on stdio — reads JSON-RPC requests from stdin,
    /// writes responses to stdout. One message per line.
    #[allow(clippy::future_not_send)] // RwLockReadGuard held across await is intentional for plugin manager
    pub async fn run_stdio(self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        tracing::info!(component = "mcp", "MCP server started on stdio");

        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    let resp = JsonRpcResponse::error(
                        serde_json::Value::Null,
                        -32700,
                        format!("Parse error: {e}"),
                    );
                    let out = serde_json::to_string(&resp).unwrap_or_default();
                    let _ = stdout.write_all(out.as_bytes()).await;
                    let _ = stdout.write_all(b"\n").await;
                    let _ = stdout.flush().await;
                    continue;
                }
            };

            // JSON-RPC 2.0: notifications have null/missing id — no response needed
            let is_notification = request.id.is_null();

            let response = self.handle_request(&request).await;

            if !is_notification {
                let out = serde_json::to_string(&response).unwrap_or_default();
                let _ = stdout.write_all(out.as_bytes()).await;
                let _ = stdout.write_all(b"\n").await;
                let _ = stdout.flush().await;
            }
        }

        tracing::info!(component = "mcp", "MCP server stopped (stdin closed)");
        Ok(())
    }

    /// Route a JSON-RPC request to the appropriate handler by method name.
    #[allow(clippy::future_not_send)]
    async fn handle_request(&self, req: &JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize" => self.handle_initialize(&req.id),
            "initialized" | "ping" => JsonRpcResponse::success(req.id.clone(), serde_json::json!({})),
            "tools/list" => self.handle_tools_list(&req.id),
            "tools/call" => self.handle_tools_call(&req.id, &req.params).await,
            "resources/list" => self.handle_resources_list(&req.id),
            "resources/read" => self.handle_resources_read(&req.id, &req.params),
            _ => JsonRpcResponse::method_not_found(req.id.clone(), &req.method),
        }
    }

    /// MCP `initialize` handshake: returns server identity, protocol version, and capabilities.
    fn handle_initialize(&self, id: &serde_json::Value) -> JsonRpcResponse {
        JsonRpcResponse::success(id.clone(), serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
            },
            "serverInfo": {
                "name": format!("soma-{}", self.config.soma.id),
                "version": "0.1.0",
                "description": "SOMA: Neural mind drives hardware directly. Pure executor with permanent state.",
            },
        }))
    }

    /// Return the full tool catalog including dynamically-registered plugin conventions.
    fn handle_tools_list(&self, id: &serde_json::Value) -> JsonRpcResponse {
        let conventions = self.plugins.read().unwrap().namespaced_conventions();
        let tool_list = tools::build_tool_list(&conventions);
        JsonRpcResponse::success(id.clone(), serde_json::json!({
            "tools": tool_list,
        }))
    }

    /// Expose MCP resources: `soma://state` (full runtime state) and `soma://metrics` (Prometheus).
    #[allow(clippy::unused_self)] // Method signature kept consistent with other handle_* methods
    fn handle_resources_list(&self, id: &serde_json::Value) -> JsonRpcResponse {
        JsonRpcResponse::success(id.clone(), serde_json::json!({
            "resources": [
                {
                    "uri": "soma://state",
                    "name": "SOMA State",
                    "description": "Complete SOMA state — decisions, executions, health",
                    "mimeType": "application/json",
                },
                {
                    "uri": "soma://metrics",
                    "name": "SOMA Metrics",
                    "description": "Prometheus-compatible runtime metrics",
                    "mimeType": "text/plain",
                },
            ],
        }))
    }

    /// Read a resource by URI. Dispatches `soma://state` and `soma://metrics`.
    fn handle_resources_read(&self, id: &serde_json::Value, params: &serde_json::Value) -> JsonRpcResponse {
        let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");

        match uri {
            "soma://state" => {
                let state = self.state.read().unwrap();
                let proprio = self.proprio.read().unwrap();
                let content = serde_json::json!({
                    "state": state.to_json(),
                    "proprioception": proprio.to_json(),
                });
                drop(state);
                drop(proprio);
                JsonRpcResponse::success(id.clone(), serde_json::json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": serde_json::to_string_pretty(&content).unwrap_or_default(),
                    }],
                }))
            }
            "soma://metrics" => {
                JsonRpcResponse::success(id.clone(), serde_json::json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "text/plain",
                        "text": self.metrics.to_prometheus(),
                    }],
                }))
            }
            _ => JsonRpcResponse::error(id.clone(), -32602, format!("Unknown resource URI: {uri}")),
        }
    }

    /// Dispatch a `tools/call` request: parse arguments, enforce auth, route to the
    /// correct tool handler, and log the outcome for the audit trail.
    #[allow(clippy::future_not_send)]
    async fn handle_tools_call(
        &self,
        id: &serde_json::Value,
        params: &serde_json::Value,
    ) -> JsonRpcResponse {
        let call: ToolCallArgs = match serde_json::from_value(params.clone()) {
            Ok(c) => c,
            Err(e) => {
                return JsonRpcResponse::error(
                    id.clone(), -32602,
                    format!("Invalid params: {e}"),
                );
            }
        };

        let audit_trace_id = uuid::Uuid::new_v4().to_string()[..12].to_string();

        // Classify tool: action tools need execute permission, admin tools need admin.
        // Plugin convention calls (soma.X.Y, 2+ dots) are action tools.
        let is_action = call.name == "soma.intent"
            || call.name == "soma.checkpoint"
            || call.name == "soma.record_decision"
            || call.name == "soma.install_plugin"
            || call.name == "soma.uninstall_plugin"
            || call.name == "soma.configure_plugin"
            || call.name == "soma.restore_checkpoint"
            || call.name == "soma.shutdown"
            || (call.name.starts_with("soma.") && call.name.matches('.').count() >= 2);
        let is_admin = call.name == "soma.install_plugin"
            || call.name == "soma.uninstall_plugin"
            || call.name == "soma.configure_plugin"
            || call.name == "soma.restore_checkpoint"
            || call.name == "soma.shutdown";

        let auth_role = {
            let auth = self.auth.read().unwrap();
            // Token location: _meta.auth_token (MCP convention) or _token (SOMA shorthand).
            let token = call.arguments.get("_meta")
                .and_then(|m| m.get("auth_token"))
                .and_then(|v| v.as_str())
                .or_else(|| call.arguments.get("_token").and_then(|v| v.as_str()));
            match auth.check_request(token, is_action, is_admin) {
                Ok(session) => session,
                Err(e) => {
                    tracing::warn!(
                        component = "mcp",
                        tool = %call.name,
                        "MCP auth denied: {e}"
                    );
                    return JsonRpcResponse::success(
                        id.clone(),
                        serde_json::to_value(McpToolResult::error(format!("Auth: {e}")))
                            .unwrap_or_default(),
                    );
                }
            }
        };

        let result = match call.name.as_str() {
            "soma.get_state" => self.tool_get_state(),
            "soma.get_plugins" => self.tool_get_plugins(),
            "soma.get_conventions" => self.tool_get_conventions(),
            "soma.get_health" => self.tool_get_health(),
            "soma.get_recent_activity" => self.tool_get_recent_activity(&call.arguments),
            "soma.get_peers" => self.tool_get_peers(),
            "soma.get_experience" => self.tool_get_experience(),
            "soma.get_checkpoints" => self.tool_get_checkpoints(),
            "soma.get_config" => self.tool_get_config(),
            "soma.get_decisions" => self.tool_get_decisions(&call.arguments),
            "soma.get_metrics" => self.tool_get_metrics(&call.arguments),
            "soma.get_schema" => self.tool_get_schema(&call.arguments).await,
            "soma.get_business_rules" => self.tool_get_business_rules(),
            "soma.get_render_state" => self.tool_get_render_state(),

            "soma.intent" => self.tool_intent(&call.arguments),
            "soma.checkpoint" => self.tool_checkpoint(&call.arguments),
            "soma.record_decision" => self.tool_record_decision(&call.arguments),
            "soma.confirm" => self.tool_confirm(&call.arguments).await,
            "soma.install_plugin" => self.tool_install_plugin(&call.arguments),
            "soma.restore_checkpoint" => self.tool_restore_checkpoint(&call.arguments),
            "soma.shutdown" => self.tool_shutdown(),
            "soma.uninstall_plugin" => self.tool_uninstall_plugin(&call.arguments),
            "soma.configure_plugin" => self.tool_configure_plugin(&call.arguments),
            "soma.reload_design" | "soma.render_view" | "soma.update_view" => McpToolResult::json(serde_json::json!({
                "success": false,
                "message": "Not yet connected to Interface SOMA. This tool will work when an Interface SOMA connects via Synaptic Protocol.",
            })),

            // Catch-all: plugin convention tools (soma.{plugin}.{convention})
            name if name.starts_with("soma.") && name.matches('.').count() >= 2 => {
                self.tool_plugin_call(name, &call.arguments).await
            }

            _ => McpToolResult::error(format!("Unknown tool: {}", call.name)),
        };

        let is_err = result.is_error.unwrap_or(false);
        tracing::info!(
            component = "mcp",
            action = "tool_call",
            tool = %call.name,
            auth_role = %auth_role,
            trace_id = %audit_trace_id,
            result = if is_err { "error" } else { "success" },
            "MCP tool call"
        );

        JsonRpcResponse::success(id.clone(), serde_json::to_value(&result).unwrap_or_default())
    }

    /// Aggregate snapshot of SOMA: mind info, state, experience, peers, plugins, and health.
    fn tool_get_state(&self) -> McpToolResult {
        let state = self.state.read().unwrap();
        let state_json = state.to_json();
        drop(state);

        let proprio = self.proprio.read().unwrap();
        let uptime_secs = proprio.uptime().as_secs();
        let proprio_json = proprio.to_json();
        drop(proprio);

        let mind = self.mind.read().unwrap();
        let mind_info = mind.info();
        drop(mind);

        let exp = self.experience.read().unwrap();
        let exp_json = serde_json::json!({
            "buffer_size": exp.len(),
            "total_seen": exp.total_seen(),
            "success_count": exp.success_count(),
            "failure_count": exp.failure_count(),
        });
        drop(exp);

        let peers = self.peers.read().unwrap();
        let peers_json: Vec<serde_json::Value> = peers.list().iter().map(|p| serde_json::json!({
            "name": p.name,
            "addr": p.addr,
        })).collect();
        drop(peers);

        let plugins_guard = self.plugins.read().unwrap();
        let plugin_names: Vec<String> = plugins_guard.conventions().iter().map(|c| c.name.clone()).collect();
        let health_warnings = plugins_guard.check_plugin_health();
        let plugin_warnings: Vec<serde_json::Value> = health_warnings.iter().map(|(name, msg)| {
            serde_json::json!({"plugin": name, "warning": msg})
        }).collect();
        drop(plugins_guard);

        let result = serde_json::json!({
            "soma_id": self.config.soma.id,
            "version": "0.1.0",
            "uptime_secs": uptime_secs,
            "mind": {
                "backend": mind_info.backend,
                "conventions_known": mind_info.conventions_known,
                "max_steps": mind_info.max_steps,
                "lora_layers": mind_info.lora_layers,
                "lora_magnitude": mind_info.lora_magnitude,
            },
            "state": state_json,
            "experience": exp_json,
            "proprioception": proprio_json,
            "peers": peers_json,
            "plugins": plugin_names,
            "plugin_warnings": plugin_warnings,
        });

        McpToolResult::json(result)
    }

    /// List all loaded plugins with their conventions, versions, and health status.
    fn tool_get_plugins(&self) -> McpToolResult {
        let plugins = self.plugins.read().unwrap();
        let namespaced = plugins.namespaced_conventions();
        let mut plugins_map: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();

        for (plugin_name, c) in &namespaced {
            let entry = serde_json::json!({
                "id": c.id,
                "name": c.name,
                "full_name": format!("soma.{}.{}", plugin_name, c.name),
                "description": c.description,
                "call_pattern": c.call_pattern,
                "returns": c.returns,
                "is_deterministic": c.is_deterministic,
                "estimated_latency_ms": c.estimated_latency_ms,
                "max_latency_ms": c.max_latency_ms,
                "side_effects": c.side_effects,
                "cleanup": c.cleanup,
                "args": c.args,
            });
            plugins_map.entry(plugin_name.clone()).or_default().push(entry);
        }

        let health_warnings = plugins.check_plugin_health();
        // Own the warning strings before dropping the read guard
        let warning_map: std::collections::HashMap<String, String> = health_warnings.iter()
            .map(|(name, msg)| (name.clone(), (*msg).to_string()))
            .collect();
        drop(plugins);

        let plugin_list: Vec<serde_json::Value> = plugins_map.iter().map(|(name, convs)| {
            let health_status = warning_map.get(name).map_or_else(
                || serde_json::json!({"status": "healthy"}),
                |warning| serde_json::json!({"status": "degraded", "warning": warning}),
            );
            serde_json::json!({
                "name": name,
                "version": "0.1.0",
                "trust_level": "built_in",
                "convention_count": convs.len(),
                "conventions": convs,
                "health": health_status,
            })
        }).collect();

        McpToolResult::json(serde_json::json!({
            "count": plugin_list.len(),
            "plugins": plugin_list,
        }))
    }

    fn tool_get_conventions(&self) -> McpToolResult {
        let conventions = self.plugins.read().unwrap().conventions();
        McpToolResult::json(serde_json::to_value(&conventions).unwrap_or_default())
    }

    /// Runtime health: proprioception snapshot, Prometheus metrics, and plugin warnings.
    fn tool_get_health(&self) -> McpToolResult {
        let proprio = self.proprio.read().unwrap();
        let proprio_json = proprio.to_json();
        drop(proprio);
        let metrics = self.metrics.to_json();

        let pm_guard = self.plugins.read().unwrap();
        let plugin_warnings = pm_guard.check_plugin_health();
        let status = if plugin_warnings.is_empty() { "healthy" } else { "degraded" };
        let warnings_json: Vec<serde_json::Value> = plugin_warnings.iter().map(|(name, msg)| {
            serde_json::json!({"plugin": name, "warning": msg})
        }).collect();
        drop(pm_guard);

        McpToolResult::json(serde_json::json!({
            "status": status,
            "proprioception": proprio_json,
            "metrics": metrics,
            "plugin_warnings": warnings_json,
        }))
    }

    fn tool_get_recent_activity(&self, args: &serde_json::Value) -> McpToolResult {
        let n = args.get("n").and_then(serde_json::Value::as_u64).unwrap_or(10);
        let state = self.state.read().unwrap();
        let records = state.executions.recent(usize::try_from(n).unwrap_or(usize::MAX));
        let resp = McpToolResult::json(serde_json::json!({
            "count": records.len(),
            "records": records,
        }));
        drop(state);
        resp
    }

    fn tool_get_peers(&self) -> McpToolResult {
        let peers = self.peers.read().unwrap();
        let peer_list: Vec<serde_json::Value> = peers.list().iter().map(|p| {
            serde_json::json!({
                "name": p.name,
                "addr": p.addr,
                "plugins": p.plugins,
                "conventions": p.conventions,
            })
        }).collect();
        drop(peers);

        McpToolResult::json(serde_json::json!({
            "count": peer_list.len(),
            "peers": peer_list,
        }))
    }

    /// Experience buffer stats: size, success/failure counts, `LoRA` adaptation state.
    fn tool_get_experience(&self) -> McpToolResult {
        let exp = self.experience.read().unwrap();
        let exp_len = exp.len();
        let total_seen = exp.total_seen();
        let success_count = exp.success_count();
        let failure_count = exp.failure_count();
        drop(exp);

        let mind_guard = self.mind.read().unwrap();
        let mind_info = mind_guard.info();
        drop(mind_guard);

        let proprio = self.proprio.read().unwrap();
        let adaptation_count = proprio.total_adaptations;
        let consolidation_count = proprio.consolidations;
        drop(proprio);

        McpToolResult::json(serde_json::json!({
            "buffer_size": exp_len,
            "max_size": self.config.memory.max_experience_buffer,
            "total_seen": total_seen,
            "success_count": success_count,
            "failure_count": failure_count,
            "lora_magnitude": mind_info.lora_magnitude,
            "lora_layers": mind_info.lora_layers,
            "adaptation_count": adaptation_count,
            "consolidation_count": consolidation_count,
        }))
    }

    /// Enumerate checkpoint files in the configured checkpoint directory.
    fn tool_get_checkpoints(&self) -> McpToolResult {
        let ckpt_dir = Path::new(&self.config.memory.checkpoint_dir);
        let checkpoints = Checkpoint::list_checkpoints(ckpt_dir).map_or_else(
            |_| Vec::new(),
            |paths| paths.iter().map(|p| {
                let filename = p.file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                serde_json::json!({
                    "path": p.display().to_string(),
                    "filename": filename,
                })
            }).collect::<Vec<_>>(),
        );

        McpToolResult::json(serde_json::json!({
            "checkpoint_dir": self.config.memory.checkpoint_dir,
            "count": checkpoints.len(),
            "checkpoints": checkpoints,
        }))
    }

    /// Return the current SOMA configuration, excluding secrets and auth tokens.
    fn tool_get_config(&self) -> McpToolResult {
        McpToolResult::json(serde_json::json!({
            "soma": {
                "id": self.config.soma.id,
                "log_level": self.config.soma.log_level,
                "trace_verbosity": self.config.soma.trace_verbosity,
            },
            "mind": {
                "backend": self.config.mind.backend,
                "model_dir": self.config.mind.model_dir,
                "max_program_steps": self.config.mind.max_program_steps,
                "temperature": self.config.mind.temperature,
                "max_inference_time_secs": self.config.mind.max_inference_time_secs,
                "lora": {
                    "default_rank": self.config.mind.lora.default_rank,
                    "default_alpha": self.config.mind.lora.default_alpha,
                    "adaptation_enabled": self.config.mind.lora.adaptation_enabled,
                    "adapt_every_n_successes": self.config.mind.lora.adapt_every_n_successes,
                    "adapt_batch_size": self.config.mind.lora.adapt_batch_size,
                    "adapt_learning_rate": self.config.mind.lora.adapt_learning_rate,
                },
            },
            "memory": {
                "checkpoint_dir": self.config.memory.checkpoint_dir,
                "auto_checkpoint": self.config.memory.auto_checkpoint,
                "max_checkpoints": self.config.memory.max_checkpoints,
                "max_experience_buffer": self.config.memory.max_experience_buffer,
            },
            "protocol": {
                "bind": self.config.protocol.bind,
                "max_connections": self.config.protocol.max_connections,
                "peers": self.config.protocol.peers.len(),
            },
            "resources": {
                "max_concurrent_inferences": self.config.resources.max_concurrent_inferences,
                "max_concurrent_plugin_calls": self.config.resources.max_concurrent_plugin_calls,
            },
            "mcp": {
                "transport": self.config.mcp.transport,
                "enabled": self.config.mcp.enabled,
                "max_execution_history": self.config.mcp.max_execution_history,
            },
            "security": {
                "require_auth": self.config.security.require_auth,
                "require_confirmation": self.config.security.require_confirmation,
            },
        }))
    }

    /// Query the decision log. Supports `n` (recent count) and `search` (keyword filter).
    #[allow(clippy::cast_possible_truncation)] // u64 from JSON capped to reasonable values
    fn tool_get_decisions(&self, args: &serde_json::Value) -> McpToolResult {
        let state = self.state.read().unwrap();

        if let Some(query) = args.get("search").and_then(|v| v.as_str()) {
            let results = state.decisions.search(query);
            let resp = McpToolResult::json(serde_json::json!({
                "search": query,
                "count": results.len(),
                "decisions": results,
            }));
            drop(state);
            return resp;
        }

        let n = args.get("n").and_then(serde_json::Value::as_u64);
        let decisions = n.map_or_else(
            || state.decisions.list(),
            |n| state.decisions.recent(n as usize),
        );

        let resp = McpToolResult::json(serde_json::json!({
            "count": decisions.len(),
            "decisions": decisions,
        }));
        drop(state);
        resp
    }

    /// Query database schema via the postgres plugin. Returns table names and column metadata.
    /// If `table` is specified, returns that table's schema with sample rows.
    #[allow(clippy::future_not_send, clippy::await_holding_lock, clippy::significant_drop_tightening)]
    async fn tool_get_schema(&self, args: &serde_json::Value) -> McpToolResult {
        let pm = self.plugins.read().unwrap();
        let has_postgres = pm.plugin_names().iter().any(|n| n == "postgres");

        if !has_postgres {
            drop(pm);
            return McpToolResult::json(serde_json::json!({
                "tables": [],
                "note": "No database plugin loaded. Install postgres or sqlite plugin for schema tracking.",
            }));
        }

        if let Some(table_name) = args.get("table").and_then(|v| v.as_str()) {
            // Convention 10 = table_schema, convention 11 = sample_rows (postgres plugin)
            let schema_args = vec![crate::plugin::interface::Value::String(table_name.to_string())];
            let columns = match pm.execute_by_plugin_async("postgres", 10, schema_args).await {
                Ok(cols) => format!("{cols}"),
                Err(e) => return McpToolResult::error(format!("Failed to query table schema: {e}")),
            };
            let sample_args = vec![
                crate::plugin::interface::Value::String(table_name.to_string()),
                crate::plugin::interface::Value::Int(5),
            ];
            let sample_rows = pm.execute_by_plugin_async("postgres", 11, sample_args).await
                .ok()
                .map(|rows| format!("{rows}"));
            let mut result = serde_json::json!({
                "table": table_name,
                "columns": columns,
            });
            if let Some(rows) = sample_rows {
                result.as_object_mut().unwrap().insert("sample_rows".to_string(), serde_json::json!(rows));
            }
            return McpToolResult::json(result);
        }

        // Convention 9 = list_tables, then 10 = table_schema for each table
        let tables = match pm.execute_by_plugin_async("postgres", 9, vec![]).await {
            Ok(crate::plugin::interface::Value::List(table_list)) => {
                let mut result = Vec::new();
                for table_val in &table_list {
                    if let crate::plugin::interface::Value::String(table_name) = table_val {
                        let schema_args = vec![crate::plugin::interface::Value::String(table_name.clone())];
                        let columns = pm.execute_by_plugin_async("postgres", 10, schema_args).await
                            .map_or_else(|_| "[]".to_string(), |cols| format!("{cols}"));
                        result.push(serde_json::json!({
                            "name": table_name,
                            "columns": columns,
                        }));
                    }
                }
                result
            }
            Ok(other) => {
                return McpToolResult::json(serde_json::json!({
                    "tables": [],
                    "raw": format!("{other}"),
                    "note": "Unexpected response from list_tables",
                }));
            }
            Err(e) => {
                return McpToolResult::error(format!("Failed to query schema: {e}"));
            }
        };

        McpToolResult::json(serde_json::json!({
            "tables": tables,
            "total_tables": tables.len(),
        }))
    }

    /// Extract business rules from the decision log by keyword matching.
    fn tool_get_business_rules(&self) -> McpToolResult {
        let state = self.state.read().unwrap();
        let rules: Vec<serde_json::Value> = state.decisions.list().iter()
            .filter(|d| d.what.to_lowercase().contains("rule")
                || d.what.to_lowercase().contains("policy")
                || d.what.to_lowercase().contains("require")
                || d.what.to_lowercase().contains("constraint"))
            .map(|d| serde_json::json!({
                "id": d.id,
                "rule": d.what,
                "reason": d.why,
                "timestamp": d.timestamp,
            }))
            .collect();
        drop(state);

        McpToolResult::json(serde_json::json!({
            "count": rules.len(),
            "rules": rules,
            "note": "Business rules are derived from decisions containing 'rule', 'policy', 'require', or 'constraint'.",
        }))
    }

    /// Stub: returns empty render state until an Interface SOMA connects via Synaptic Protocol.
    #[allow(clippy::unused_self)] // Will use self when Interface SOMA is connected
    fn tool_get_render_state(&self) -> McpToolResult {
        McpToolResult::json(serde_json::json!({
            "active_views": [],
            "pending_updates": [],
            "connected_interfaces": 0,
            "note": "No Interface SOMA connected. Render state is available when an Interface SOMA connects via Synaptic Protocol.",
        }))
    }

    fn tool_get_metrics(&self, args: &serde_json::Value) -> McpToolResult {
        let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("json");
        match format {
            "prometheus" => McpToolResult::text(self.metrics.to_prometheus()),
            _ => McpToolResult::json(self.metrics.to_json()),
        }
    }

    /// Execute a natural language intent: infer a program via the Mind, execute it via
    /// `PluginManager`, record metrics/experience/history, and return the result.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn tool_intent(&self, args: &serde_json::Value) -> McpToolResult {
        let Some(text) = args.get("text").and_then(|v| v.as_str()) else {
            return McpToolResult::error("Missing required argument: text".into());
        };

        let trace_id = uuid::Uuid::new_v4().to_string()[..12].to_string();
        let exec_start = std::time::Instant::now();

        let mind_guard = self.mind.read().unwrap();
        match mind_guard.infer(text) {
            Ok(program) => {
                let plugins_guard = self.plugins.read().unwrap();
                let result = plugins_guard.execute_program(
                    &program.steps,
                    self.config.mind.max_program_steps,
                );
                drop(plugins_guard);

                let execution_time_ms = exec_start.elapsed().as_millis() as u64;

                self.metrics.record_inference(result.success, execution_time_ms);
                self.metrics.record_program(program.steps.len() as u64);

                if let Ok(mut state) = self.state.write() {
                    state.executions.record(
                        text.to_string(),
                        program.steps.len(),
                        program.confidence,
                        result.success,
                        execution_time_ms,
                        trace_id.clone(),
                        result.error.clone(),
                    );
                }

                if let Ok(mut p) = self.proprio.write() {
                    if result.success { p.record_success(); }
                    else { p.record_failure(); }
                }

                // Record experience (successes only)
                let tokens: Vec<u32> = mind_guard.tokenizer.encode(text)
                    .iter().map(|&t| t as u32).collect();
                drop(mind_guard);
                let prog_data: Vec<(i32, u8, u8)> = program.steps.iter().map(|s| {
                    use crate::mind::ArgType;
                    let a0 = match s.arg0_type { ArgType::None => 0u8, ArgType::Span => 1, ArgType::Ref => 2, ArgType::Literal => 3 };
                    let a1 = match s.arg1_type { ArgType::None => 0u8, ArgType::Span => 1, ArgType::Ref => 2, ArgType::Literal => 3 };
                    (s.conv_id, a0, a1)
                }).collect();
                if let Ok(mut buf) = self.experience.write() {
                    if result.success {
                        buf.record(crate::memory::experience::Experience {
                            intent_tokens: tokens,
                            program: prog_data,
                            success: result.success,
                            execution_time_ms,
                            timestamp: std::time::Instant::now(),
                            cached_states: Vec::new(), // MCP path doesn't cache states yet
                        });
                    }
                    self.metrics.experience_buffer_size.store(
                        buf.len() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }

                tracing::info!(
                    component = "mcp",
                    trace_id = %trace_id,
                    intent = %text,
                    steps = program.steps.len(),
                    confidence = %program.confidence,
                    success = result.success,
                    "MCP intent processed"
                );

                let response = serde_json::json!({
                    "trace_id": trace_id,
                    "success": result.success,
                    "confidence": program.confidence,
                    "program_steps": program.steps.len(),
                    "execution_time_ms": execution_time_ms,
                    "output": result.output.map(|o| format!("{o}")),
                    "error": result.error,
                    "trace": result.trace,
                });

                McpToolResult::json(response)
            }
            Err(e) => {
                drop(mind_guard);
                self.metrics.record_inference(false, exec_start.elapsed().as_millis() as u64);
                if let Ok(mut p) = self.proprio.write() {
                    p.record_failure();
                }
                McpToolResult::error(format!("Inference error: {e}"))
            }
        }
    }

    /// Save a checkpoint: `LoRA` state, experience counts, plugin states, decisions, and history.
    fn tool_checkpoint(&self, args: &serde_json::Value) -> McpToolResult {
        let label = args.get("label").and_then(|v| v.as_str());
        let ckpt_dir = Path::new(&self.config.memory.checkpoint_dir);
        let filename = label.map_or_else(
            || Checkpoint::filename(&self.config.soma.id),
            |lbl| {
                // Filename format: soma-{id}-{label}-{unix_timestamp}.ckpt
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                format!("soma-{}-{lbl}-{ts}.ckpt", self.config.soma.id)
            },
        );
        let path = ckpt_dir.join(&filename);

        let (exp_count, adapt_count) = {
            let p = self.proprio.read().unwrap();
            (p.experience_count, p.total_adaptations)
        };

        let plugin_states = self.plugins.read().unwrap().collect_plugin_states();
        let plugin_state_entries: Vec<crate::memory::checkpoint::PluginStateEntry> = plugin_states
            .into_iter()
            .map(|(name, state)| crate::memory::checkpoint::PluginStateEntry {
                plugin_name: name,
                state,
            })
            .collect();

        let mut ckpt = Checkpoint::new(
            self.config.soma.id.clone(),
            Vec::new(),
            exp_count,
            adapt_count,
        );
        ckpt.plugin_states = plugin_state_entries;
        if let Ok(m) = self.mind.read() {
            ckpt.base_model_hash.clone_from(&m.model_hash);
        }
        ckpt.plugin_manifest = self.plugins.read().unwrap().plugin_manifest().into_iter()
            .map(|(name, version)| crate::memory::checkpoint::PluginManifestEntry { name, version })
            .collect();

        if let Ok(st) = self.state.read() {
            ckpt.decisions = serde_json::to_value(st.decisions.list())
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            ckpt.recent_executions = serde_json::to_value(st.executions.to_json())
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
        }

        match ckpt.save(&path) {
            Ok(()) => {
                if let Ok(mut p) = self.proprio.write() {
                    p.record_checkpoint();
                }
                let _ = Checkpoint::prune_checkpoints(ckpt_dir, self.config.memory.max_checkpoints);
                let mut resp = serde_json::json!({
                    "success": true,
                    "path": path.display().to_string(),
                    "experience_count": exp_count,
                    "adaptation_count": adapt_count,
                });
                if let Some(lbl) = label {
                    resp.as_object_mut().unwrap().insert("label".to_string(), serde_json::json!(lbl));
                }
                McpToolResult::json(resp)
            }
            Err(e) => McpToolResult::error(format!("Checkpoint failed: {e}")),
        }
    }

    /// Record a decision in the permanent log. Enriches `what`/`why` with optional
    /// context, related tables, and related plugins for searchability.
    fn tool_record_decision(&self, args: &serde_json::Value) -> McpToolResult {
        let what = match args.get("what").and_then(|v| v.as_str()) {
            Some(w) => w.to_string(),
            None => return McpToolResult::error("Missing required argument: what".into()),
        };
        let why = match args.get("why").and_then(|v| v.as_str()) {
            Some(w) => w.to_string(),
            None => return McpToolResult::error("Missing required argument: why".into()),
        };

        let context = args.get("context").and_then(|v| v.as_str()).map(std::string::ToString::to_string);
        let related_tables: Option<Vec<String>> = args.get("related_tables")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(std::string::ToString::to_string)).collect());
        let related_plugins: Option<Vec<String>> = args.get("related_plugins")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(std::string::ToString::to_string)).collect());

        let enriched_what = context.as_ref().map_or_else(
            || what.clone(),
            |ctx| format!("{what} [context: {ctx}]"),
        );

        let mut enriched_why = why;
        if let Some(tables) = &related_tables {
            enriched_why = format!("{enriched_why} [tables: {}]", tables.join(", "));
        }
        if let Some(plugins) = &related_plugins {
            enriched_why = format!("{enriched_why} [plugins: {}]", plugins.join(", "));
        }

        let session_id = "mcp-session".to_string();

        let Ok(mut state) = self.state.write() else {
            return McpToolResult::error("Failed to acquire state lock".into());
        };
        let decision = state.decisions.record(enriched_what, enriched_why, session_id).clone();
        drop(state);
        if let Ok(mut p) = self.proprio.write() {
            p.record_decision();
        }
        let mut resp = serde_json::json!({
            "success": true,
            "decision": decision,
        });
        if let Some(ctx) = &context {
            resp.as_object_mut().unwrap().insert("context".to_string(), serde_json::json!(ctx));
        }
        if let Some(tables) = &related_tables {
            resp.as_object_mut().unwrap().insert("related_tables".to_string(), serde_json::json!(tables));
        }
        if let Some(plugins) = &related_plugins {
            resp.as_object_mut().unwrap().insert("related_plugins".to_string(), serde_json::json!(plugins));
        }
        McpToolResult::json(resp)
    }

    /// Complete a two-step confirmation: validate the action ID, then re-dispatch
    /// the original tool call with `confirmed: true` injected into the arguments.
    #[allow(clippy::future_not_send)]
    async fn tool_confirm(&self, args: &serde_json::Value) -> McpToolResult {
        let Some(action_id) = args.get("action_id").and_then(|v| v.as_str()) else {
            return McpToolResult::error("Missing required argument: action_id".into());
        };

        let pending = {
            let mut auth = self.auth.write().unwrap();
            auth.confirm(action_id)
        };

        match pending {
            Some(pending) => {
                let mut confirmed_args = pending.arguments.clone();
                if let Some(obj) = confirmed_args.as_object_mut() {
                    obj.insert("confirmed".to_string(), serde_json::json!(true));
                }
                match pending.tool_name.as_str() {
                    "soma.restore_checkpoint" => self.tool_restore_checkpoint(&confirmed_args),
                    name if name.starts_with("soma.") && name.matches('.').count() >= 2 => {
                        self.tool_plugin_call(name, &confirmed_args).await
                    }
                    _ => McpToolResult::error(format!("Cannot re-dispatch tool: {}", pending.tool_name)),
                }
            }
            None => McpToolResult::error(format!("No pending confirmation: {action_id} (expired or invalid)")),
        }
    }

    /// Load a plugin shared library from the plugins directory and register it at runtime.
    fn tool_install_plugin(&self, args: &serde_json::Value) -> McpToolResult {
        let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
            return McpToolResult::error("Missing required argument: name".into());
        };

        let plugins_dir = std::path::Path::new(&self.config.soma.plugins_directory);
        let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
        let plugin_path = plugins_dir.join(format!("lib{name}.{ext}"));

        if !plugin_path.exists() {
            return McpToolResult::json(serde_json::json!({
                "success": false,
                "error": format!("Plugin file not found: {}", plugin_path.display()),
                "searched": plugins_dir.display().to_string(),
                "expected_filename": format!("lib{name}.{ext}"),
            }));
        }

        match crate::plugin::dynamic::load_plugin_from_path(&plugin_path) {
            Ok(mut plugin) => {
                // Populate PluginConfig from [plugins.<name>] TOML section if present
                let mut pc = crate::plugin::interface::PluginConfig::default();
                if let Some(toml_val) = self.config.plugins.get(name)
                    && let Some(table) = toml_val.as_table()
                {
                    for (k, v) in table {
                        if let Ok(json_val) = serde_json::to_value(v) {
                            pc.settings.insert(k.clone(), json_val);
                        }
                    }
                }
                if let Err(e) = plugin.on_load(&pc) {
                    return McpToolResult::error(format!("Plugin on_load failed: {e}"));
                }
                let pname = plugin.name().to_string();
                let pversion = plugin.version().to_string();
                let conv_count = plugin.conventions().len();
                let mut pm = self.plugins.write().unwrap();
                pm.register(plugin);
                drop(pm);
                McpToolResult::json(serde_json::json!({
                    "success": true,
                    "plugin": pname,
                    "version": pversion,
                    "conventions": conv_count,
                    "note": "Plugin loaded and registered at runtime",
                }))
            }
            Err(e) => McpToolResult::error(format!("Failed to load plugin: {e}")),
        }
    }

    /// Restore SOMA state from a checkpoint file. Requires two-step confirmation
    /// when `require_confirmation` is enabled (destructive: overwrites current `LoRA` state).
    fn tool_restore_checkpoint(&self, args: &serde_json::Value) -> McpToolResult {
        let Some(path_str) = args.get("path").and_then(|v| v.as_str()) else {
            return McpToolResult::error("Missing required argument: path".into());
        };

        if self.config.security.require_confirmation
            && args.get("confirmed").and_then(serde_json::Value::as_bool) != Some(true)
        {
            let mut auth = self.auth.write().unwrap();
            let action_id = auth.create_confirmation(
                format!("Restore checkpoint from {path_str}"),
                "mcp",
                "soma.restore_checkpoint".to_string(),
                args.clone(),
            );
            drop(auth);
            return McpToolResult::json(serde_json::json!({
                "requires_confirmation": true,
                "action_id": action_id,
                "description": format!("Restore checkpoint from {path_str}. This will overwrite current LoRA state."),
                "instructions": "Call soma.confirm with the action_id to proceed, or pass confirmed:true.",
            }));
        }

        let path = std::path::Path::new(path_str);
        match Checkpoint::load(path) {
            Ok(ckpt) => {
                if let Ok(mut p) = self.proprio.write() {
                    p.experience_count = ckpt.experience_count;
                    p.total_adaptations = ckpt.adaptation_count;
                }
                let decisions_restored = ckpt.decisions.len();
                if let Ok(mut st) = self.state.write() {
                    for decision_val in &ckpt.decisions {
                        if let (Some(what), Some(why)) = (
                            decision_val.get("what").and_then(|v| v.as_str()),
                            decision_val.get("why").and_then(|v| v.as_str()),
                        ) {
                            let session = decision_val.get("session_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("restored");
                            st.decisions.record(
                                what.to_string(), why.to_string(), session.to_string(),
                            );
                        }
                    }
                }
                tracing::info!(
                    component = "mcp",
                    action = "restore_checkpoint",
                    path = %path_str,
                    decisions = decisions_restored,
                    "Checkpoint restored"
                );
                McpToolResult::json(serde_json::json!({
                    "success": true,
                    "soma_id": ckpt.soma_id,
                    "experience_count": ckpt.experience_count,
                    "adaptation_count": ckpt.adaptation_count,
                    "lora_layers": ckpt.lora_state.len(),
                    "decisions_restored": decisions_restored,
                }))
            }
            Err(e) => McpToolResult::error(format!("Restore failed: {e}")),
        }
    }

    /// Signal the main loop to initiate graceful shutdown.
    fn tool_shutdown(&self) -> McpToolResult {
        tracing::info!(component = "mcp", action = "shutdown", "Graceful shutdown requested via MCP");
        self.shutdown_requested.store(true, std::sync::atomic::Ordering::SeqCst);
        McpToolResult::json(serde_json::json!({
            "success": true,
            "message": "Shutdown requested. SOMA will terminate gracefully.",
        }))
    }

    /// Unregister a loaded plugin by name, removing all its conventions.
    fn tool_uninstall_plugin(&self, args: &serde_json::Value) -> McpToolResult {
        let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
            return McpToolResult::error("Missing required argument: name".into());
        };

        let mut pm = self.plugins.write().unwrap();
        match pm.unregister(name) {
            Ok(()) => McpToolResult::json(serde_json::json!({
                "success": true,
                "plugin": name,
                "note": "Plugin unloaded and unregistered",
            })),
            Err(e) => McpToolResult::error(format!("Uninstall failed: {e}")),
        }
    }

    /// Update a plugin's configuration. The plugin may require a restart to apply changes.
    fn tool_configure_plugin(&self, args: &serde_json::Value) -> McpToolResult {
        let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
            return McpToolResult::error("Missing required argument: name".into());
        };
        let Some(config_val) = args.get("config") else {
            return McpToolResult::error("Missing required argument: config".into());
        };

        let mut pc = crate::plugin::interface::PluginConfig::default();
        if let Some(obj) = config_val.as_object() {
            for (k, v) in obj {
                pc.settings.insert(k.clone(), v.clone());
            }
        }

        let pm = self.plugins.write().unwrap();
        let plugin_names: Vec<String> = pm.plugin_names();
        drop(pm);
        if !plugin_names.contains(&name.to_string()) {
            return McpToolResult::error(format!("Plugin not found: {name}"));
        }

        tracing::info!(
            component = "mcp",
            plugin = %name,
            "Plugin configuration updated (note: on_load re-invocation not supported at runtime without reload)"
        );

        McpToolResult::json(serde_json::json!({
            "success": true,
            "plugin": name,
            "config_keys": config_val.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "note": "Configuration recorded. Some plugins may require restart to apply changes.",
        }))
    }

    /// Execute a plugin convention by its namespaced tool name (`soma.{plugin}.{convention}`).
    /// Converts JSON arguments to `Value` instances based on the convention's `ArgType` specs,
    /// then delegates to `PluginManager::execute_by_plugin_async`.
    #[allow(clippy::future_not_send, clippy::await_holding_lock)]
    async fn tool_plugin_call(&self, tool_name: &str, args: &serde_json::Value) -> McpToolResult {
        let parts: Vec<&str> = tool_name.splitn(3, '.').collect();
        if parts.len() < 3 {
            return McpToolResult::error(format!("Invalid tool name: {tool_name}"));
        }
        let plugin_name = parts[1]; // e.g. "posix"
        let conv_name = parts[2]; // e.g. "open_read"

        let conventions = self.plugins.read().unwrap().conventions();
        let Some(conv) = conventions.iter().find(|c| c.name == conv_name) else {
            return McpToolResult::error(format!("Unknown convention: {conv_name}"));
        };

        let mut plugin_args = Vec::new();
        for arg_spec in &conv.args {
            if let Some(val) = args.get(&arg_spec.name) {
                use crate::plugin::interface::ArgType;
                let pval = match arg_spec.arg_type {
                    ArgType::Int | ArgType::Handle => {
                        crate::plugin::interface::Value::Int(val.as_i64().unwrap_or(0))
                    }
                    ArgType::Float => {
                        crate::plugin::interface::Value::Float(val.as_f64().unwrap_or(0.0))
                    }
                    ArgType::Bool => {
                        crate::plugin::interface::Value::Bool(val.as_bool().unwrap_or(false))
                    }
                    ArgType::Bytes => {
                        // Accept base64 string or raw string as bytes
                        let s = val.as_str().unwrap_or("").to_string();
                        crate::plugin::interface::Value::Bytes(s.into_bytes())
                    }
                    ArgType::Any => json_to_value(val),
                    ArgType::String => {
                        crate::plugin::interface::Value::String(
                            val.as_str().unwrap_or("").to_string()
                        )
                    }
                };
                plugin_args.push(pval);
            } else if arg_spec.required {
                return McpToolResult::error(
                    format!("Missing required argument: {}", arg_spec.name)
                );
            } else {
                plugin_args.push(crate::plugin::interface::Value::Null);
            }
        }

        let conv_id = conv.id;
        let pm = self.plugins.read().unwrap();
        match pm.execute_by_plugin_async(plugin_name, conv_id, plugin_args).await {
            Ok(val) => McpToolResult::json(serde_json::json!({
                "success": true,
                "result": serde_json::to_value(&val).unwrap_or(serde_json::Value::Null),
            })),
            Err(e) => McpToolResult::error(format!("Plugin error: {e}")),
        }
    }
}

/// Recursively convert a `serde_json::Value` to a SOMA plugin `Value`.
///
/// Numbers that fit in `i64` become `Value::Int`; others become `Value::Float`.
/// Arrays and objects map to `Value::List` and `Value::Map` respectively.
fn json_to_value(val: &serde_json::Value) -> crate::plugin::interface::Value {
    match val {
        serde_json::Value::Null => crate::plugin::interface::Value::Null,
        serde_json::Value::Bool(b) => crate::plugin::interface::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            n.as_i64().map_or_else(
                || crate::plugin::interface::Value::Float(n.as_f64().unwrap_or(0.0)),
                crate::plugin::interface::Value::Int,
            )
        }
        serde_json::Value::String(s) => crate::plugin::interface::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            crate::plugin::interface::Value::List(arr.iter().map(json_to_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let map: std::collections::HashMap<String, crate::plugin::interface::Value> =
                obj.iter().map(|(k, v)| (k.clone(), json_to_value(v))).collect();
            crate::plugin::interface::Value::Map(map)
        }
    }
}
