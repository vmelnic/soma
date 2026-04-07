//! MCP Server — JSON-RPC 2.0 over stdio (Whitepaper Section 8).
//!
//! This is Milestone 3: "At this point, an LLM can drive SOMA."
//!
//! Protocol: Model Context Protocol (MCP) — JSON-RPC 2.0 messages,
//! one per line on stdin/stdout. The server exposes SOMA's state and
//! actions as MCP tools that any LLM can call.

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

/// JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// JSON-RPC 2.0 response.
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
        Self::error(id, -32601, format!("Method not found: {}", method))
    }
}

/// MCP Server — holds references to all SOMA subsystems.
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
                        format!("Parse error: {}", e),
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

    /// Handle a single JSON-RPC request.
    async fn handle_request(&self, req: &JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            // MCP lifecycle
            "initialize" => self.handle_initialize(&req.id),
            "initialized" => JsonRpcResponse::success(req.id.clone(), serde_json::json!({})),
            "ping" => JsonRpcResponse::success(req.id.clone(), serde_json::json!({})),

            // MCP tool discovery
            "tools/list" => self.handle_tools_list(&req.id),

            // MCP tool execution
            "tools/call" => self.handle_tools_call(&req.id, &req.params).await,

            // MCP resource discovery and read
            "resources/list" => self.handle_resources_list(&req.id),
            "resources/read" => self.handle_resources_read(&req.id, &req.params),

            _ => JsonRpcResponse::method_not_found(req.id.clone(), &req.method),
        }
    }

    /// Handle initialize — return server info and capabilities.
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

    /// Handle tools/list — return all available tools.
    fn handle_tools_list(&self, id: &serde_json::Value) -> JsonRpcResponse {
        let conventions = self.plugins.read().unwrap().namespaced_conventions();
        let tool_list = tools::build_tool_list(&conventions);
        JsonRpcResponse::success(id.clone(), serde_json::json!({
            "tools": tool_list,
        }))
    }

    /// Handle resources/list — return available resources.
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

    /// Handle resources/read — return resource content by URI.
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
            _ => JsonRpcResponse::error(id.clone(), -32602, format!("Unknown resource URI: {}", uri)),
        }
    }

    /// Handle tools/call — dispatch to the appropriate tool handler.
    /// Every call is logged for audit trail (Section 12.1).
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
                    format!("Invalid params: {}", e),
                );
            }
        };

        // Audit trail: log every MCP action with spec-required fields (Section 12.1)
        let audit_trace_id = uuid::Uuid::new_v4().to_string()[..12].to_string();

        // Auth enforcement (Section 8.3): check permissions for action tools.
        // Any plugin convention (soma.X.Y with 2+ dots) is an action tool, not just posix.
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
            // MCP auth: token can be in _meta.auth_token (MCP extension),
            // in a top-level _token field, or passed during initialize.
            // For stdio transport, auth is typically handled at the process level.
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
                        "MCP auth denied: {}", e
                    );
                    return JsonRpcResponse::success(
                        id.clone(),
                        serde_json::to_value(&McpToolResult::error(format!("Auth: {}", e)))
                            .unwrap_or_default(),
                    );
                }
            }
        };

        let result = match call.name.as_str() {
            // State tools
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

            // Action tools
            "soma.intent" => self.tool_intent(&call.arguments),
            "soma.checkpoint" => self.tool_checkpoint(&call.arguments),
            "soma.record_decision" => self.tool_record_decision(&call.arguments),
            "soma.confirm" => self.tool_confirm(&call.arguments).await,
            "soma.install_plugin" => self.tool_install_plugin(&call.arguments),
            "soma.restore_checkpoint" => self.tool_restore_checkpoint(&call.arguments),
            "soma.shutdown" => self.tool_shutdown(),
            "soma.uninstall_plugin" => self.tool_uninstall_plugin(&call.arguments),
            "soma.configure_plugin" => self.tool_configure_plugin(&call.arguments),
            "soma.reload_design" => McpToolResult::json(serde_json::json!({
                "success": false,
                "message": "Not yet connected to Interface SOMA. This tool will work when an Interface SOMA connects via Synaptic Protocol.",
            })),
            "soma.render_view" | "soma.update_view" => McpToolResult::json(serde_json::json!({
                "success": false,
                "message": "Not yet connected to Interface SOMA. This tool will work when an Interface SOMA connects via Synaptic Protocol.",
            })),

            // Plugin convention tools — dynamically namespaced: soma.{plugin}.{convention}
            name if name.starts_with("soma.") && name.matches('.').count() >= 2 => {
                self.tool_plugin_call(name, &call.arguments).await
            }

            _ => McpToolResult::error(format!("Unknown tool: {}", call.name)),
        };

        // Audit trail: log every MCP action with spec-required fields (Section 12.1)
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

    // ---- State tool implementations ----

    fn tool_get_state(&self) -> McpToolResult {
        let state = self.state.read().unwrap();
        let proprio = self.proprio.read().unwrap();
        let mind = self.mind.read().unwrap();
        let mind_info = mind.info();
        let exp = self.experience.read().unwrap();
        let peers = self.peers.read().unwrap();

        let result = serde_json::json!({
            "soma_id": self.config.soma.id,
            "version": "0.1.0",
            "uptime_secs": proprio.uptime().as_secs(),
            "mind": {
                "backend": mind_info.backend,
                "conventions_known": mind_info.conventions_known,
                "max_steps": mind_info.max_steps,
                "lora_layers": mind_info.lora_layers,
                "lora_magnitude": mind_info.lora_magnitude,
            },
            "state": state.to_json(),
            "experience": {
                "buffer_size": exp.len(),
                "total_seen": exp.total_seen(),
                "success_count": exp.success_count(),
                "failure_count": exp.failure_count(),
            },
            "proprioception": proprio.to_json(),
            "peers": peers.list().iter().map(|p| serde_json::json!({
                "name": p.name,
                "addr": p.addr,
            })).collect::<Vec<_>>(),
            "plugins": self.plugins.read().unwrap().conventions().iter().map(|c| &c.name).collect::<Vec<_>>(),
            "plugin_warnings": self.plugins.read().unwrap().check_plugin_health().iter().map(|(name, msg)| {
                serde_json::json!({"plugin": name, "warning": msg})
            }).collect::<Vec<_>>(),
        });

        McpToolResult::json(result)
    }

    fn tool_get_plugins(&self) -> McpToolResult {
        let plugins = self.plugins.read().unwrap();
        // Show namespaced conventions with full metadata including cleanup
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

        // Per-plugin health status (Section 11.3 — dead plugin detection)
        let health_warnings = plugins.check_plugin_health();
        let warning_map: std::collections::HashMap<&str, &str> = health_warnings.iter()
            .map(|(name, msg)| (name.as_str(), *msg))
            .collect();

        let plugin_list: Vec<serde_json::Value> = plugins_map.iter().map(|(name, convs)| {
            let health_status = if let Some(warning) = warning_map.get(name.as_str()) {
                serde_json::json!({"status": "degraded", "warning": warning})
            } else {
                serde_json::json!({"status": "healthy"})
            };
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

    fn tool_get_health(&self) -> McpToolResult {
        let proprio = self.proprio.read().unwrap();
        let metrics = self.metrics.to_json();

        // Check plugin health (Section 11.3 — dead plugin detection)
        let pm_guard = self.plugins.read().unwrap();
        let plugin_warnings = pm_guard.check_plugin_health();
        let status = if plugin_warnings.is_empty() { "healthy" } else { "degraded" };

        McpToolResult::json(serde_json::json!({
            "status": status,
            "proprioception": proprio.to_json(),
            "metrics": metrics,
            "plugin_warnings": plugin_warnings.iter().map(|(name, msg)| {
                serde_json::json!({"plugin": name, "warning": msg})
            }).collect::<Vec<_>>(),
        }))
    }

    fn tool_get_recent_activity(&self, args: &serde_json::Value) -> McpToolResult {
        let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let state = self.state.read().unwrap();
        let records = state.executions.recent(n);

        McpToolResult::json(serde_json::json!({
            "count": records.len(),
            "records": records,
        }))
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

        McpToolResult::json(serde_json::json!({
            "count": peer_list.len(),
            "peers": peer_list,
        }))
    }

    fn tool_get_experience(&self) -> McpToolResult {
        let exp = self.experience.read().unwrap();
        let mind_info = self.mind.read().unwrap().info();
        let proprio = self.proprio.read().unwrap();
        McpToolResult::json(serde_json::json!({
            "buffer_size": exp.len(),
            "max_size": self.config.memory.max_experience_buffer,
            "total_seen": exp.total_seen(),
            "success_count": exp.success_count(),
            "failure_count": exp.failure_count(),
            "lora_magnitude": mind_info.lora_magnitude,
            "lora_layers": mind_info.lora_layers,
            "adaptation_count": proprio.total_adaptations,
            "consolidation_count": proprio.consolidations,
        }))
    }

    fn tool_get_checkpoints(&self) -> McpToolResult {
        let ckpt_dir = Path::new(&self.config.memory.checkpoint_dir);
        let checkpoints = match Checkpoint::list_checkpoints(ckpt_dir) {
            Ok(paths) => paths.iter().map(|p| {
                let filename = p.file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                serde_json::json!({
                    "path": p.display().to_string(),
                    "filename": filename,
                })
            }).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };

        McpToolResult::json(serde_json::json!({
            "checkpoint_dir": self.config.memory.checkpoint_dir,
            "count": checkpoints.len(),
            "checkpoints": checkpoints,
        }))
    }

    fn tool_get_config(&self) -> McpToolResult {
        // Serialize config (safe fields only — no tokens/secrets)
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

    fn tool_get_decisions(&self, args: &serde_json::Value) -> McpToolResult {
        let state = self.state.read().unwrap();

        if let Some(query) = args.get("search").and_then(|v| v.as_str()) {
            let results = state.decisions.search(query);
            return McpToolResult::json(serde_json::json!({
                "search": query,
                "count": results.len(),
                "decisions": results,
            }));
        }

        let n = args.get("n").and_then(|v| v.as_u64());
        let decisions = match n {
            Some(n) => state.decisions.recent(n as usize),
            None => state.decisions.list(),
        };

        McpToolResult::json(serde_json::json!({
            "count": decisions.len(),
            "decisions": decisions,
        }))
    }

    async fn tool_get_schema(&self, args: &serde_json::Value) -> McpToolResult {
        // Check if a database plugin (postgres or sqlite) is loaded
        let pm = self.plugins.read().unwrap();
        let has_postgres = pm.plugin_names().iter().any(|n| n == "postgres");

        if !has_postgres {
            return McpToolResult::json(serde_json::json!({
                "tables": [],
                "note": "No database plugin loaded. Install postgres or sqlite plugin for schema tracking.",
            }));
        }

        // If a specific table is requested, query only that table's schema with sample rows
        if let Some(table_name) = args.get("table").and_then(|v| v.as_str()) {
            let schema_args = vec![crate::plugin::interface::Value::String(table_name.to_string())];
            let columns = match pm.execute_by_plugin_async("postgres", 10, schema_args).await {
                Ok(cols) => format!("{}", cols),
                Err(e) => return McpToolResult::error(format!("Failed to query table schema: {}", e)),
            };
            // Try to get sample rows (convention id 11 = sample_rows, if available)
            let sample_args = vec![
                crate::plugin::interface::Value::String(table_name.to_string()),
                crate::plugin::interface::Value::Int(5),
            ];
            let sample_rows = match pm.execute_by_plugin_async("postgres", 11, sample_args).await {
                Ok(rows) => Some(format!("{}", rows)),
                Err(_) => None,
            };
            let mut result = serde_json::json!({
                "table": table_name,
                "columns": columns,
            });
            if let Some(rows) = sample_rows {
                result.as_object_mut().unwrap().insert("sample_rows".to_string(), serde_json::json!(rows));
            }
            return McpToolResult::json(result);
        }

        // Query list_tables convention (id 9 in postgres plugin)
        let tables = match pm.execute_by_plugin_async("postgres", 9, vec![]).await {
            Ok(crate::plugin::interface::Value::List(table_list)) => {
                let mut result = Vec::new();
                for table_val in &table_list {
                    if let crate::plugin::interface::Value::String(table_name) = table_val {
                        // Query table_schema convention (id 10) for each table
                        let schema_args = vec![crate::plugin::interface::Value::String(table_name.clone())];
                        let columns = match pm.execute_by_plugin_async("postgres", 10, schema_args).await {
                            Ok(cols) => format!("{}", cols),
                            Err(_) => "[]".to_string(),
                        };
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
                    "raw": format!("{}", other),
                    "note": "Unexpected response from list_tables",
                }));
            }
            Err(e) => {
                return McpToolResult::error(format!("Failed to query schema: {}", e));
            }
        };

        McpToolResult::json(serde_json::json!({
            "tables": tables,
            "total_tables": tables.len(),
        }))
    }

    fn tool_get_business_rules(&self) -> McpToolResult {
        // Business rules derived from decisions and configuration
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

        McpToolResult::json(serde_json::json!({
            "count": rules.len(),
            "rules": rules,
            "note": "Business rules are derived from decisions containing 'rule', 'policy', 'require', or 'constraint'.",
        }))
    }

    fn tool_get_render_state(&self) -> McpToolResult {
        // Render state — populated when Interface SOMA is connected.
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

    // ---- Action tool implementations ----

    fn tool_intent(&self, args: &serde_json::Value) -> McpToolResult {
        let text = match args.get("text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return McpToolResult::error("Missing required argument: text".into()),
        };

        let trace_id = uuid::Uuid::new_v4().to_string()[..12].to_string();
        let exec_start = std::time::Instant::now();

        let mind_guard = self.mind.read().unwrap();
        match mind_guard.infer(text) {
            Ok(program) => {
                let result = self.plugins.read().unwrap().execute_program(
                    &program.steps,
                    self.config.mind.max_program_steps,
                );

                let execution_time_ms = exec_start.elapsed().as_millis() as u64;

                // Record metrics
                self.metrics.record_inference(result.success, execution_time_ms);
                self.metrics.record_program(program.steps.len() as u64);

                // Record in execution history
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

                // Update proprioception
                if let Ok(mut p) = self.proprio.write() {
                    if result.success { p.record_success(); }
                    else { p.record_failure(); }
                }

                // Record experience
                let tokens: Vec<u32> = mind_guard.tokenizer.encode(text)
                    .iter().map(|&t| t as u32).collect();
                let prog_data: Vec<(i32, u8, u8)> = program.steps.iter().map(|s| {
                    use crate::mind::ArgType;
                    let a0 = match s.arg0_type { ArgType::None => 0u8, ArgType::Span => 1, ArgType::Ref => 2, ArgType::Literal => 3 };
                    let a1 = match s.arg1_type { ArgType::None => 0u8, ArgType::Span => 1, ArgType::Ref => 2, ArgType::Literal => 3 };
                    (s.conv_id, a0, a1)
                }).collect();
                if let Ok(mut buf) = self.experience.write() {
                    // Section 17.1: Only successful executions are recorded
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
                    // Update experience buffer gauge
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
                    "output": result.output.map(|o| format!("{}", o)),
                    "error": result.error,
                    "trace": result.trace,
                });

                McpToolResult::json(response)
            }
            Err(e) => {
                self.metrics.record_inference(false, exec_start.elapsed().as_millis() as u64);
                if let Ok(mut p) = self.proprio.write() {
                    p.record_failure();
                }
                McpToolResult::error(format!("Inference error: {}", e))
            }
        }
    }

    fn tool_checkpoint(&self, args: &serde_json::Value) -> McpToolResult {
        let label = args.get("label").and_then(|v| v.as_str());
        let ckpt_dir = Path::new(&self.config.memory.checkpoint_dir);
        let filename = if let Some(lbl) = label {
            // Include label in filename: soma-{id}-{label}-{timestamp}.ckpt
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("soma-{}-{}-{}.ckpt", self.config.soma.id, lbl, ts)
        } else {
            Checkpoint::filename(&self.config.soma.id)
        };
        let path = ckpt_dir.join(&filename);

        let (exp_count, adapt_count) = {
            let p = self.proprio.read().unwrap();
            (p.experience_count, p.total_adaptations)
        };

        // Collect plugin state (same as do_checkpoint in main.rs)
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
            ckpt.base_model_hash = m.model_hash.clone();
        }
        ckpt.plugin_manifest = self.plugins.read().unwrap().plugin_manifest().into_iter()
            .map(|(name, version)| crate::memory::checkpoint::PluginManifestEntry { name, version })
            .collect();

        // Persist decisions and execution history
        if let Ok(st) = self.state.read() {
            ckpt.decisions = serde_json::to_value(st.decisions.list())
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            ckpt.recent_executions = serde_json::to_value(&st.executions.to_json())
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
            Err(e) => McpToolResult::error(format!("Checkpoint failed: {}", e)),
        }
    }

    fn tool_record_decision(&self, args: &serde_json::Value) -> McpToolResult {
        let what = match args.get("what").and_then(|v| v.as_str()) {
            Some(w) => w.to_string(),
            None => return McpToolResult::error("Missing required argument: what".into()),
        };
        let why = match args.get("why").and_then(|v| v.as_str()) {
            Some(w) => w.to_string(),
            None => return McpToolResult::error("Missing required argument: why".into()),
        };

        // Optional enrichment fields
        let context = args.get("context").and_then(|v| v.as_str()).map(|s| s.to_string());
        let related_tables: Option<Vec<String>> = args.get("related_tables")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());
        let related_plugins: Option<Vec<String>> = args.get("related_plugins")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());

        // Build enriched 'what' with context if provided
        let enriched_what = if let Some(ctx) = &context {
            format!("{} [context: {}]", what, ctx)
        } else {
            what
        };

        // Build enriched 'why' with related info if provided
        let mut enriched_why = why;
        if let Some(tables) = &related_tables {
            enriched_why = format!("{} [tables: {}]", enriched_why, tables.join(", "));
        }
        if let Some(plugins) = &related_plugins {
            enriched_why = format!("{} [plugins: {}]", enriched_why, plugins.join(", "));
        }

        // Session ID from auth token or anonymous
        let session_id = "mcp-session".to_string();

        if let Ok(mut state) = self.state.write() {
            let decision = state.decisions.record(enriched_what, enriched_why, session_id);
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
        } else {
            McpToolResult::error("Failed to acquire state lock".into())
        }
    }

    async fn tool_confirm(&self, args: &serde_json::Value) -> McpToolResult {
        let action_id = match args.get("action_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return McpToolResult::error("Missing required argument: action_id".into()),
        };

        let pending = {
            let mut auth = self.auth.write().unwrap();
            auth.confirm(action_id)
        };

        match pending {
            Some(pending) => {
                // Re-dispatch the original tool call with confirmed:true injected
                let mut confirmed_args = pending.arguments.clone();
                if let Some(obj) = confirmed_args.as_object_mut() {
                    obj.insert("confirmed".to_string(), serde_json::json!(true));
                }
                let redispatch_result = match pending.tool_name.as_str() {
                    "soma.restore_checkpoint" => self.tool_restore_checkpoint(&confirmed_args),
                    name if name.starts_with("soma.") && name.matches('.').count() >= 2 => {
                        self.tool_plugin_call(name, &confirmed_args).await
                    }
                    _ => McpToolResult::error(format!("Cannot re-dispatch tool: {}", pending.tool_name)),
                };
                redispatch_result
            }
            None => McpToolResult::error(format!("No pending confirmation: {} (expired or invalid)", action_id)),
        }
    }

    fn tool_install_plugin(&self, args: &serde_json::Value) -> McpToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return McpToolResult::error("Missing required argument: name".into()),
        };

        // Look for plugin .so/.dylib in plugins directory
        let plugins_dir = std::path::Path::new(&self.config.soma.plugins_directory);
        let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
        let plugin_path = plugins_dir.join(format!("lib{}.{}", name, ext));

        if !plugin_path.exists() {
            return McpToolResult::json(serde_json::json!({
                "success": false,
                "error": format!("Plugin file not found: {}", plugin_path.display()),
                "searched": plugins_dir.display().to_string(),
                "expected_filename": format!("lib{}.{}", name, ext),
            }));
        }

        match crate::plugin::dynamic::load_plugin_from_path(&plugin_path) {
            Ok(mut plugin) => {
                // Build plugin config from [plugins.<name>] section
                let mut pc = crate::plugin::interface::PluginConfig::default();
                if let Some(toml_val) = self.config.plugins.get(name) {
                    if let Some(table) = toml_val.as_table() {
                        for (k, v) in table {
                            if let Ok(json_val) = serde_json::to_value(v) {
                                pc.settings.insert(k.clone(), json_val);
                            }
                        }
                    }
                }
                if let Err(e) = plugin.on_load(&pc) {
                    return McpToolResult::error(format!("Plugin on_load failed: {}", e));
                }
                let pname = plugin.name().to_string();
                let pversion = plugin.version().to_string();
                let conv_count = plugin.conventions().len();
                // Register dynamically via write lock
                let mut pm = self.plugins.write().unwrap();
                pm.register(plugin);
                McpToolResult::json(serde_json::json!({
                    "success": true,
                    "plugin": pname,
                    "version": pversion,
                    "conventions": conv_count,
                    "note": "Plugin loaded and registered at runtime",
                }))
            }
            Err(e) => McpToolResult::error(format!("Failed to load plugin: {}", e)),
        }
    }

    fn tool_restore_checkpoint(&self, args: &serde_json::Value) -> McpToolResult {
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return McpToolResult::error("Missing required argument: path".into()),
        };

        // Destructive action: require two-step confirmation (Section 12.1)
        if self.config.security.require_confirmation {
            if args.get("confirmed").and_then(|v| v.as_bool()) != Some(true) {
                let mut auth = self.auth.write().unwrap();
                let action_id = auth.create_confirmation(
                    format!("Restore checkpoint from {}", path_str),
                    "mcp",
                    "soma.restore_checkpoint".to_string(),
                    args.clone(),
                );
                return McpToolResult::json(serde_json::json!({
                    "requires_confirmation": true,
                    "action_id": action_id,
                    "description": format!("Restore checkpoint from {}. This will overwrite current LoRA state.", path_str),
                    "instructions": "Call soma.confirm with the action_id to proceed, or pass confirmed:true.",
                }));
            }
        }

        let path = std::path::Path::new(path_str);
        match Checkpoint::load(path) {
            Ok(ckpt) => {
                if let Ok(mut p) = self.proprio.write() {
                    p.experience_count = ckpt.experience_count;
                    p.total_adaptations = ckpt.adaptation_count;
                }
                // Restore decisions into SomaState (Section 7.5)
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
            Err(e) => McpToolResult::error(format!("Restore failed: {}", e)),
        }
    }

    fn tool_shutdown(&self) -> McpToolResult {
        tracing::info!(component = "mcp", action = "shutdown", "Graceful shutdown requested via MCP");
        self.shutdown_requested.store(true, std::sync::atomic::Ordering::SeqCst);
        McpToolResult::json(serde_json::json!({
            "success": true,
            "message": "Shutdown requested. SOMA will terminate gracefully.",
        }))
    }

    fn tool_uninstall_plugin(&self, args: &serde_json::Value) -> McpToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return McpToolResult::error("Missing required argument: name".into()),
        };

        let mut pm = self.plugins.write().unwrap();
        match pm.unregister(name) {
            Ok(()) => McpToolResult::json(serde_json::json!({
                "success": true,
                "plugin": name,
                "note": "Plugin unloaded and unregistered",
            })),
            Err(e) => McpToolResult::error(format!("Uninstall failed: {}", e)),
        }
    }

    fn tool_configure_plugin(&self, args: &serde_json::Value) -> McpToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return McpToolResult::error("Missing required argument: name".into()),
        };
        let config_val = match args.get("config") {
            Some(c) => c,
            None => return McpToolResult::error("Missing required argument: config".into()),
        };

        // Build a PluginConfig from the provided JSON object
        let mut pc = crate::plugin::interface::PluginConfig::default();
        if let Some(obj) = config_val.as_object() {
            for (k, v) in obj {
                pc.settings.insert(k.clone(), v.clone());
            }
        }

        // Find the plugin and call on_load with new config (acts as config update)
        let mut pm = self.plugins.write().unwrap();
        let plugin_names: Vec<String> = pm.plugin_names();
        if !plugin_names.contains(&name.to_string()) {
            return McpToolResult::error(format!("Plugin not found: {}", name));
        }

        // on_load is the lifecycle hook for configuration; there is no separate on_config_changed
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

    async fn tool_plugin_call(&self, tool_name: &str, args: &serde_json::Value) -> McpToolResult {
        // Dynamic plugin namespacing: soma.{plugin}.{convention} (Section 12.2)
        let parts: Vec<&str> = tool_name.splitn(3, '.').collect();
        if parts.len() < 3 {
            return McpToolResult::error(format!("Invalid tool name: {}", tool_name));
        }
        let plugin_name = parts[1]; // e.g. "posix"
        let conv_name = parts[2]; // e.g. "open_read"

        let conventions = self.plugins.read().unwrap().conventions();
        let conv = match conventions.iter().find(|c| c.name == conv_name) {
            Some(c) => c,
            None => return McpToolResult::error(format!("Unknown convention: {}", conv_name)),
        };

        // Build args from JSON, supporting List and Map types for complex arguments
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
                    _ => {
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
                // Optional argument not provided — push Null as placeholder
                plugin_args.push(crate::plugin::interface::Value::Null);
            }
        }

        // Use async execution for I/O-bound plugins (postgres, redis, http, etc.)
        let conv_id = conv.id;
        let pm = self.plugins.read().unwrap();
        match pm.execute_by_plugin_async(plugin_name, conv_id, plugin_args).await {
            Ok(val) => McpToolResult::json(serde_json::json!({
                "success": true,
                "result": format!("{}", val),
            })),
            Err(e) => McpToolResult::error(format!("Plugin error: {}", e)),
        }
    }
}

/// Convert a JSON value to a plugin Value, preserving structure for complex types.
fn json_to_value(val: &serde_json::Value) -> crate::plugin::interface::Value {
    match val {
        serde_json::Value::Null => crate::plugin::interface::Value::Null,
        serde_json::Value::Bool(b) => crate::plugin::interface::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                crate::plugin::interface::Value::Int(i)
            } else {
                crate::plugin::interface::Value::Float(n.as_f64().unwrap_or(0.0))
            }
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
