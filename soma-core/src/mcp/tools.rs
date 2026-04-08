//! MCP tool definitions and result types.
//!
//! Defines the schema for all tools exposed over the MCP interface:
//! - **State tools** (read-only): `soma.get_state`, `soma.get_plugins`, `soma.get_conventions`,
//!   `soma.get_health`, `soma.get_recent_activity`, `soma.get_peers`, `soma.get_experience`,
//!   `soma.get_checkpoints`, `soma.get_config`, `soma.get_decisions`, `soma.get_metrics`,
//!   `soma.get_schema`, `soma.get_business_rules`, `soma.get_render_state`
//! - **Action tools** (side effects): `soma.intent`, `soma.checkpoint`, `soma.record_decision`,
//!   `soma.confirm`, `soma.install_plugin`, `soma.restore_checkpoint`, `soma.shutdown`,
//!   `soma.uninstall_plugin`, `soma.configure_plugin`, `soma.reload_design`,
//!   `soma.render_view`, `soma.update_view`
//! - **Plugin convention tools**: dynamically generated as `soma.{plugin}.{convention}`
//!   from every loaded plugin's conventions.

use serde::{Deserialize, Serialize};

/// An MCP tool definition, serialized in `tools/list` responses. Each tool has a name,
/// human-readable description, and a JSON Schema describing its accepted arguments.
#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Result of a `tools/call` invocation, returned as the JSON-RPC result payload.
/// Contains one or more content blocks and an optional error flag.
#[derive(Debug, Serialize)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

/// A single content block within an `McpToolResult`. Currently only `"text"` type is used.
#[derive(Debug, Serialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl McpToolResult {
    /// Wrap a plain string as a successful text result.
    pub fn text(s: String) -> Self {
        Self {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: s,
            }],
            is_error: None,
        }
    }

    /// Pretty-print a JSON value as a successful text result.
    #[allow(clippy::needless_pass_by_value)] // Ergonomic: callers pass json!() directly
    pub fn json(val: serde_json::Value) -> Self {
        Self::text(serde_json::to_string_pretty(&val).unwrap_or_default())
    }

    /// Wrap an error message. Sets `is_error: true` so the LLM knows the call failed.
    pub fn error(msg: String) -> Self {
        Self {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: msg,
            }],
            is_error: Some(true),
        }
    }
}

/// Build the complete MCP tool catalog: built-in state tools, built-in action tools,
/// and one tool per loaded plugin convention (namespaced as `soma.{plugin}.{convention}`).
///
/// Called on every `tools/list` request. Plugin conventions are passed in as
/// `(plugin_name, Convention)` pairs from `PluginManager::namespaced_conventions()`.
#[allow(clippy::too_many_lines)] // Tool list is declarative; splitting would harm readability
pub fn build_tool_list(conventions: &[(String, crate::plugin::interface::Convention)]) -> Vec<McpTool> {
    let mut tools = Vec::new();

    tools.push(McpTool {
        name: "soma.get_state".into(),
        description: "Get the complete SOMA state: decisions, recent executions, health. This is the primary context tool — call it at the start of every session.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_plugins".into(),
        description: "List all loaded plugins with their conventions, versions, and trust levels.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_conventions".into(),
        description: "List all available calling conventions with argument specs and descriptions.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_health".into(),
        description: "Get SOMA health: uptime, inference stats, memory, protocol status, metrics.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_recent_activity".into(),
        description: "Get the N most recent execution records.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "n": { "type": "integer", "description": "Number of records (default: 10)", "default": 10 }
            },
        }),
    });

    tools.push(McpTool {
        name: "soma.get_peers".into(),
        description: "List all known peer SOMAs with their plugins and connection status.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_experience".into(),
        description: "Get experience buffer stats: size, success rate, recent experiences.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_checkpoints".into(),
        description: "List available checkpoints with timestamps and metadata.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_config".into(),
        description: "Get the current SOMA configuration.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_decisions".into(),
        description: "Get the decision log: what was built, why, when, by which session.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "n": { "type": "integer", "description": "Number of recent decisions (default: all)" },
                "search": { "type": "string", "description": "Search keyword in decisions" }
            },
        }),
    });

    tools.push(McpTool {
        name: "soma.get_metrics".into(),
        description: "Get Prometheus-compatible metrics for monitoring.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "format": { "type": "string", "enum": ["json", "prometheus"], "default": "json" }
            },
        }),
    });

    tools.push(McpTool {
        name: "soma.get_schema".into(),
        description: "Get database schema state — tables, columns, types. Returns empty until a database plugin is loaded.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "table": { "type": "string", "description": "Optional: query only this table's schema with sample rows" }
            },
        }),
    });

    tools.push(McpTool {
        name: "soma.get_business_rules".into(),
        description: "Get business rules derived from decisions and configuration.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.get_render_state".into(),
        description: "Get the current render state for Interface SOMA — active views, pending updates.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.intent".into(),
        description: "Execute a natural language intent. The Mind generates a program and the Body executes it.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "The intent to execute" }
            },
            "required": ["text"],
        }),
    });

    tools.push(McpTool {
        name: "soma.checkpoint".into(),
        description: "Save a checkpoint of the current SOMA state (LoRA, experiences, adaptations).".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "label": { "type": "string", "description": "Optional label for the checkpoint (included in filename and metadata)" }
            },
        }),
    });

    tools.push(McpTool {
        name: "soma.record_decision".into(),
        description: "Record a decision in the permanent decision log. Use this to document what was built and why.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "what": { "type": "string", "description": "What was decided/built" },
                "why": { "type": "string", "description": "Why this decision was made" },
                "context": { "type": "string", "description": "Optional additional context for the decision" },
                "related_tables": { "type": "array", "items": { "type": "string" }, "description": "Optional list of related database tables" },
                "related_plugins": { "type": "array", "items": { "type": "string" }, "description": "Optional list of related plugin names" }
            },
            "required": ["what", "why"],
        }),
    });

    tools.push(McpTool {
        name: "soma.confirm".into(),
        description: "Confirm a pending destructive action by its action_id.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action_id": { "type": "string", "description": "The action ID to confirm" }
            },
            "required": ["action_id"],
        }),
    });

    tools.push(McpTool {
        name: "soma.install_plugin".into(),
        description: "Install a plugin by name from the plugin registry. Requires admin access.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Plugin name (e.g. 'postgres', 'redis')" }
            },
            "required": ["name"],
        }),
    });

    tools.push(McpTool {
        name: "soma.restore_checkpoint".into(),
        description: "Restore SOMA state from a specific checkpoint file. Requires admin access and confirmation.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to checkpoint file" }
            },
            "required": ["path"],
        }),
    });

    tools.push(McpTool {
        name: "soma.shutdown".into(),
        description: "Trigger graceful shutdown. Requires admin access.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.uninstall_plugin".into(),
        description: "Uninstall a loaded plugin by name. Requires admin access.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Plugin name to uninstall (e.g. 'postgres', 'redis')" }
            },
            "required": ["name"],
        }),
    });

    tools.push(McpTool {
        name: "soma.configure_plugin".into(),
        description: "Update configuration for a loaded plugin. Requires admin access.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Plugin name to configure" },
                "config": { "type": "object", "description": "Configuration key-value pairs to set" }
            },
            "required": ["name", "config"],
        }),
    });

    tools.push(McpTool {
        name: "soma.reload_design".into(),
        description: "Reload the current UI design from Interface SOMA.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    });

    tools.push(McpTool {
        name: "soma.render_view".into(),
        description: "Render a named view via Interface SOMA.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "view": { "type": "string", "description": "View name to render" },
                "data": { "type": "object", "description": "Data context for the view" }
            },
            "required": ["view"],
        }),
    });

    tools.push(McpTool {
        name: "soma.update_view".into(),
        description: "Update an existing rendered view with new data.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "view": { "type": "string", "description": "View name to update" },
                "patch": { "type": "object", "description": "Partial data to merge into the view" }
            },
            "required": ["view"],
        }),
    });

    // Generate one tool per loaded plugin convention, using ArgType::json_type() for schema mapping.
    for (plugin_name, conv) in conventions {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for arg in &conv.args {
            let json_type = arg.arg_type.json_type();
            properties.insert(arg.name.clone(), serde_json::json!({
                "type": json_type,
                "description": arg.description,
            }));
            if arg.required {
                required.push(serde_json::Value::String(arg.name.clone()));
            }
        }

        tools.push(McpTool {
            name: format!("soma.{}.{}", plugin_name, conv.name),
            description: conv.description.clone(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required,
            }),
        });
    }

    tools
}

/// Deserialized payload from a `tools/call` JSON-RPC request.
#[derive(Debug, Deserialize)]
pub struct ToolCallArgs {
    /// Fully-qualified tool name, e.g. `"soma.get_state"` or `"soma.postgres.query"`.
    pub name: String,
    /// Tool-specific arguments. May contain `_meta.auth_token` or `_token` for auth.
    #[serde(default)]
    pub arguments: serde_json::Value,
}
