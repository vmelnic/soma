//! Plugin manager -- registers plugins, routes convention calls, and executes Mind-generated programs.
//!
//! Convention IDs are namespaced by plugin index: `global_id = plugin_idx * 1000 + local_id`.
//! This prevents routing conflicts when multiple plugins define conventions with the same local IDs.
//! Crash recovery uses interior mutability (`RwLock<HashSet>`) so `execute_step(&self)` can
//! disable a crashed plugin without requiring `&mut self`.

use super::interface::{Convention, PluginError, SomaPlugin, Value};
use crate::mind::{ArgValue, ProgramStep, EMIT_ID, STOP_ID};

/// Outcome of executing a full Mind-generated program through the plugin system.
#[derive(serde::Serialize)]
pub struct ProgramResult {
    pub success: bool,
    /// Final emitted value (set by EMIT steps).
    pub output: Option<Value>,
    /// Per-step execution trace for debugging and experience recording.
    pub trace: Vec<TraceEntry>,
    pub error: Option<String>,
}

/// Single step within a program execution trace.
#[derive(serde::Serialize)]
pub struct TraceEntry {
    pub step: usize,
    pub op: String,
    pub success: bool,
    pub summary: String,
}

/// Per-convention execution statistics for metrics and proprioception (Section 19.3).
#[derive(Debug)]
pub struct ConventionStats {
    pub call_count: u64,
    pub total_time_ms: u64,
    pub error_count: u64,
    pub timeout_count: u64,
    /// Circular buffer of recent durations (capped at 100) for percentile computation.
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
    /// Record a duration sample, overwriting the oldest entry when the buffer is full.
    pub fn record_duration(&mut self, duration_ms: u64) {
        if self.recent_durations.len() >= STATS_RING_SIZE {
            #[allow(clippy::cast_possible_truncation)] // ring buffer index always < 100
            let idx = (self.call_count as usize - 1) % STATS_RING_SIZE;
            self.recent_durations[idx] = duration_ms;
        } else {
            self.recent_durations.push(duration_ms);
        }
    }

    /// Compute average duration from the recent samples ring buffer.
    #[allow(dead_code)] // Spec Section 19.3 -- exposed via metrics/proprioception
    #[allow(clippy::cast_precision_loss)] // duration values fit comfortably in f64
    pub fn avg_duration_ms(&self) -> f64 {
        if self.recent_durations.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.recent_durations.iter().sum();
        sum as f64 / self.recent_durations.len() as f64
    }

    /// Compute p50 (median) duration from recent samples.
    #[allow(dead_code)] // Spec Section 19.3 -- exposed via metrics/proprioception
    pub fn p50_duration_ms(&self) -> u64 {
        self.percentile(50)
    }

    /// Compute p99 duration from recent samples.
    #[allow(dead_code)] // Spec Section 19.3 -- exposed via metrics/proprioception
    pub fn p99_duration_ms(&self) -> u64 {
        self.percentile(99)
    }

    /// Sort-based percentile computation over the ring buffer.
    #[allow(dead_code)] // Used by p50/p99 methods above
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

/// Central registry that owns all loaded plugins and routes convention calls.
///
/// Wrapped in `Arc<RwLock<PluginManager>>` at the runtime level to allow concurrent reads
/// and write-locked registration of new plugins (e.g., via MCP Bridge). Interior mutability
/// via `RwLock` fields allows `execute_step(&self)` to record crashes and stats without `&mut self`.
pub struct PluginManager {
    plugins: Vec<Box<dyn SomaPlugin>>,
    /// Global convention ID (`plugin_idx * 1000 + local_id`) -> (`plugin_index`, `local_conv_id`).
    routing: std::collections::HashMap<u32, (usize, u32)>,
    /// Indices of plugins disabled after a panic (Section 11.3). Interior mutability lets
    /// `execute_step(&self)` mark crashes without requiring `&mut self`.
    crashed_plugins: std::sync::RwLock<std::collections::HashSet<usize>>,
    /// Name-based lookup: "plugin.convention" -> global routing ID (Section 5.4).
    name_routing: std::collections::HashMap<String, u32>,
    /// Model catalog ID -> global routing ID. Bridges the Mind's output IDs (from training
    /// catalog) to the runtime's plugin-index-based routing IDs.
    catalog_routing: std::collections::HashMap<u32, u32>,
    metrics: Option<std::sync::Arc<crate::metrics::SomaMetrics>>,
    /// Per-convention execution stats keyed by global ID (Section 19.3).
    convention_stats: std::sync::RwLock<std::collections::HashMap<u32, ConventionStats>>,
    /// Plugin names denied from execution (Section 12.2 permission enforcement).
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

    /// Attach shared metrics for recording plugin call durations and outcomes.
    pub fn set_metrics(&mut self, metrics: std::sync::Arc<crate::metrics::SomaMetrics>) {
        self.metrics = Some(metrics);
    }

    /// Populate the denied-plugins set from configuration (Section 12.2).
    #[allow(dead_code)] // Spec Section 12.2 -- called during startup with config
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

    /// Deny a single plugin by name at runtime (e.g., via MCP admin command).
    #[allow(dead_code)] // Spec Section 12.2 -- runtime plugin denial via MCP
    pub fn deny_plugin(&self, name: &str) {
        if let Ok(mut denied) = self.denied_plugins.write() {
            denied.insert(name.to_string());
            tracing::info!(plugin = name, "Plugin denied");
        }
    }

    /// Re-enable a previously denied plugin.
    #[allow(dead_code)] // Spec Section 12.2 -- runtime plugin re-enable via MCP
    pub fn allow_plugin(&self, name: &str) {
        if let Ok(mut denied) = self.denied_plugins.write() {
            denied.remove(name);
            tracing::info!(plugin = name, "Plugin allowed");
        }
    }

    /// Check whether a plugin is currently denied.
    #[allow(dead_code)] // Spec Section 12.2 -- queried by MCP tools
    pub fn is_plugin_denied(&self, name: &str) -> bool {
        self.denied_plugins
            .read()
            .map(|d| d.contains(name))
            .unwrap_or(false)
    }

    /// Register a plugin after verifying its dependencies are already loaded.
    ///
    /// Convention IDs are offset by `plugin_idx * 1000` to create a collision-free global
    /// namespace. For example, plugin at index 2 gets IDs 2000..2999. Both numeric routing
    /// and name-based routing ("plugin.convention") are populated.
    pub fn register(&mut self, plugin: Box<dyn SomaPlugin>) {
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
        #[allow(clippy::cast_possible_truncation)] // plugin count is small
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
    ///
    /// Uses DFS-based topological sort with three-color cycle detection:
    /// white (0) = unvisited, gray (1) = in current DFS path, black (2) = finished.
    /// A gray->gray edge indicates a cycle; cyclic plugins are skipped with an error log.
    #[allow(dead_code)] // Spec Section 6.7 -- batch plugin registration with toposort
    pub fn register_all(&mut self, mut plugins: Vec<Box<dyn SomaPlugin>>) {
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
                if let Some(dep_idx) = names.iter().position(|n| n == &dep.name)
                    && visit_with_cycle_check(dep_idx, plugins, names, color, order, in_cycle) {
                        // Cycle found — mark both this node and dep as cyclic
                        in_cycle[idx] = true;
                        in_cycle[dep_idx] = true;
                        return true;
                    }
            }

            color[idx] = 2; // mark black (done)
            order.push(idx);
            false
        }

        // Build dependency graph
        let names: Vec<String> = plugins.iter().map(|p| p.name().to_string()).collect();
        let n = plugins.len();
        let mut color = vec![0u8; n];
        let mut order: Vec<usize> = Vec::new();
        let mut in_cycle: Vec<bool> = vec![false; n];

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
        let mut indexed: Vec<(usize, Box<dyn SomaPlugin>)> = std::mem::take(&mut plugins)
            .into_iter()
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

    /// Route a convention call to its owning plugin and execute it.
    ///
    /// Safety layers applied in order:
    /// 1. Crashed plugin check -- refuses calls to previously-panicked plugins
    /// 2. Permission enforcement -- checks the `denied_plugins` set
    /// 3. Preemptive timeout -- wraps in `tokio::time::timeout` when `max_latency_ms > 0`
    /// 4. Panic isolation -- `catch_unwind` prevents plugin panics from crashing SOMA
    /// 5. Stats recording -- updates per-convention metrics and ring buffer
    #[allow(clippy::too_many_lines)]
    fn execute_step(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        let (plugin_idx, plugin_conv_id) = self.routing.get(&conv_id)
            .ok_or_else(|| PluginError::NotFound(
                format!("No plugin for convention {conv_id}")))?;

        let idx = *plugin_idx;
        let cid = *plugin_conv_id;

        {
            let crashed = self.crashed_plugins.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if crashed.contains(&idx) {
                return Err(PluginError::Failed(format!(
                    "plugin '{}' was disabled after crash",
                    self.plugins[idx].name()
                )));
            }
        }

        let plugin_name = self.plugins[idx].name().to_string();

        {
            let denied = self.denied_plugins.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if denied.contains(&plugin_name) {
                tracing::warn!(
                    plugin = %plugin_name,
                    conv_id,
                    "Plugin execution denied — plugin is in denied_plugins list"
                );
                return Err(PluginError::PermissionDenied(format!(
                    "plugin '{plugin_name}' is denied by configuration"
                )));
            }
        }

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

        let timeout_ms = {
            let plugin_convs = self.plugins[idx].conventions();
            plugin_convs.iter()
                .find(|c| c.id == cid)
                .map_or(0, |c| u64::from(c.max_latency_ms))
        };

        let start = std::time::Instant::now();

        // Preemptive timeout when tokio is available; sync catch_unwind fallback otherwise
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
                            "plugin '{plugin_name}' convention {conv_id} timed out after {timeout_ms}ms"
                        )))
                    }
                }
            } else {
                Self::execute_with_catch_unwind(
                    &self.plugins[idx], &plugin_name, cid, args,
                    conv_id, &self.crashed_plugins, idx,
                )
            }
        } else if let Ok(_handle) = tokio::runtime::Handle::try_current() {
            Self::execute_with_catch_unwind(
                &self.plugins[idx], &plugin_name, cid, args,
                conv_id, &self.crashed_plugins, idx,
            )
        } else {
            Self::execute_with_catch_unwind(
                &self.plugins[idx], &plugin_name, cid, args,
                conv_id, &self.crashed_plugins, idx,
            )
        };

        #[allow(clippy::cast_possible_truncation)] // plugin calls won't exceed u64::MAX ms
        let elapsed_ms = start.elapsed().as_millis() as u64;

        if let Some(ref m) = self.metrics {
            m.record_plugin_call_named(&plugin_name, elapsed_ms, outcome.is_ok());
        }

        // Advisory post-execution timeout check when preemptive timeout was not used
        if timeout_ms == 0 {
            let plugin_convs = self.plugins[idx].conventions();
            if let Some(conv) = plugin_convs.iter().find(|c| c.id == cid)
                && conv.max_latency_ms > 0 && elapsed_ms > u64::from(conv.max_latency_ms) {
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

    /// Execute a plugin call wrapped in `catch_unwind` for panic isolation.
    /// On panic, marks the plugin as crashed so future calls are refused (Section 11.3).
    #[allow(clippy::borrowed_box)] // Box<dyn SomaPlugin> needed for catch_unwind AssertUnwindSafe
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
                let msg = panic_info.downcast_ref::<&str>().map_or_else(
                    || panic_info.downcast_ref::<String>().map_or_else(
                        || format!("plugin '{plugin_name}' panicked (unknown cause)"),
                        |s| format!("plugin '{plugin_name}' panicked: {s}"),
                    ),
                    |s| format!("plugin '{plugin_name}' panicked: {s}"),
                );
                tracing::error!(conv_id, plugin = %plugin_name, "Plugin crash -- disabling plugin");
                if let Ok(mut crashed) = crashed_plugins.write() {
                    crashed.insert(idx);
                }
                Err(PluginError::Failed(msg))
            }
        }
    }

    /// Execute a step with one automatic retry for transient errors (100ms backoff).
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

    /// Execute a full Mind-generated program, stepping through each `ProgramStep` in order.
    ///
    /// Handles STOP (halt), EMIT (set output), and convention calls. On failure, invokes
    /// cleanup conventions in reverse order (LIFO) for any steps that declared a `CleanupSpec`,
    /// then closes any remaining open handles as a safety net.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // conv_id/handle casts are bounded
    pub fn execute_program(&self, steps: &[ProgramStep], max_steps: usize) -> ProgramResult {
        let mut results: Vec<Value> = Vec::new();
        let mut output: Option<Value> = None;
        let mut trace = Vec::new();
        let step_limit = if max_steps == 0 { usize::MAX } else { max_steps };

        for (i, step) in steps.iter().enumerate() {
            if i >= step_limit {
                let err = format!("Program exceeded max_steps limit ({max_steps})");
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
                if let ArgValue::Ref(r) = &step.arg0_value
                    && *r < results.len() {
                        output = Some(results[*r].clone());
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
                            shellexpand::tilde(s).clone()
                        } else {
                            s.clone()
                        };
                        resolved.push(Value::String(expanded));
                    }
                    ArgValue::Ref(r) => {
                        if *r < results.len() {
                            resolved.push(results[*r].clone());
                        } else {
                            let err = format!("Step {i} invalid ref ${r}");
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
                    let err = format!("Step {i}: {e}");
                    trace.push(TraceEntry {
                        step: i,
                        op: format!("conv:{}", step.conv_id),
                        success: false,
                        summary: err.clone(),
                    });

                    // Cleanup in reverse order (LIFO) so resources are released correctly
                    for j in (0..i).rev() {
                        let prev_step = &steps[j];
                        if prev_step.conv_id == EMIT_ID || prev_step.conv_id == STOP_ID {
                            continue;
                        }
                        let global_step_id = prev_step.conv_id as u32;
                        if let Some(&(owner_plugin_idx, local_conv_id)) = self.routing.get(&global_step_id) {
                            let plugin_convs = self.plugins[owner_plugin_idx].conventions();
                            if let Some(conv) = plugin_convs.iter().find(|c| c.id == local_conv_id)
                                && let Some(ref cleanup_spec) = conv.cleanup {
                                    // Convert local cleanup convention ID to global routing ID
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

    /// Execute a convention by local ID, searching all plugin offsets if the raw ID is not found.
    /// Used by MCP tool calls that reference conventions by local ID.
    #[allow(dead_code)] // Spec Section 5.4 -- direct convention execution via MCP
    #[allow(clippy::cast_possible_truncation)] // plugin count is small
    pub fn execute_direct(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        // Try raw ID first (works for plugin_idx=0 where global == local)
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
        Err(PluginError::NotFound(format!("No plugin for convention {conv_id}")))
    }

    /// Execute a convention by plugin name + local ID (unambiguous across plugins).
    #[allow(dead_code)] // Spec Section 5.4 -- plugin-namespaced convention execution
    #[allow(clippy::cast_possible_truncation)] // plugin count is small
    pub fn execute_by_plugin(&self, plugin_name: &str, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        let plugin_idx = self.plugins.iter().position(|p| p.name() == plugin_name)
            .ok_or_else(|| PluginError::NotFound(format!("Plugin '{plugin_name}' not loaded")))?;
        let global_id = (plugin_idx as u32) * 1000 + conv_id;
        self.execute_step_with_retry(global_id, args)
    }

    /// Async variant of `execute_by_plugin` for I/O-bound plugin operations.
    pub async fn execute_by_plugin_async(&self, plugin_name: &str, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        let plugin_idx = self.plugins.iter().position(|p| p.name() == plugin_name)
            .ok_or_else(|| PluginError::NotFound(format!("Plugin '{plugin_name}' not loaded")))?;

        if self.crashed_plugins.read().unwrap().contains(&plugin_idx) {
            return Err(PluginError::Failed(format!("Plugin '{plugin_name}' is crashed")));
        }

        let plugin = &self.plugins[plugin_idx];
        plugin.execute_async(conv_id, args).await
    }

    /// Resolve "`plugin_name.convention_name`" to a global routing ID.
    pub fn resolve_by_name(&self, name: &str) -> Option<u32> {
        self.name_routing.get(name).copied()
    }

    pub fn conventions(&self) -> Vec<Convention> {
        self.plugins.iter().flat_map(|p| p.conventions()).collect()
    }

    pub fn plugin_names(&self) -> Vec<String> {
        self.plugins.iter().map(|p| p.name().to_string()).collect()
    }

    /// Build the catalog-to-routing bridge. Called after all plugins are registered.
    ///
    /// The Mind outputs catalog IDs (from training), but the runtime uses plugin-index-based
    /// global IDs. This mapping lets `resolve_catalog_id` translate between the two.
    #[allow(clippy::cast_possible_truncation)] // catalog IDs are small
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

    /// Resolve a catalog ID to a global routing ID, falling back to the raw ID if unmapped.
    pub fn resolve_catalog_id(&self, catalog_id: u32) -> u32 {
        self.catalog_routing.get(&catalog_id).copied().unwrap_or(catalog_id)
    }

    /// Collect serialized state from all plugins for checkpointing (Section 7.5).
    pub fn collect_plugin_states(&self) -> Vec<(String, serde_json::Value)> {
        self.plugins.iter()
            .filter_map(|p| {
                p.checkpoint_state().map(|state| (p.name().to_string(), state))
            })
            .collect()
    }

    /// Restore plugin states from checkpoint data, matching by plugin name.
    pub fn restore_plugin_states(&mut self, states: &[(String, serde_json::Value)]) {
        for (name, state) in states {
            for plugin in &mut self.plugins {
                if plugin.name() == name
                    && let Err(e) = plugin.restore_state(state) {
                        tracing::warn!(plugin = name, error = %e, "Failed to restore plugin state");
                    }
            }
        }
    }

    /// Returns (name, version) pairs for all loaded plugins.
    pub fn plugin_manifest(&self) -> Vec<(String, String)> {
        self.plugins.iter().map(|p| (p.name().to_string(), p.version().to_string())).collect()
    }

    /// Snapshot all per-convention execution stats (Section 19.3).
    #[allow(dead_code)] // Spec Section 19.3 -- exposed via metrics/proprioception
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

    /// Returns all conventions paired with their owning plugin name.
    pub fn namespaced_conventions(&self) -> Vec<(String, Convention)> {
        self.plugins.iter()
            .flat_map(|p| {
                let pname = p.name().to_string();
                p.conventions().into_iter().map(move |c| (pname.clone(), c))
            })
            .collect()
    }

    /// Returns names of plugins that provide `LoRA` weights for Mind adaptation.
    pub fn plugins_with_lora_weights(&self) -> Vec<String> {
        self.plugins.iter()
            .filter(|p| p.lora_weights().is_some())
            .map(|p| p.name().to_string())
            .collect()
    }

    /// Get raw `LoRA` weight bytes for a specific plugin.
    pub fn get_plugin_lora_weights(&self, name: &str) -> Option<Vec<u8>> {
        self.plugins.iter()
            .find(|p| p.name() == name)
            .and_then(|p| p.lora_weights())
    }

    /// Unload all plugins in reverse registration order during shutdown.
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

    /// Unregister a plugin by name: calls `on_unload`, removes routing entries, and drops it.
    pub fn unregister(&mut self, name: &str) -> Result<(), String> {
        let idx = self.plugins.iter().position(|p| p.name() == name)
            .ok_or_else(|| format!("Plugin not found: {name}"))?;

        // Call on_unload
        if let Err(e) = self.plugins[idx].on_unload() {
            tracing::warn!(plugin = %name, error = %e, "Plugin unload error during unregister");
        }

        #[allow(clippy::cast_possible_truncation)] // plugin count is small
        let id_offset = (idx as u32) * 1000;

        let conventions = self.plugins[idx].conventions();
        for conv in &conventions {
            let global_id = id_offset + conv.id;
            self.routing.remove(&global_id);
            let full_name = format!("{}.{}", name, conv.name);
            self.name_routing.remove(&full_name);
        }

        self.plugins.remove(idx);

        tracing::info!(plugin = %name, "Plugin unregistered");
        Ok(())
    }

    /// Detect unhealthy plugins by aggregating error rates across their conventions.
    /// Returns warnings for plugins with >50% error rate (Section 11.3).
    #[allow(clippy::significant_drop_tightening)] // stats_map lock needed for entire iteration
    pub fn check_plugin_health(&self) -> Vec<(String, &str)> {
        let mut warnings = Vec::new();
        let Ok(stats_map) = self.convention_stats.read() else {
            return warnings;
        };

        for (idx, plugin) in self.plugins.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)] // plugin count is small
            let id_offset = (idx as u32) * 1000;
            let mut total_calls: u64 = 0;
            let mut total_errors: u64 = 0;

            // Sum stats for IDs in this plugin's 1000-wide range
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

/// Validate a plugin's config against its declared schema at registration time.
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

/// Minimal tilde expansion for paths in program arguments.
mod shellexpand {
    /// Expand leading `~/` to `$HOME/`.
    pub fn tilde(path: &str) -> String {
        if let Some(rest) = path.strip_prefix("~/")
            && let Ok(home) = std::env::var("HOME") {
                return format!("{home}/{rest}");
            }
        if path == "~"
            && let Ok(home) = std::env::var("HOME") {
                return home;
            }
        path.to_string()
    }
}
