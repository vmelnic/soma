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
#[derive(Debug)]
pub struct ConventionStats {
    pub call_count: u64,
    pub total_time_ms: u64,
    pub error_count: u64,
    pub timeout_count: u64,
    /// Ring buffer of recent durations (last 100) for percentile tracking.
    recent_durations: Vec<u64>,
}

impl Default for ConventionStats {
    fn default() -> Self {
        Self {
            call_count: 0,
            total_time_ms: 0,
            error_count: 0,
            timeout_count: 0,
            recent_durations: Vec::with_capacity(100),
        }
    }
}

const STATS_RING_SIZE: usize = 100;

impl ConventionStats {
    /// Record a duration sample into the ring buffer.
    pub fn record_duration(&mut self, duration_ms: u64) {
        if self.recent_durations.len() >= STATS_RING_SIZE {
            // Overwrite oldest entry (ring buffer behavior)
            let idx = (self.call_count as usize - 1) % STATS_RING_SIZE;
            self.recent_durations[idx] = duration_ms;
        } else {
            self.recent_durations.push(duration_ms);
        }
    }

    /// Compute average duration from recent samples.
    pub fn avg_duration_ms(&self) -> f64 {
        if self.recent_durations.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.recent_durations.iter().sum();
        sum as f64 / self.recent_durations.len() as f64
    }

    /// Compute p50 (median) from recent samples.
    pub fn p50_duration_ms(&self) -> u64 {
        self.percentile(50)
    }

    /// Compute p99 from recent samples.
    pub fn p99_duration_ms(&self) -> u64 {
        self.percentile(99)
    }

    fn percentile(&self, pct: usize) -> u64 {
        if self.recent_durations.is_empty() {
            return 0;
        }
        let mut sorted = self.recent_durations.clone();
        sorted.sort_unstable();
        let idx = (pct * sorted.len() / 100).min(sorted.len() - 1);
        sorted[idx]
    }
}

pub struct PluginManager {
    plugins: Vec<Box<dyn SomaPlugin>>,
    /// Maps global convention ID (plugin_idx*1000+local_id) -> (plugin_index, plugin_convention_id)
    routing: std::collections::HashMap<u32, (usize, u32)>,
    /// Tracks crashed plugins via interior mutability — execute_step(&self) can mark
    /// crashed plugins without &mut self (Section 11.3).
    crashed_plugins: std::sync::RwLock<std::collections::HashSet<usize>>,
    /// Name-based convention lookup (Section 5.4). Maps "plugin.convention" → global_id.
    name_routing: std::collections::HashMap<String, u32>,
    /// Maps model catalog IDs → global routing IDs. Populated by `build_catalog_routing`.
    /// When the Mind outputs catalog_id=34 (e.g. "postgres.execute"), this maps to
    /// the correct global routing ID (e.g. 1001).
    catalog_routing: std::collections::HashMap<u32, u32>,
    /// Optional metrics reference for tracking plugin calls
    metrics: Option<std::sync::Arc<crate::metrics::SomaMetrics>>,
    /// Per-convention stats: global_id -> stats (Section 19.3)
    convention_stats: std::sync::RwLock<std::collections::HashMap<u32, ConventionStats>>,
    /// Plugins whose execution is denied (Section 12.2 permission enforcement).
    denied_plugins: std::sync::RwLock<std::collections::HashSet<String>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            routing: std::collections::HashMap::new(),
            name_routing: std::collections::HashMap::new(),
            catalog_routing: std::collections::HashMap::new(),
            crashed_plugins: std::sync::RwLock::new(std::collections::HashSet::new()),
            metrics: None,
            convention_stats: std::sync::RwLock::new(std::collections::HashMap::new()),
            denied_plugins: std::sync::RwLock::new(std::collections::HashSet::new()),
        }
    }

    /// Attach metrics for plugin call tracking (Section 11.5).
    pub fn set_metrics(&mut self, metrics: std::sync::Arc<crate::metrics::SomaMetrics>) {
        self.metrics = Some(metrics);
    }

    /// Populate denied_plugins from a list of plugin names (Section 12.2).
    /// Called during startup with config.security.denied_plugins.
    pub fn set_denied_plugins(&self, names: &[String]) {
        if let Ok(mut denied) = self.denied_plugins.write() {
            denied.clear();
            for name in names {
                denied.insert(name.clone());
            }
            if !names.is_empty() {
                tracing::info!(count = names.len(), "Denied plugins configured");
            }
        }
    }

    /// Deny a single plugin by name at runtime.
    pub fn deny_plugin(&self, name: &str) {
        if let Ok(mut denied) = self.denied_plugins.write() {
            denied.insert(name.to_string());
            tracing::info!(plugin = name, "Plugin denied");
        }
    }

    /// Allow a previously denied plugin.
    pub fn allow_plugin(&self, name: &str) {
        if let Ok(mut denied) = self.denied_plugins.write() {
            denied.remove(name);
            tracing::info!(plugin = name, "Plugin allowed");
        }
    }

    /// Check whether a plugin is denied.
    pub fn is_plugin_denied(&self, name: &str) -> bool {
        self.denied_plugins
            .read()
            .map(|d| d.contains(name))
            .unwrap_or(false)
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
                    "Required dependency not loaded — refusing to register plugin"
                );
                return;
            }
        }

        // Validate plugin config against its own schema (Section 5.2)
        validate_plugin_config(&*plugin);

        // Surface LoRA weights if the plugin provides them (Section 7.3)
        if let Some(lora_data) = plugin.lora_weights() {
            tracing::info!(
                plugin = plugin.name(),
                lora_bytes = lora_data.len(),
                "Plugin provides LoRA weights — available for Mind attachment"
            );
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
    /// Detects dependency cycles and skips cyclic plugins.
    pub fn register_all(&mut self, mut plugins: Vec<Box<dyn SomaPlugin>>) {
        // Build dependency graph
        let names: Vec<String> = plugins.iter().map(|p| p.name().to_string()).collect();

        // Cycle detection using DFS with three-color marking:
        // 0 = white (unvisited), 1 = gray (in progress), 2 = black (done)
        let n = plugins.len();
        let mut color = vec![0u8; n];
        let mut order: Vec<usize> = Vec::new();
        let mut in_cycle: Vec<bool> = vec![false; n];

        fn visit_with_cycle_check(
            idx: usize,
            plugins: &[Box<dyn SomaPlugin>],
            names: &[String],
            color: &mut [u8],
            order: &mut Vec<usize>,
            in_cycle: &mut [bool],
        ) -> bool {
            if color[idx] == 2 { return false; } // black — already processed
            if color[idx] == 1 { return true; }   // gray — cycle detected

            color[idx] = 1; // mark gray (in progress)

            for dep in plugins[idx].dependencies() {
                if let Some(dep_idx) = names.iter().position(|n| n == &dep.name) {
                    if visit_with_cycle_check(dep_idx, plugins, names, color, order, in_cycle) {
                        // Cycle found — mark both this node and dep as cyclic
                        in_cycle[idx] = true;
                        in_cycle[dep_idx] = true;
                        return true;
                    }
                }
            }

            color[idx] = 2; // mark black (done)
            order.push(idx);
            false
        }

        for i in 0..n {
            if color[i] == 0 {
                visit_with_cycle_check(i, &plugins, &names, &mut color, &mut order, &mut in_cycle);
            }
        }

        // Log and skip plugins involved in cycles
        for i in 0..n {
            if in_cycle[i] {
                tracing::error!(
                    plugin = plugins[i].name(),
                    "Dependency cycle detected — skipping plugin"
                );
            }
        }

        // Register in topological order, skipping cyclic plugins
        let mut indexed: Vec<(usize, Box<dyn SomaPlugin>)> = plugins
            .drain(..)
            .enumerate()
            .collect();

        for idx in order {
            if in_cycle[idx] {
                continue;
            }
            if let Some(pos) = indexed.iter().position(|(i, _)| *i == idx) {
                let (_, plugin) = indexed.remove(pos);
                self.register(plugin);
            }
        }
    }

    /// Execute a single step by routing to the correct plugin.
    /// Wrapped in catch_unwind to prevent plugin panics from crashing SOMA.
    /// Crashed plugins are marked and disabled (Whitepaper Section 11.3).
    ///
    /// Gap #29: When a convention declares max_latency_ms and a tokio runtime is
    /// available, execution is wrapped with tokio::time::timeout for preemptive
    /// timeout enforcement (not just post-execution checking).
    ///
    /// Gap #30: Permission enforcement is real — denied plugins are refused
    /// via the denied_plugins set, and plugin permission declarations are logged.
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

        let plugin_name = self.plugins[idx].name().to_string();

        // Permission enforcement (Section 12.2, Gap #30)
        // Check denied_plugins list — if plugin is denied, refuse execution.
        {
            let denied = self.denied_plugins.read().unwrap_or_else(|e| e.into_inner());
            if denied.contains(&plugin_name) {
                tracing::warn!(
                    plugin = %plugin_name,
                    conv_id,
                    "Plugin execution denied — plugin is in denied_plugins list"
                );
                return Err(PluginError::PermissionDenied(format!(
                    "plugin '{}' is denied by configuration",
                    plugin_name
                )));
            }
        }

        // Log permission declarations for auditing (Section 12.2)
        {
            let perms = self.plugins[idx].permissions();
            if !perms.filesystem.is_empty() || !perms.network.is_empty()
                || !perms.env_vars.is_empty() || perms.process_spawn
            {
                tracing::debug!(
                    plugin = %plugin_name,
                    conv_id,
                    fs_paths = ?perms.filesystem,
                    net = ?perms.network,
                    env = ?perms.env_vars,
                    spawn = perms.process_spawn,
                    "Plugin declares permissions"
                );
            }
        }

        // Determine max_latency_ms for preemptive timeout (Gap #29)
        let timeout_ms = {
            let plugin_convs = self.plugins[idx].conventions();
            plugin_convs.iter()
                .find(|c| c.id == cid)
                .map(|c| c.max_latency_ms as u64)
                .unwrap_or(0)
        };

        let start = std::time::Instant::now();

        // If the convention declares a max_latency_ms and we have a tokio runtime,
        // use preemptive timeout via execute_async + tokio::time::timeout (Gap #29).
        let outcome = if timeout_ms > 0 {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let plugin = &self.plugins[idx];
                let timeout_dur = std::time::Duration::from_millis(timeout_ms);
                let timed_result = tokio::task::block_in_place(|| {
                    handle.block_on(async {
                        tokio::time::timeout(
                            timeout_dur,
                            plugin.execute_async(cid, args),
                        ).await
                    })
                });
                match timed_result {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        tracing::warn!(
                            plugin = %plugin_name,
                            conv_id,
                            timeout_ms,
                            "Convention execution timed out (preemptive)"
                        );
                        if let Ok(mut stats_map) = self.convention_stats.write() {
                            let stats = stats_map.entry(conv_id).or_default();
                            stats.timeout_count += 1;
                        }
                        Err(PluginError::Failed(format!(
                            "plugin '{}' convention {} timed out after {}ms",
                            plugin_name, conv_id, timeout_ms
                        )))
                    }
                }
            } else {
                // No tokio runtime — fall back to sync with catch_unwind
                Self::execute_with_catch_unwind(
                    &self.plugins[idx], &plugin_name, cid, args,
                    conv_id, &self.crashed_plugins, idx,
                )
            }
        } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // No timeout but tokio available — use async path to avoid reactor issues
            let plugin = &self.plugins[idx];
            tokio::task::block_in_place(|| {
                handle.block_on(plugin.execute_async(cid, args))
            })
        } else {
            // No timeout, no tokio — sync execution with panic catching
            Self::execute_with_catch_unwind(
                &self.plugins[idx], &plugin_name, cid, args,
                conv_id, &self.crashed_plugins, idx,
            )
        };

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Record per-plugin and global metrics
        if let Some(ref m) = self.metrics {
            m.record_plugin_call_named(&plugin_name, elapsed_ms, outcome.is_ok());
        }

        // Post-execution timeout check for conventions without preemptive timeout
        // (when no tokio runtime was available, or timeout_ms was 0 but convention
        // still has a max_latency_ms for advisory logging)
        if timeout_ms == 0 {
            let plugin_convs = self.plugins[idx].conventions();
            if let Some(conv) = plugin_convs.iter().find(|c| c.id == cid) {
                if conv.max_latency_ms > 0 && elapsed_ms > conv.max_latency_ms as u64 {
                    tracing::warn!(
                        plugin = self.plugins[idx].name(),
                        conv_id,
                        elapsed_ms,
                        max_latency_ms = conv.max_latency_ms,
                        "Convention exceeded max_latency_ms (post-execution)"
                    );
                    if let Ok(mut stats_map) = self.convention_stats.write() {
                        let stats = stats_map.entry(conv_id).or_default();
                        stats.timeout_count += 1;
                    }
                }
            }
        }

        // Track per-convention stats (Section 19.3)
        if let Ok(mut stats_map) = self.convention_stats.write() {
            let stats = stats_map.entry(conv_id).or_default();
            stats.call_count += 1;
            stats.total_time_ms += elapsed_ms;
            stats.record_duration(elapsed_ms);
            if outcome.is_err() {
                stats.error_count += 1;
            }
        }

        outcome
    }

    /// Execute a plugin call with catch_unwind for panic safety.
    /// Extracted helper to avoid duplication between timeout and non-timeout paths.
    fn execute_with_catch_unwind(
        plugin: &Box<dyn SomaPlugin>,
        plugin_name: &str,
        cid: u32,
        args: Vec<Value>,
        conv_id: u32,
        crashed_plugins: &std::sync::RwLock<std::collections::HashSet<usize>>,
        idx: usize,
    ) -> Result<Value, PluginError> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plugin.execute(cid, args)
        })) {
            Ok(result) => result,
            Err(panic_info) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    format!("plugin '{}' panicked: {}", plugin_name, s)
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    format!("plugin '{}' panicked: {}", plugin_name, s)
                } else {
                    format!("plugin '{}' panicked (unknown cause)", plugin_name)
                };
                tracing::error!(conv_id, plugin = %plugin_name, "Plugin crash — disabling plugin");
                // Mark as crashed via interior mutability — SOMA continues with reduced capabilities
                if let Ok(mut crashed) = crashed_plugins.write() {
                    crashed.insert(idx);
                }
                Err(PluginError::Failed(msg))
            }
        }
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
                    ArgValue::Literal(s) => {
                        resolved.push(Value::String(s.clone()));
                    }
                    ArgValue::None => {}
                }
            }

            // Resolve model catalog ID to plugin manager global ID
            let global_id = self.resolve_catalog_id(step.conv_id as u32);
            match self.execute_step_with_retry(global_id, resolved) {
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
                    // in REVERSE order (Whitepaper Section 6.7 — error recovery).
                    // Reverse order ensures resources are released in LIFO order.
                    for j in (0..i).rev() {
                        let prev_step = &steps[j];
                        if prev_step.conv_id == EMIT_ID || prev_step.conv_id == STOP_ID {
                            continue;
                        }
                        // The model's conv_id is a global ID. Convert to local for convention lookup.
                        let global_step_id = prev_step.conv_id as u32;
                        // Find which plugin owns this convention via the routing table
                        if let Some(&(owner_plugin_idx, local_conv_id)) = self.routing.get(&global_step_id) {
                            let plugin_convs = self.plugins[owner_plugin_idx].conventions();
                            if let Some(conv) = plugin_convs.iter().find(|c| c.id == local_conv_id) {
                                if let Some(ref cleanup_spec) = conv.cleanup {
                                    // CleanupSpec convention_id is local to the plugin.
                                    // Compute global cleanup ID for execute_step.
                                    let global_cleanup_id = (owner_plugin_idx as u32) * 1000 + cleanup_spec.convention_id;
                                    if j < results.len() {
                                        let cleanup_args = vec![results[j].clone()];
                                        tracing::debug!(
                                            step = j,
                                            cleanup_conv = global_cleanup_id,
                                            "Invoking cleanup convention"
                                        );
                                        if let Err(ce) = self.execute_step(global_cleanup_id, cleanup_args) {
                                            tracing::warn!(
                                                step = j,
                                                cleanup_conv = global_cleanup_id,
                                                error = %ce,
                                                "Cleanup convention failed"
                                            );
                                        }
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

    /// Async variant of execute_by_plugin — uses plugin's execute_async for I/O-bound operations.
    pub async fn execute_by_plugin_async(&self, plugin_name: &str, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        let plugin_idx = self.plugins.iter().position(|p| p.name() == plugin_name)
            .ok_or_else(|| PluginError::NotFound(format!("Plugin '{}' not loaded", plugin_name)))?;

        // Check if plugin is crashed
        if self.crashed_plugins.read().unwrap().contains(&plugin_idx) {
            return Err(PluginError::Failed(format!("Plugin '{}' is crashed", plugin_name)));
        }

        let plugin = &self.plugins[plugin_idx];
        plugin.execute_async(conv_id, args).await
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

    /// Build a mapping from model catalog IDs to plugin manager global routing IDs.
    /// Call this after all plugins are loaded and the model catalog is available.
    /// Each catalog entry has a `full_name` like "postgres.query" and a `catalog_id`.
    /// This maps catalog_id → name_routing[full_name] (the global routing ID).
    pub fn build_catalog_routing(&mut self, catalog: &[super::super::mind::CatalogEntry]) {
        self.catalog_routing.clear();
        for entry in catalog {
            // Try to find this convention name in the loaded plugins' name_routing
            if let Some(&global_id) = self.name_routing.get(&entry.name) {
                self.catalog_routing.insert(entry.id as u32, global_id);
            }
            // Also try function field (some catalogs use this)
            else if let Some(&global_id) = self.name_routing.get(&entry.function) {
                self.catalog_routing.insert(entry.id as u32, global_id);
            }
        }
        tracing::info!(
            mapped = self.catalog_routing.len(),
            total = catalog.len(),
            "Catalog-to-plugin routing built"
        );
    }

    /// Resolve a model catalog ID to a global routing ID.
    /// Returns the global ID if mapped, otherwise returns the catalog ID unchanged
    /// (backwards compatible with models that use global IDs directly).
    pub fn resolve_catalog_id(&self, catalog_id: u32) -> u32 {
        self.catalog_routing.get(&catalog_id).copied().unwrap_or(catalog_id)
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
                recent_durations: v.recent_durations.clone(),
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

    /// Check which plugins provide LoRA weights (Section 7.3).
    /// Returns a list of plugin names that have LoRA knowledge available.
    pub fn plugins_with_lora_weights(&self) -> Vec<String> {
        self.plugins.iter()
            .filter(|p| p.lora_weights().is_some())
            .map(|p| p.name().to_string())
            .collect()
    }

    /// Get LoRA weights bytes for a specific plugin by name (Section 7.3).
    pub fn get_plugin_lora_weights(&self, name: &str) -> Option<Vec<u8>> {
        self.plugins.iter()
            .find(|p| p.name() == name)
            .and_then(|p| p.lora_weights())
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

    /// Unregister a plugin by name. Calls on_unload, removes from plugins Vec,
    /// and cleans up routing and name_routing entries.
    pub fn unregister(&mut self, name: &str) -> Result<(), String> {
        let idx = self.plugins.iter().position(|p| p.name() == name)
            .ok_or_else(|| format!("Plugin not found: {}", name))?;

        // Call on_unload
        if let Err(e) = self.plugins[idx].on_unload() {
            tracing::warn!(plugin = %name, error = %e, "Plugin unload error during unregister");
        }

        // Compute the ID offset for this plugin
        let id_offset = (idx as u32) * 1000;

        // Remove routing entries for this plugin
        let conventions = self.plugins[idx].conventions();
        for conv in &conventions {
            let global_id = id_offset + conv.id;
            self.routing.remove(&global_id);
            let full_name = format!("{}.{}", name, conv.name);
            self.name_routing.remove(&full_name);
        }

        // Remove the plugin
        self.plugins.remove(idx);

        tracing::info!(plugin = %name, "Plugin unregistered");
        Ok(())
    }

    /// Dead plugin detection stub (Section 11.3).
    /// Checks each plugin's error rate from ConventionStats and returns warnings
    /// for plugins with >50% error rate.
    pub fn check_plugin_health(&self) -> Vec<(String, &str)> {
        let mut warnings = Vec::new();
        let stats_map = match self.convention_stats.read() {
            Ok(s) => s,
            Err(_) => return warnings,
        };

        for (idx, plugin) in self.plugins.iter().enumerate() {
            let id_offset = (idx as u32) * 1000;
            let mut total_calls: u64 = 0;
            let mut total_errors: u64 = 0;

            // Aggregate stats across all conventions owned by this plugin
            for (&global_id, stats) in stats_map.iter() {
                if global_id >= id_offset && global_id < id_offset + 1000 {
                    total_calls += stats.call_count;
                    total_errors += stats.error_count;
                }
            }

            if total_calls > 0 && total_errors * 100 / total_calls > 50 {
                warnings.push((
                    plugin.name().to_string(),
                    "error rate exceeds 50% — plugin may be dead or malfunctioning",
                ));
            }
        }

        warnings
    }
}

/// Validate a plugin's config against its declared schema (Section 5.2).
/// Called during register() to catch misconfigurations early.
fn validate_plugin_config(plugin: &dyn SomaPlugin) {
    if let Some(schema) = plugin.config_schema() {
        let config = super::interface::PluginConfig::default();
        let errors = config.validate(&schema);
        if !errors.is_empty() {
            for err in &errors {
                tracing::warn!(
                    plugin = plugin.name(),
                    error = %err,
                    "Plugin config validation warning"
                );
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
