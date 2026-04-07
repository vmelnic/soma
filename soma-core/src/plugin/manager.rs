//! Plugin Manager — loads plugins, routes program steps to the right plugin.

use super::interface::{Convention, PluginError, SomaPlugin, Value};
use crate::mind::{ArgValue, ProgramStep, EMIT_ID, STOP_ID};

/// Result of executing a full program.
#[derive(serde::Serialize)]
pub struct ProgramResult {
    pub success: bool,
    pub output: Option<Value>,
    pub trace: Vec<TraceEntry>,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct TraceEntry {
    pub step: usize,
    pub op: String,
    pub success: bool,
    pub summary: String,
}

/// Per-convention execution statistics (Section 19.3).
#[derive(Debug, Default)]
pub struct ConventionStats {
    pub call_count: u64,
    pub total_time_ms: u64,
    pub error_count: u64,
    pub timeout_count: u64,
}

pub struct PluginManager {
    plugins: Vec<Box<dyn SomaPlugin>>,
    /// Maps convention ID (from catalog) -> (plugin_index, plugin_convention_id)
    routing: std::collections::HashMap<u32, (usize, u32)>,
    /// Tracks crashed plugins via interior mutability — execute_step(&self) can mark
    /// crashed plugins without &mut self (Section 11.3).
    crashed_plugins: std::sync::RwLock<std::collections::HashSet<usize>>,
    /// Name-based convention lookup (Section 5.4). Maps "plugin.convention" → global_id.
    /// Used for future name-based model predictions. Currently populated alongside ID routing.
    name_routing: std::collections::HashMap<String, u32>,
    /// Optional metrics reference for tracking plugin calls
    metrics: Option<std::sync::Arc<crate::metrics::SomaMetrics>>,
    /// Per-convention stats: global_id -> stats (Section 19.3)
    convention_stats: std::sync::RwLock<std::collections::HashMap<u32, ConventionStats>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            routing: std::collections::HashMap::new(),
            name_routing: std::collections::HashMap::new(),
            crashed_plugins: std::sync::RwLock::new(std::collections::HashSet::new()),
            metrics: None,
            convention_stats: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Attach metrics for plugin call tracking (Section 11.5).
    pub fn set_metrics(&mut self, metrics: std::sync::Arc<crate::metrics::SomaMetrics>) {
        self.metrics = Some(metrics);
    }

    /// Register a plugin. Checks dependencies are satisfied before registering
    /// (Whitepaper Section 6.7 — topological sort).
    // Future: libloading-based dynamic plugin loading from .so/.dylib files (Section 5.3).
    // Currently only built-in plugins are supported.
    pub fn register(&mut self, plugin: Box<dyn SomaPlugin>) {
        // Check dependencies are satisfied
        let deps = plugin.dependencies();
        let loaded_names: Vec<String> = self.plugins.iter().map(|p| p.name().to_string()).collect();
        for dep in &deps {
            if dep.required && !loaded_names.contains(&dep.name) {
                tracing::error!(
                    plugin = plugin.name(),
                    missing_dep = %dep.name,
                    "Required dependency not loaded — register plugins in dependency order"
                );
                // Still register (don't crash), but warn loudly
            }
        }

        let plugin_idx = self.plugins.len();
        // Convention IDs are namespaced: global_id = plugin_idx * 1000 + local_id
        // This prevents routing conflicts when multiple plugins use the same local IDs.
        let id_offset = (plugin_idx as u32) * 1000;
        let conventions = plugin.conventions();
        let mut registered = 0;
        for conv in &conventions {
            let global_id = id_offset + conv.id;
            if self.routing.contains_key(&global_id) {
                tracing::warn!(
                    plugin = plugin.name(),
                    conv_id = conv.id,
                    global_id = global_id,
                    "Convention ID conflict — skipping"
                );
                continue;
            }
            self.routing.insert(global_id, (plugin_idx, conv.id));
            let full_name = format!("{}.{}", plugin.name(), conv.name);
            self.name_routing.insert(full_name, global_id);
            registered += 1;
        }
        tracing::info!(
            plugin = plugin.name(),
            conventions = registered,
            id_offset = id_offset,
            "Plugin registered"
        );
        self.plugins.push(plugin);
    }

    /// Register multiple plugins in dependency-resolved order (Section 6.7).
    /// Uses topological sort to determine correct loading order.
    pub fn register_all(&mut self, mut plugins: Vec<Box<dyn SomaPlugin>>) {
        // Build dependency graph
        let names: Vec<String> = plugins.iter().map(|p| p.name().to_string()).collect();
        let mut order: Vec<usize> = Vec::new();
        let mut visited = vec![false; plugins.len()];

        // Simple topological sort (DFS-based)
        fn visit(
            idx: usize,
            plugins: &[Box<dyn SomaPlugin>],
            names: &[String],
            visited: &mut [bool],
            order: &mut Vec<usize>,
        ) {
            if visited[idx] { return; }
            visited[idx] = true;

            for dep in plugins[idx].dependencies() {
                if let Some(dep_idx) = names.iter().position(|n| n == &dep.name) {
                    visit(dep_idx, plugins, names, visited, order);
                }
            }
            order.push(idx);
        }

        for i in 0..plugins.len() {
            visit(i, &plugins, &names, &mut visited, &mut order);
        }

        // Register in topological order
        // We need to drain in order, so collect indices and sort
        let mut indexed: Vec<(usize, Box<dyn SomaPlugin>)> = plugins
            .drain(..)
            .enumerate()
            .collect();

        for idx in order {
            if let Some(pos) = indexed.iter().position(|(i, _)| *i == idx) {
                let (_, plugin) = indexed.remove(pos);
                self.register(plugin);
            }
        }
    }

    /// Execute a single step by routing to the correct plugin.
    /// Wrapped in catch_unwind to prevent plugin panics from crashing SOMA.
    /// Crashed plugins are marked and disabled (Whitepaper Section 11.3).
    fn execute_step(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        let (plugin_idx, plugin_conv_id) = self.routing.get(&conv_id)
            .ok_or_else(|| PluginError::NotFound(
                format!("No plugin for convention {}", conv_id)))?;

        let idx = *plugin_idx;
        let cid = *plugin_conv_id;

        // Refuse to call a crashed plugin (Section 11.3: "unloaded")
        {
            let crashed = self.crashed_plugins.read().unwrap_or_else(|e| e.into_inner());
            if crashed.contains(&idx) {
                return Err(PluginError::Failed(format!(
                    "plugin '{}' was disabled after crash",
                    self.plugins[idx].name()
                )));
            }
        }

        let start = std::time::Instant::now();
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.plugins[idx].execute(cid, args)
        })) {
            Ok(result) => {
                if let Some(ref m) = self.metrics {
                    m.record_plugin_call(result.is_ok());
                }
                result
            }
            Err(panic_info) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    format!("plugin '{}' panicked: {}", self.plugins[idx].name(), s)
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    format!("plugin '{}' panicked: {}", self.plugins[idx].name(), s)
                } else {
                    format!("plugin '{}' panicked (unknown cause)", self.plugins[idx].name())
                };
                tracing::error!(conv_id, plugin = self.plugins[idx].name(), "Plugin crash — disabling plugin");
                // Mark as crashed via interior mutability — SOMA continues with reduced capabilities
                if let Ok(mut crashed) = self.crashed_plugins.write() {
                    crashed.insert(idx);
                }
                Err(PluginError::Failed(msg))
            }
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Track per-convention stats (Section 19.3)
        if let Ok(mut stats_map) = self.convention_stats.write() {
            let stats = stats_map.entry(conv_id).or_default();
            stats.call_count += 1;
            stats.total_time_ms += elapsed_ms;
            if outcome.is_err() {
                stats.error_count += 1;
            }
        }

        outcome
    }

    // mark_crashed is handled automatically by execute_step via interior mutability.
    // Crashed plugins are tracked in self.crashed_plugins (RwLock<HashSet<usize>>).

    /// Execute a single step with retry: if the first attempt fails with a retryable
    /// error, sleep 100ms and try once more.
    fn execute_step_with_retry(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match self.execute_step(conv_id, args.clone()) {
            Ok(val) => Ok(val),
            Err(e) if e.is_retryable() => {
                tracing::debug!(conv_id, error = %e, "Retryable error, retrying in 100ms");
                if let Some(ref m) = self.metrics {
                    m.record_plugin_retry();
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                self.execute_step(conv_id, args)
            }
            Err(e) => Err(e),
        }
    }

    /// Execute a full program generated by the mind.
    /// `max_steps` caps the number of steps executed (0 = unlimited).
    pub fn execute_program(&self, steps: &[ProgramStep], max_steps: usize) -> ProgramResult {
        let mut results: Vec<Value> = Vec::new();
        let mut output: Option<Value> = None;
        let mut trace = Vec::new();
        let step_limit = if max_steps == 0 { usize::MAX } else { max_steps };

        for (i, step) in steps.iter().enumerate() {
            if i >= step_limit {
                let err = format!("Program exceeded max_steps limit ({})", max_steps);
                trace.push(TraceEntry {
                    step: i, op: "LIMIT".into(), success: false, summary: err.clone(),
                });
                // Cleanup open handles
                for r in &results {
                    if let Value::Handle(h) = r {
                        unsafe { libc::close(*h as i32); }
                    }
                }
                return ProgramResult {
                    success: false, output: None, trace, error: Some(err),
                };
            }

            if step.conv_id == STOP_ID {
                trace.push(TraceEntry {
                    step: i, op: "STOP".into(), success: true, summary: String::new(),
                });
                break;
            }

            if step.conv_id == EMIT_ID {
                if let ArgValue::Ref(r) = &step.arg0_value {
                    if *r < results.len() {
                        output = Some(results[*r].clone());
                    }
                }
                trace.push(TraceEntry {
                    step: i, op: "EMIT".into(), success: true, summary: String::new(),
                });
                continue;
            }

            // Resolve arguments
            let mut resolved = Vec::new();
            for (_atype, aval) in [
                (&step.arg0_type, &step.arg0_value),
                (&step.arg1_type, &step.arg1_value),
            ] {
                match aval {
                    ArgValue::Span(s) => {
                        // Expand ~ in paths
                        let expanded = if s.starts_with('~') {
                            shellexpand::tilde(s).to_string()
                        } else {
                            s.clone()
                        };
                        resolved.push(Value::String(expanded));
                    }
                    ArgValue::Ref(r) => {
                        if *r < results.len() {
                            resolved.push(results[*r].clone());
                        } else {
                            let err = format!("Step {} invalid ref ${}", i, r);
                            trace.push(TraceEntry {
                                step: i, op: "?".into(), success: false, summary: err.clone(),
                            });
                            return ProgramResult {
                                success: false, output: None, trace, error: Some(err),
                            };
                        }
                    }
                    ArgValue::None => {}
                }
            }

            match self.execute_step_with_retry(step.conv_id as u32, resolved) {
                Ok(result) => {
                    let summary = format!("{}", &result);
                    trace.push(TraceEntry {
                        step: i,
                        op: format!("conv:{}", step.conv_id),
                        success: true,
                        summary,
                    });
                    results.push(result);
                }
                Err(e) => {
                    let err = format!("Step {}: {}", i, e);
                    trace.push(TraceEntry {
                        step: i,
                        op: format!("conv:{}", step.conv_id),
                        success: false,
                        summary: err.clone(),
                    });

                    // Invoke cleanup conventions for previously successful steps
                    // (Whitepaper Section 6.7 — error recovery)
                    for (j, prev_step) in steps[..i].iter().enumerate() {
                        if prev_step.conv_id == EMIT_ID || prev_step.conv_id == STOP_ID {
                            continue;
                        }
                        // Look up the convention's cleanup action
                        let convs = self.conventions();
                        if let Some(conv) = convs.iter().find(|c| c.id == prev_step.conv_id as u32) {
                            if let Some(ref cleanup_spec) = conv.cleanup {
                                let cleanup_id = cleanup_spec.convention_id;
                                if j < results.len() {
                                    let cleanup_args = vec![results[j].clone()];
                                    tracing::debug!(
                                        step = j,
                                        cleanup_conv = cleanup_id,
                                        "Invoking cleanup convention"
                                    );
                                    if let Err(ce) = self.execute_step(cleanup_id, cleanup_args) {
                                        tracing::warn!(
                                            step = j,
                                            cleanup_conv = cleanup_id,
                                            error = %ce,
                                            "Cleanup convention failed"
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // Cleanup open handles (fallback)
                    for r in &results {
                        if let Value::Handle(h) = r {
                            unsafe { libc::close(*h as i32); }
                        }
                    }
                    return ProgramResult {
                        success: false, output: None, trace, error: Some(err),
                    };
                }
            }
        }

        ProgramResult { success: true, output, trace, error: None }
    }

    /// Execute a convention by plugin name + local convention ID (for MCP tool calls).
    /// Resolves to global routing ID via plugin index offset.
    pub fn execute_direct(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        // For backwards compat: try the raw ID first (works for plugin_idx=0)
        if self.routing.contains_key(&conv_id) {
            return self.execute_step_with_retry(conv_id, args);
        }
        // Search all offsets for this local convention ID
        for (idx, _plugin) in self.plugins.iter().enumerate() {
            let global_id = (idx as u32) * 1000 + conv_id;
            if self.routing.contains_key(&global_id) {
                return self.execute_step_with_retry(global_id, args);
            }
        }
        Err(PluginError::NotFound(format!("No plugin for convention {}", conv_id)))
    }

    /// Execute a convention by plugin name + local ID (multi-plugin safe).
    pub fn execute_by_plugin(&self, plugin_name: &str, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        let plugin_idx = self.plugins.iter().position(|p| p.name() == plugin_name)
            .ok_or_else(|| PluginError::NotFound(format!("Plugin '{}' not loaded", plugin_name)))?;
        let global_id = (plugin_idx as u32) * 1000 + conv_id;
        self.execute_step_with_retry(global_id, args)
    }

    /// Resolve a convention by name (Section 5.4).
    /// Format: "plugin_name.convention_name" → global routing ID.
    pub fn resolve_by_name(&self, name: &str) -> Option<u32> {
        self.name_routing.get(name).copied()
    }

    pub fn conventions(&self) -> Vec<Convention> {
        self.plugins.iter().flat_map(|p| p.conventions()).collect()
    }

    /// Get plugin names list.
    pub fn plugin_names(&self) -> Vec<String> {
        self.plugins.iter().map(|p| p.name().to_string()).collect()
    }

    /// Collect checkpoint state from all plugins (Section 7.5).
    pub fn collect_plugin_states(&self) -> Vec<(String, serde_json::Value)> {
        self.plugins.iter()
            .filter_map(|p| {
                p.checkpoint_state().map(|state| (p.name().to_string(), state))
            })
            .collect()
    }

    /// Restore plugin states from checkpoint data.
    pub fn restore_plugin_states(&mut self, states: &[(String, serde_json::Value)]) {
        for (name, state) in states {
            for plugin in self.plugins.iter_mut() {
                if plugin.name() == name {
                    if let Err(e) = plugin.restore_state(state) {
                        tracing::warn!(plugin = name, error = %e, "Failed to restore plugin state");
                    }
                }
            }
        }
    }

    /// Get plugin manifest — list of (name, version) for all loaded plugins.
    pub fn plugin_manifest(&self) -> Vec<(String, String)> {
        self.plugins.iter().map(|p| (p.name().to_string(), p.version().to_string())).collect()
    }

    /// Get per-convention execution stats snapshot (Section 19.3).
    pub fn get_convention_stats(&self) -> std::collections::HashMap<u32, ConventionStats> {
        self.convention_stats.read()
            .map(|s| s.iter().map(|(k, v)| (*k, ConventionStats {
                call_count: v.call_count,
                total_time_ms: v.total_time_ms,
                error_count: v.error_count,
                timeout_count: v.timeout_count,
            })).collect())
            .unwrap_or_default()
    }

    /// Get conventions with plugin name prefix for namespacing (Section 12.2).
    pub fn namespaced_conventions(&self) -> Vec<(String, Convention)> {
        self.plugins.iter()
            .flat_map(|p| {
                let pname = p.name().to_string();
                p.conventions().into_iter().map(move |c| (pname.clone(), c))
            })
            .collect()
    }

    /// Call on_unload for all plugins during shutdown (Section 11.4).
    /// Unloads in reverse registration order (Whitepaper Section 16.2 step 6).
    pub fn unload_all(&mut self) {
        for i in (0..self.plugins.len()).rev() {
            let name = self.plugins[i].name().to_string();
            if let Err(e) = self.plugins[i].on_unload() {
                tracing::warn!(plugin = %name, error = %e, "Plugin unload error");
            } else {
                tracing::debug!(plugin = %name, "Plugin unloaded");
            }
        }
    }
}

// shellexpand for ~ paths
mod shellexpand {
    pub fn tilde(path: &str) -> String {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                return format!("{}/{}", home, rest);
            }
        }
        if path == "~" {
            if let Ok(home) = std::env::var("HOME") {
                return home;
            }
        }
        path.to_string()
    }
}
