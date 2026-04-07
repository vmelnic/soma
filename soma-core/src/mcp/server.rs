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
    pub plugins: Arc<PluginManager>,
    pub proprio: Arc<RwLock<Proprioception>>,
    pub experience: Arc<RwLock<ExperienceBuffer>>,
    pub state: Arc<RwLock<SomaState>>,
    pub metrics: Arc<SomaMetrics>,
    pub peers: Arc<RwLock<PeerRegistry>>,
    pub auth: Arc<RwLock<AuthManager>>,
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
        let conventions = self.plugins.namespaced_conventions();
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

        // Audit trail: log every MCP action (Section 12.1)
        tracing::info!(
            component = "mcp",
            action = "tool_call",
            tool = %call.name,
            "MCP tool invoked"
        );

        // Auth enforcement (Section 8.3): check permissions for action tools
        let is_action = call.name == "soma.intent"
            || call.name == "soma.checkpoint"
            || call.name == "soma.record_decision"
            || call.name == "soma.install_plugin"
            || call.name == "soma.restore_checkpoint"
            || call.name.starts_with("soma.posix.");
        let is_admin = call.name == "soma.install_plugin"
            || call.name == "soma.restore_checkpoint";

        {
            let auth = self.auth.read().unwrap();
            // MCP auth: token can be in _meta.auth_token (MCP extension),
            // in a top-level _token field, or passed during initialize.
            // For stdio transport, auth is typically handled at the process level.
            let token = call.arguments.get("_meta")
                .and_then(|m| m.get("auth_token"))
                .and_then(|v| v.as_str())
                .or_else(|| call.arguments.get("_token").and_then(|v| v.as_str()));
            match auth.check_request(token, is_action, is_admin) {
                Ok(_session) => {} // authorized
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
        }

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
            "soma.get_schema" => self.tool_get_schema(),
            "soma.get_business_rules" => self.tool_get_business_rules(),
            "soma.get_render_state" => self.tool_get_render_state(),

            // Action tools
            "soma.intent" => self.tool_intent(&call.arguments),
            "soma.checkpoint" => self.tool_checkpoint(),
            "soma.record_decision" => self.tool_record_decision(&call.arguments),
            "soma.confirm" => self.tool_confirm(&call.arguments),
            "soma.install_plugin" => self.tool_install_plugin(&call.arguments),
            "soma.restore_checkpoint" => self.tool_restore_checkpoint(&call.arguments),

            // Plugin convention tools — dynamically namespaced: soma.{plugin}.{convention}
            name if name.starts_with("soma.") && name.matches('.').count() >= 2 => {
                self.tool_plugin_call(name, &call.arguments)
            }

            _ => McpToolResult::error(format!("Unknown tool: {}", call.name)),
        };

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
            "plugins": self.plugins.conventions().iter().map(|c| &c.name).collect::<Vec<_>>(),
        });

        McpToolResult::json(result)
    }

    fn tool_get_plugins(&self) -> McpToolResult {
        // Show namespaced conventions with full metadata including cleanup
        let namespaced = self.plugins.namespaced_conventions();
        let mut plugins_map: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();

        for (plugin_name, c) in &namespaced {
            let entry = serde_json::json!({
                "id": c.id,
                "name": c.name,
                "full_name": format!("soma.{}.{}", plugin_name, c.name),
                "description": c.description,
                "call_pattern": c.call_pattern,
                "return_type": c.return_type,
                "estimated_latency_ms": c.estimated_latency_ms,
                "cleanup_convention": c.cleanup_convention,
                "args": c.args,
            });
            plugins_map.entry(plugin_name.clone()).or_default().push(entry);
        }

        let plugin_list: Vec<serde_json::Value> = plugins_map.iter().map(|(name, convs)| {
            serde_json::json!({
                "name": name,
                "version": "0.1.0",
                "trust_level": "built_in",
                "convention_count": convs.len(),
                "conventions": convs,
            })
        }).collect();

        McpToolResult::json(serde_json::json!({
            "count": plugin_list.len(),
            "plugins": plugin_list,
        }))
    }

    fn tool_get_conventions(&self) -> McpToolResult {
        let conventions = self.plugins.conventions();
        McpToolResult::json(serde_json::to_value(&conventions).unwrap_or_default())
    }

    fn tool_get_health(&self) -> McpToolResult {
        let proprio = self.proprio.read().unwrap();
        let metrics = self.metrics.to_json();

        McpToolResult::json(serde_json::json!({
            "status": "healthy",
            "proprioception": proprio.to_json(),
            "metrics": metrics,
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
        McpToolResult::json(serde_json::json!({
            "buffer_size": exp.len(),
            "max_size": self.config.memory.max_experience_buffer,
            "total_seen": exp.total_seen(),
            "success_count": exp.success_count(),
            "failure_count": exp.failure_count(),
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

    fn tool_get_schema(&self) -> McpToolResult {
        // Database schema — populated when a database plugin (postgres, sqlite) is loaded.
        // Until then, returns empty structure.
        McpToolResult::json(serde_json::json!({
            "tables": [],
            "note": "No database plugin loaded. Install postgres or sqlite plugin for schema tracking.",
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
                let result = self.plugins.execute_program(
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
                    let a0 = match s.arg0_type { ArgType::None => 0u8, ArgType::Span => 1, ArgType::Ref => 2 };
                    let a1 = match s.arg1_type { ArgType::None => 0u8, ArgType::Span => 1, ArgType::Ref => 2 };
                    (s.conv_id, a0, a1)
                }).collect();
                if let Ok(mut buf) = self.experience.write() {
                    buf.record(crate::memory::experience::Experience {
                        intent_tokens: tokens,
                        program: prog_data,
                        success: result.success,
                        execution_time_ms,
                        timestamp: std::time::Instant::now(),
                    });
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

    fn tool_checkpoint(&self) -> McpToolResult {
        let ckpt_dir = Path::new(&self.config.memory.checkpoint_dir);
        let filename = Checkpoint::filename(&self.config.soma.id);
        let path = ckpt_dir.join(&filename);

        let (exp_count, adapt_count) = {
            let p = self.proprio.read().unwrap();
            (p.experience_count, p.total_adaptations)
        };

        // Collect plugin state (same as do_checkpoint in main.rs)
        let plugin_states = self.plugins.collect_plugin_states();
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
                McpToolResult::json(serde_json::json!({
                    "success": true,
                    "path": path.display().to_string(),
                    "experience_count": exp_count,
                    "adaptation_count": adapt_count,
                }))
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

        // Session ID from auth token or anonymous
        let session_id = "mcp-session".to_string();

        if let Ok(mut state) = self.state.write() {
            let decision = state.decisions.record(what, why, session_id);
            if let Ok(mut p) = self.proprio.write() {
                p.record_decision();
            }
            McpToolResult::json(serde_json::json!({
                "success": true,
                "decision": decision,
            }))
        } else {
            McpToolResult::error("Failed to acquire state lock".into())
        }
    }

    fn tool_confirm(&self, args: &serde_json::Value) -> McpToolResult {
        let action_id = match args.get("action_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return McpToolResult::error("Missing required argument: action_id".into()),
        };

        let mut auth = self.auth.write().unwrap();
        match auth.confirm(action_id) {
            Some(pending) => {
                McpToolResult::json(serde_json::json!({
                    "success": true,
                    "action_id": pending.action_id,
                    "description": pending.description,
                    "confirmed": true,
                }))
            }
            None => McpToolResult::error(format!("No pending confirmation: {} (expired or invalid)", action_id)),
        }
    }

    fn tool_install_plugin(&self, args: &serde_json::Value) -> McpToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return McpToolResult::error("Missing required argument: name".into()),
        };

        // Plugin installation requires the plugin registry (future feature).
        // For now, return informative error about available plugins.
        McpToolResult::json(serde_json::json!({
            "success": false,
            "error": format!("Plugin registry not yet available. Cannot install '{}'.", name),
            "available_builtin": ["posix"],
            "note": "Dynamic plugin loading will be available when .soma-plugin archive format is implemented.",
        }))
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

    fn tool_plugin_call(&self, tool_name: &str, args: &serde_json::Value) -> McpToolResult {
        // Dynamic plugin namespacing: soma.{plugin}.{convention} (Section 12.2)
        let parts: Vec<&str> = tool_name.splitn(3, '.').collect();
        if parts.len() < 3 {
            return McpToolResult::error(format!("Invalid tool name: {}", tool_name));
        }
        let plugin_name = parts[1]; // e.g. "posix"
        let conv_name = parts[2]; // e.g. "open_read"

        let conventions = self.plugins.conventions();
        let conv = match conventions.iter().find(|c| c.name == conv_name) {
            Some(c) => c,
            None => return McpToolResult::error(format!("Unknown convention: {}", conv_name)),
        };

        // Build args from JSON
        let mut plugin_args = Vec::new();
        for arg_spec in &conv.args {
            if let Some(val) = args.get(&arg_spec.name) {
                let pval = match arg_spec.arg_type.as_str() {
                    "int" | "handle" => {
                        crate::plugin::interface::Value::Int(val.as_i64().unwrap_or(0))
                    }
                    "bool" => {
                        crate::plugin::interface::Value::Bool(val.as_bool().unwrap_or(false))
                    }
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
            }
        }

        match self.plugins.execute_by_plugin(plugin_name, conv.id, plugin_args) {
            Ok(val) => McpToolResult::json(serde_json::json!({
                "success": true,
                "result": format!("{}", val),
            })),
            Err(e) => McpToolResult::error(format!("Plugin error: {}", e)),
        }
    }
}
