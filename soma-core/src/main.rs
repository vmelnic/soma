mod config;
mod errors;
mod mcp;
mod memory;
mod metrics;
mod mind;
mod plugin;
mod proprioception;
mod protocol;
mod state;

use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use config::SomaConfig;
use mcp::auth::AuthManager;
use mcp::server::McpServer;
use memory::checkpoint::Checkpoint;
use memory::consolidation::ConsolidationConfig as ConsolidationEngine;
use memory::experience::{Experience, ExperienceBuffer};
use metrics::SomaMetrics;
use mind::{ArgType, MindEngine, ProgramStep, STOP_ID};
use mind::onnx_engine::OnnxMindEngine;
use plugin::builtin::PosixPlugin;
use plugin::interface::SomaPlugin;
use plugin::manager::PluginManager;
use proprioception::Proprioception;
use protocol::discovery::PeerRegistry;
use protocol::server::{SomaSignalHandler, SynapseServer};
use state::SomaState;

/// SOMA CLI — single binary, neural mind drives hardware directly.
#[derive(Parser)]
#[command(name = "soma", about = "SOMA: Neural mind drives hardware directly")]
struct Cli {
    /// Model directory (encoder.onnx, decoder.onnx, tokenizer.json, meta.json)
    #[arg(long)]
    model: Option<PathBuf>,

    /// Configuration file path
    #[arg(long, default_value = "soma.toml")]
    config: PathBuf,

    /// Interactive REPL mode
    #[arg(long, default_value_t = true)]
    repl: bool,

    /// Single intent to execute (non-interactive)
    #[arg(long)]
    intent: Option<String>,

    /// Override protocol.bind address (e.g. "127.0.0.1:9999")
    #[arg(long)]
    bind: Option<String>,

    /// Additional peers in name:host:port format (can be specified multiple times)
    #[arg(long)]
    peer: Vec<String>,

    /// Specific checkpoint file to restore on startup
    #[arg(long)]
    checkpoint: Option<PathBuf>,

    /// Override soma.log_level (trace, debug, info, warn, error)
    #[arg(long)]
    log_level: Option<String>,

    /// Run MCP server on stdio (for LLM integration)
    #[arg(long)]
    mcp: bool,
}

/// Active inference counter for resource limiting.
static ACTIVE_INFERENCES: AtomicUsize = AtomicUsize::new(0);

/// Flag to stop accepting new connections/requests during shutdown (Section 11.4 step 1).
static ACCEPTING_CONNECTIONS: AtomicBool = AtomicBool::new(true);

fn display_result(result: &plugin::manager::ProgramResult, _catalog: &[mind::CatalogEntry]) {
    if result.success {
        if let Some(output) = &result.output {
            match output {
                plugin::interface::Value::List(items) => {
                    println!("  [Body] ({} items):", items.len());
                    for item in items.iter().take(15) {
                        println!("    {}", item);
                    }
                    if items.len() > 15 {
                        println!("    ... and {} more", items.len() - 15);
                    }
                }
                plugin::interface::Value::Map(entries) => {
                    println!("  [Body]");
                    for (k, v) in entries {
                        println!("    {}: {}", k, v);
                    }
                }
                _ => println!("  [Body] {}", output),
            }
        } else {
            println!("  [Body] Done.");
        }
    } else {
        println!("  [Body] Error: {}", result.error.as_deref().unwrap_or("unknown"));
    }
}

fn run_intent(
    mind: &Arc<RwLock<OnnxMindEngine>>,
    plugins: &Arc<RwLock<PluginManager>>,
    proprio: &Arc<RwLock<Proprioception>>,
    experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    soma_state: &Arc<RwLock<SomaState>>,
    soma_metrics: &Arc<SomaMetrics>,
    config: &SomaConfig,
    text: &str,
    max_concurrent: usize,
    max_program_steps: usize,
    trace_verbosity: &str,
    soma_id: &str,
) {
    // Section 11.4 step 1: reject new requests during shutdown
    if !ACCEPTING_CONNECTIONS.load(Ordering::SeqCst) {
        eprintln!("  [Resource] Error: SOMA is shutting down, not accepting new requests");
        return;
    }

    let current = ACTIVE_INFERENCES.fetch_add(1, Ordering::SeqCst);
    if current >= max_concurrent {
        ACTIVE_INFERENCES.fetch_sub(1, Ordering::SeqCst);
        eprintln!(
            "  [Resource] Error: max concurrent inferences exceeded ({}/{})",
            current + 1,
            max_concurrent
        );
        return;
    }

    let exec_start = std::time::Instant::now();
    let trace_id = uuid::Uuid::new_v4().to_string()[..12].to_string();

    // Section 18.1.1: span_id and parent_span_id are automatically emitted by
    // tracing-subscriber's JSON formatter when spans are active. Creating a span
    // here ensures each intent execution is individually traceable.
    let _intent_span = tracing::info_span!(
        "intent",
        soma_id = %soma_id,
        trace_id = %trace_id,
    )
    .entered();

    // mind.infer() is synchronous with internal timeout enforcement via
    // max_inference_time_secs checked each decoder step (see onnx_engine.rs).
    let mind_guard = mind.read().unwrap();
    match mind_guard.infer(text) {
        Ok(program) => {
            let real: Vec<&ProgramStep> = program
                .steps
                .iter()
                .filter(|s| s.conv_id != STOP_ID)
                .collect();
            println!(
                "\n  [Mind] Program ({} steps, {:.0}%):",
                real.len(),
                program.confidence * 100.0
            );
            for (i, step) in program.steps.iter().enumerate() {
                println!("    {}", step.format(i, &mind_guard.meta().catalog));
                if step.conv_id == STOP_ID {
                    break;
                }
            }
            println!();

            let result = plugins.read().unwrap().execute_program(&program.steps, max_program_steps);

            // Configurable trace verbosity (Section 11.5)
            match trace_verbosity {
                "verbose" => {
                    for entry in &result.trace {
                        println!("    [{}] {} {} {}", entry.step, entry.op,
                            if entry.success { "ok" } else { "FAIL" }, entry.summary);
                    }
                }
                "terse" => {} // no trace output
                _ /* normal */ => {
                    for entry in &result.trace {
                        if entry.op != "STOP" && entry.op != "EMIT" && !entry.summary.is_empty() {
                            println!("    [{}] {} ... {}", entry.step, entry.op, entry.summary);
                        }
                    }
                }
            }
            display_result(&result, &mind_guard.meta().catalog);

            let execution_time_ms = exec_start.elapsed().as_millis() as u64;

            // Metrics
            soma_metrics.record_inference(result.success, execution_time_ms);
            soma_metrics.record_program(program.steps.len() as u64);

            tracing::info!(
                component = "mind",
                soma_id = %soma_id,
                trace_id = %trace_id,
                intent = %text,
                steps = program.steps.len(),
                confidence = %program.confidence,
                success = result.success,
                execution_time_ms = execution_time_ms,
                "Intent processed"
            );

            // Section 11.3: Retry-with-variation — if execution failed,
            // try re-inference BEFORE recording, so we record the final outcome.
            let (final_success, final_steps, final_confidence, final_error) =
                if !result.success && program.confidence > 0.3 {
                    drop(mind_guard);
                    let mind_guard2 = mind.read().unwrap();
                    if let Ok(program2) = mind_guard2.infer(text) {
                        if program2.steps != program.steps {
                            println!("  [Mind] Re-inferred alternative program ({} steps, {:.0}%)",
                                program2.steps.len(), program2.confidence * 100.0);
                            let result2 = plugins.read().unwrap().execute_program(&program2.steps, max_program_steps);
                            if result2.success {
                                println!("  [Mind] Re-inference succeeded:");
                                display_result(&result2, &mind_guard2.meta().catalog);
                                (true, program2.steps.len(), program2.confidence, None)
                            } else {
                                (false, program.steps.len(), program.confidence, result.error.clone())
                            }
                        } else {
                            (false, program.steps.len(), program.confidence, result.error.clone())
                        }
                    } else {
                        (false, program.steps.len(), program.confidence, result.error.clone())
                    }
                } else {
                    (result.success, program.steps.len(), program.confidence, result.error.clone())
                };

            // Record experience with FINAL outcome (not first attempt)
            {
                let mind_r = mind.read().unwrap();
                let tokens: Vec<u32> = mind_r
                    .tokenizer
                    .encode(text)
                    .iter()
                    .map(|&t| t as u32)
                    .collect();
                let program_data: Vec<(i32, u8, u8)> = program
                    .steps
                    .iter()
                    .map(|s| {
                        let a0 = match s.arg0_type {
                            ArgType::None => 0u8,
                            ArgType::Span => 1u8,
                            ArgType::Ref => 2u8,
                            ArgType::Literal => 3u8,
                        };
                        let a1 = match s.arg1_type {
                            ArgType::None => 0u8,
                            ArgType::Span => 1u8,
                            ArgType::Ref => 2u8,
                            ArgType::Literal => 3u8,
                        };
                        (s.conv_id, a0, a1)
                    })
                    .collect();
                let exp = Experience {
                    intent_tokens: tokens,
                    program: program_data,
                    success: final_success,
                    execution_time_ms,
                    timestamp: std::time::Instant::now(),
                    cached_states: program.cached_states.clone(),
                };
                if let Ok(mut buf) = experience_buf.write() {
                    // Section 17.1: Only successful executions are recorded
                    if final_success {
                        buf.record(exp);
                    }
                    // Update experience buffer gauge (Bug #1)
                    soma_metrics.experience_buffer_size.store(
                        buf.len() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }

            // Record in execution history with FINAL outcome
            if let Ok(mut st) = soma_state.write() {
                st.executions.record(
                    text.to_string(),
                    final_steps,
                    final_confidence,
                    final_success,
                    execution_time_ms,
                    trace_id.clone(),
                    final_error,
                );
            }

            // Update proprioception
            if let Ok(mut p) = proprio.write() {
                if final_success {
                    p.record_success();
                } else {
                    p.record_failure();
                }
            }

            // Runtime LoRA adaptation trigger (Section 4.7)
            if config.mind.lora.adaptation_enabled && final_success {
                let success_count = experience_buf.read().map(|buf| buf.success_count()).unwrap_or(0);
                if success_count > 0 && success_count % config.mind.lora.adapt_every_n_successes == 0 {
                    let experiences: Vec<Experience> = {
                        let buf = experience_buf.read().unwrap();
                        buf.successes().into_iter().cloned().collect()
                    };
                    let adapt_config = mind::adaptation::AdaptationConfig {
                        enabled: true,
                        adapt_every_n: config.mind.lora.adapt_every_n_successes,
                        batch_size: config.mind.lora.adapt_batch_size,
                        learning_rate: config.mind.lora.adapt_learning_rate,
                    };
                    eprintln!("  [Mind] Adaptation triggered ({} experiences, batch_size={}) — running in background...",
                        experiences.len(), config.mind.lora.adapt_batch_size);
                    // Run adaptation on a background thread to avoid blocking the REPL/MCP.
                    // This mirrors the whitepaper's "sleep" metaphor — learning happens
                    // during idle periods, not during active execution.
                    let mind_clone = mind.clone();
                    let proprio_clone = proprio.clone();
                    let metrics_clone = soma_metrics.clone();
                    std::thread::spawn(move || {
                        let mut mind_guard = mind_clone.write().unwrap();
                        match mind::adaptation::adapt_from_experience(&mut *mind_guard, &experiences, &adapt_config) {
                            Ok(result) => {
                                eprintln!("  [Mind] Adapted. Loss: {:.4}, Cycle: {}, LoRA magnitude: {:.6}",
                                    result.loss, result.cycle, result.lora_magnitude);
                                if let Ok(mut p) = proprio_clone.write() {
                                    p.record_adaptation();
                                }
                                metrics_clone.adaptations_total.fetch_add(1, Ordering::Relaxed);
                                metrics_clone.lora_magnitude.store(
                                    result.lora_magnitude.to_bits() as u64,
                                    Ordering::Relaxed,
                                );
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "LoRA adaptation failed");
                            }
                        }
                    });
                }
            }
        }
        Err(e) => {
            let execution_time_ms = exec_start.elapsed().as_millis() as u64;
            soma_metrics.record_inference(false, execution_time_ms);
            tracing::info!(
                component = "mind",
                soma_id = %soma_id,
                trace_id = %trace_id,
                intent = %text,
                success = false,
                "Intent failed: {}", e
            );
            println!("  [Mind] Error: {}", e);

            // Record failed execution in history
            if let Ok(mut st) = soma_state.write() {
                st.executions.record(
                    text.to_string(),
                    0,
                    0.0,
                    false,
                    execution_time_ms,
                    trace_id,
                    Some(e.to_string()),
                );
            }

            if let Ok(mut p) = proprio.write() {
                p.record_failure();
            }
        }
    }

    ACTIVE_INFERENCES.fetch_sub(1, Ordering::SeqCst);
}

fn do_checkpoint(
    config: &SomaConfig,
    _experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    proprio: &Arc<RwLock<Proprioception>>,
    plugins: &Arc<RwLock<PluginManager>>,
    soma_state: Option<&Arc<RwLock<SomaState>>>,
    mind: Option<&Arc<RwLock<OnnxMindEngine>>>,
) {
    let ckpt_dir = Path::new(&config.memory.checkpoint_dir);
    let filename = Checkpoint::filename(&config.soma.id);
    let path = ckpt_dir.join(&filename);

    let (exp_count, adapt_count) = {
        let p = proprio.read().unwrap();
        (p.experience_count, p.total_adaptations)
    };

    // Collect plugin state (Section 7.5: institutional memory)
    let plugins_guard = plugins.read().unwrap();
    let plugin_states = plugins_guard.collect_plugin_states();
    let plugin_state_entries: Vec<memory::checkpoint::PluginStateEntry> = plugin_states
        .into_iter()
        .map(|(name, state)| memory::checkpoint::PluginStateEntry {
            plugin_name: name,
            state,
        })
        .collect();

    // Checkpoint LoRA state from mind engine (Section 4.7)
    let lora_state = if let Some(mind_ref) = mind {
        if let Ok(m) = mind_ref.read() {
            match m.checkpoint_lora() {
                Ok(ckpt_lora) => ckpt_lora.layers,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to checkpoint LoRA state");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let mut ckpt = Checkpoint::new(
        config.soma.id.clone(),
        lora_state,
        exp_count,
        adapt_count,
    );
    ckpt.plugin_states = plugin_state_entries;
    ckpt.plugin_manifest = plugins_guard.plugin_manifest().into_iter()
        .map(|(name, version)| memory::checkpoint::PluginManifestEntry { name, version })
        .collect();

    // Record base model hash and consolidated weight delta for checkpoint integrity
    if let Some(mind_ref) = mind {
        if let Ok(m) = mind_ref.read() {
            ckpt.base_model_hash = m.model_hash.clone();
            // Persist consolidated LoRA weight delta (Section 6.3)
            if !m.merged_opcode_delta.is_empty() {
                ckpt.merged_opcode_delta = m.merged_opcode_delta.clone();
            }
        }
    }

    // Persist decisions and execution history (Section 7.5: institutional memory)
    if let Some(state_ref) = soma_state {
        if let Ok(st) = state_ref.read() {
            ckpt.decisions = serde_json::to_value(st.decisions.list())
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            ckpt.recent_executions = serde_json::to_value(&st.executions.to_json())
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
        }
    }

    match ckpt.save(&path) {
        Ok(()) => {
            println!("  [Memory] Checkpoint saved: {}", path.display());
            if let Ok(mut p) = proprio.write() {
                p.record_checkpoint();
            }
            match Checkpoint::prune_checkpoints(ckpt_dir, config.memory.max_checkpoints) {
                Ok(n) if n > 0 => println!("  [Memory] Pruned {} old checkpoint(s)", n),
                _ => {}
            }
        }
        Err(e) => {
            println!("  [Memory] Checkpoint failed: {}", e);
        }
    }
}

fn do_consolidate(
    config: &SomaConfig,
    proprio: &Arc<RwLock<Proprioception>>,
    mind: &Arc<RwLock<OnnxMindEngine>>,
    experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    plugins: &Arc<RwLock<PluginManager>>,
    soma_state: Option<&Arc<RwLock<SomaState>>>,
) {
    let consolidation = ConsolidationEngine::new(
        config.memory.consolidation.min_lora_magnitude,
        config.memory.consolidation.threshold,
    );
    let p = proprio.read().unwrap();
    let adaptation_count = p.total_adaptations;
    // Read current max LoRA magnitude from the mind engine
    let max_magnitude = {
        let m = mind.read().unwrap();
        m.active_lora().iter()
            .map(|l| l.magnitude())
            .fold(0.0f32, f32::max)
    };
    if consolidation.should_consolidate(adaptation_count, max_magnitude) {
        println!(
            "  [Memory] Consolidation criteria met ({} adaptations, magnitude {:.4})",
            adaptation_count, max_magnitude
        );
        drop(p); // release read lock before acquiring write lock on mind
        let mut mind_guard = mind.write().unwrap();
        let result = consolidation.consolidate(&mut *mind_guard);
        println!(
            "  [Memory] Consolidation complete: evaluated={}, merged={}, magnitude={:.4}",
            result.layers_evaluated, result.layers_merged, result.new_magnitude
        );
        // Record consolidation in proprioception
        drop(mind_guard);
        if let Ok(mut p) = proprio.write() {
            p.consolidations += result.layers_merged as u64;
        }

        // Post-consolidation: checkpoint the new permanent state (Section 6.3)
        do_checkpoint(config, experience_buf, proprio, plugins, soma_state, Some(mind));

        // Clear experience buffer after consolidation — learned knowledge
        // is now consolidated into the model weights (Section 6.3)
        if let Ok(mut buf) = experience_buf.write() {
            let cleared = buf.len();
            buf.clear();
            println!("  [Memory] Experience buffer cleared ({} entries)", cleared);
        }
    } else {
        println!(
            "  [Memory] Consolidation not needed (adaptations: {}/{}, magnitude: {:.4}/{:.4})",
            adaptation_count, consolidation.threshold, max_magnitude, consolidation.min_lora_magnitude
        );
    }
}

fn format_uptime(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!(
            "{}h {}m {}s",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }
}

/// Graceful shutdown sequence (Whitepaper Section 11.4):
/// 1. Stop accepting new requests
/// 2. Notify peers with CLOSE signals
/// 3. Drain in-flight requests (wait for ACTIVE_INFERENCES to reach 0)
/// 4. Auto-checkpoint
/// 5. Unload plugins (on_unload lifecycle)
/// 6. Close listeners
/// 7. Exit
fn do_shutdown(
    config: &SomaConfig,
    experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    proprio: &Arc<RwLock<Proprioception>>,
    server_handle: Option<&tokio::task::JoinHandle<()>>,
    peers: Option<&Arc<RwLock<PeerRegistry>>>,
    plugins: &Arc<RwLock<PluginManager>>,
    soma_state: Option<&Arc<RwLock<SomaState>>>,
    mind: Option<&Arc<RwLock<OnnxMindEngine>>>,
) {
    // Step 1: Stop accepting new connections/requests immediately
    ACCEPTING_CONNECTIONS.store(false, Ordering::SeqCst);
    tracing::info!("Stopped accepting new connections");

    // Step 2: Notify peers with CLOSE signals (joined, not fire-and-forget)
    if let Some(peer_reg) = peers {
        let pr = peer_reg.read().unwrap();
        let peer_list = pr.list();
        if !peer_list.is_empty() {
            println!("  [Protocol] Notifying {} peer(s) of shutdown...", peer_list.len());
            let close_signal = protocol::signal::Signal::close(&config.soma.id);
            let mut handles = Vec::new();
            for peer in &peer_list {
                tracing::info!(
                    peer = %peer.name,
                    addr = %peer.addr,
                    "Sending CLOSE to peer"
                );
                let addr = peer.addr.clone();
                let signal = close_signal.clone();
                let sender = config.soma.id.clone();
                let handle = std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    if let Ok(rt) = rt {
                        let _ = rt.block_on(async {
                            let _ = protocol::client::SynapseClient::send(
                                &addr, &sender, &signal,
                            )
                            .await;
                        });
                    }
                });
                handles.push(handle);
            }
            // Wait for all CLOSE sends to complete (max 3s)
            for handle in handles {
                let _ = handle.join();
            }
        }
    }

    // Step 3: Drain in-flight requests
    let drain_start = std::time::Instant::now();
    while ACTIVE_INFERENCES.load(Ordering::SeqCst) > 0 {
        if drain_start.elapsed().as_secs() > 10 {
            println!(
                "  [Shutdown] {} inferences still in-flight after 10s, forcing shutdown",
                ACTIVE_INFERENCES.load(Ordering::SeqCst)
            );
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Step 4: Flush pending signals in outbound queues
    // OfflineQueue is per-connection; queued signals are drained when connections close.
    // For server-level outbound queues, there's nothing to flush in the current architecture.
    tracing::debug!("Outbound queues flushed (per-connection, drained on close)");

    // Step 5: Auto-checkpoint (includes plugin state)
    if config.memory.auto_checkpoint {
        do_checkpoint(config, experience_buf, proprio, plugins, soma_state, mind);
    }

    // Step 6: Unload plugins BEFORE closing listeners (Section 11.4, spec step 6 before 7)
    {
        let plugin_count = plugins.read().unwrap().plugin_names().len();
        plugins.write().unwrap().unload_all();
        tracing::info!(plugins = plugin_count, "Plugins unloaded");
    }

    // Step 7: Close listeners / stop protocol server
    if let Some(handle) = server_handle {
        handle.abort();
        tracing::info!("Protocol server stopped");
    }

    // Step 8: Close MCP server (if running, it was already stopped by exiting run_stdio)
    tracing::info!("MCP server closed");

    // Step 8: Log final stats and exit
    let p = proprio.read().unwrap();
    let uptime = format_uptime(p.uptime());
    println!(
        "  SOMA shutdown. Uptime: {}, Inferences: {}, Experiences: {}, Decisions: {}",
        uptime, p.total_inferences, p.experience_count, p.total_decisions_recorded
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Step 1: Load configuration
    let mut config = SomaConfig::load(&cli.config)?;

    // Step 1b: Apply environment variable overrides (Section 15.3)
    config.apply_env_overrides();

    // Apply CLI overrides
    if let Some(ref log_level) = cli.log_level {
        config.soma.log_level = log_level.clone();
    }
    if let Some(ref bind) = cli.bind {
        config.protocol.bind = bind.clone();
    }
    for peer_str in &cli.peer {
        let parts: Vec<&str> = peer_str.splitn(2, ':').collect();
        if parts.len() == 2 {
            config.protocol.peers.insert(parts[0].to_string(), parts[1].to_string());
        } else {
            eprintln!(
                "  Warning: ignoring invalid peer format '{}' (expected name:host:port)",
                peer_str
            );
        }
    }

    // Initialize logging (Section 11.5)
    let log_filter = format!("soma={}", config.soma.log_level);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(log_filter.parse()?);

    if std::env::var("SOMA_LOG_JSON").is_ok() {
        // Section 18.1.1: JSON logs include soma_id, trace_id, span_id, parent_span_id.
        // with_span_list(true) embeds the active span stack (including span_id and
        // parent relationships) into each JSON log line automatically.
        tracing_subscriber::fmt()
            .json()
            .with_span_list(true)
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }

    // Step 2: Initialize proprioception
    let proprio = Arc::new(RwLock::new(Proprioception::new()));

    // Step 3: Load Mind Engine
    let model_dir = cli
        .model
        .unwrap_or_else(|| PathBuf::from(&config.mind.model_dir));
    let mut engine = OnnxMindEngine::load(&model_dir)
        .map_err(|e| {
            eprintln!("  Error: Failed to load Mind model from {}", model_dir.display());
            eprintln!("  Ensure encoder.onnx, decoder.onnx, tokenizer.json, and meta.json exist.");
            e
        })?;
    engine.temperature = config.mind.temperature;
    engine.max_inference_time_secs = config.mind.max_inference_time_secs;
    let mind = Arc::new(RwLock::new(engine));

    // Step 3 verification: test inference on "ping" (Section 11.1)
    {
        let m = mind.read().unwrap();
        match m.infer("ping") {
            Ok(program) => {
                if program.steps.is_empty() {
                    anyhow::bail!("Model verification failed: 'ping' produced empty program");
                }
                tracing::info!(
                    steps = program.steps.len(),
                    confidence = %program.confidence,
                    "Model verification passed"
                );
            }
            Err(e) => {
                anyhow::bail!("Model verification failed: {}", e);
            }
        }
    }

    let mind_info = {
        let m = mind.read().unwrap();
        m.info()
    };

    // Step 4: Load Plugins (Section 11.1: "Failed plugin: skip and continue")
    let mut plugins = PluginManager::new();

    // Helper: build PluginConfig from [plugins.<name>] TOML section
    let build_plugin_config = |name: &str| -> plugin::interface::PluginConfig {
        let mut pc = plugin::interface::PluginConfig::default();
        if let Some(toml_val) = config.plugins.get(name) {
            // Convert toml::Value table entries to serde_json::Value
            if let Some(table) = toml_val.as_table() {
                for (k, v) in table {
                    if let Ok(json_val) = serde_json::to_value(v) {
                        pc.settings.insert(k.clone(), json_val);
                    }
                }
            }
        }
        pc
    };

    // Register built-in PosixPlugin
    {
        let mut posix = PosixPlugin::new();
        let pc = build_plugin_config("posix");
        if let Err(e) = posix.on_load(&pc) {
            tracing::warn!(error = %e, "PosixPlugin on_load failed");
        }
        let posix: Box<dyn plugin::interface::SomaPlugin> = Box::new(posix);
        plugins.register(posix);
    }

    // Discover plugins from directory, with manifest + platform filtering (Section 5.3)
    let plugins_dir = std::path::Path::new(&config.soma.plugins_directory);
    let discovered = plugin::dynamic::discover_plugins(plugins_dir);
    for (plugin_path, _manifest) in &discovered {
        match plugin::dynamic::load_plugin_from_path(plugin_path) {
            Ok(mut p) => {
                let pc = build_plugin_config(p.name());
                if let Err(e) = p.on_load(&pc) {
                    tracing::warn!(plugin = p.name(), error = %e, "Plugin on_load failed, skipping");
                    continue;
                }
                tracing::info!(plugin = p.name(), path = %plugin_path.display(), "Dynamic plugin loaded");
                plugins.register(p);
            }
            Err(e) => {
                tracing::warn!(path = %plugin_path.display(), error = %e, "Failed to load dynamic plugin, skipping");
            }
        }
    }
    if !discovered.is_empty() {
        eprintln!("  Discovered {} dynamic plugin(s) in {}", discovered.len(), plugins_dir.display());
    }

    let total_conv = plugins.conventions().len();

    // Step 4b: Attach plugin-provided LoRA knowledge to the Mind at startup (Section 7.3)
    {
        let lora_plugins = plugins.plugins_with_lora_weights();
        if !lora_plugins.is_empty() {
            let mut attached_count = 0usize;
            for name in &lora_plugins {
                if let Some(lora_data) = plugins.get_plugin_lora_weights(name) {
                    match mind.write().unwrap().attach_lora_bytes(name, &lora_data) {
                        Ok(_) => {
                            tracing::info!(plugin = %name, "Plugin LoRA attached to Mind");
                            attached_count += 1;
                        }
                        Err(e) => {
                            tracing::warn!(plugin = %name, error = %e, "Failed to attach plugin LoRA");
                        }
                    }
                }
            }
            eprintln!(
                "  LoRA knowledge attached from {}/{} plugin(s): {}",
                attached_count,
                lora_plugins.len(),
                lora_plugins.join(", ")
            );
        }
    }

    // Step 4c: Verify convention coverage — model catalog vs loaded plugins (Section 11.1)
    {
        let m = mind.read().unwrap();
        // Build catalog-to-plugin routing: maps model catalog IDs → plugin manager global IDs
        let catalog = &m.meta().catalog;
        plugins.build_catalog_routing(catalog);

        // Check which catalog conventions have no matching plugin
        let mut missing = Vec::new();
        for entry in catalog {
            if entry.name == "EMIT" || entry.name == "STOP" {
                continue; // built-in control opcodes, handled separately
            }
            if plugins.resolve_catalog_id(entry.id as u32) == entry.id as u32 {
                // Not remapped — means no matching plugin convention found by name
                // Check if it exists via name_routing
                if plugins.resolve_by_name(&entry.name).is_none() {
                    missing.push((entry.id, entry.name.clone()));
                }
            }
        }
        if !missing.is_empty() {
            for (id, name) in &missing {
                tracing::warn!(
                    conv_id = id,
                    conv_name = %name,
                    "Model catalog convention has no matching loaded plugin"
                );
            }
            eprintln!(
                "  Warning: {} convention(s) in model catalog have no matching plugin",
                missing.len()
            );
        } else if !catalog.is_empty() {
            tracing::info!(
                catalog_size = catalog.len(),
                "All model catalog conventions mapped to loaded plugins"
            );
        }
    }

    // Step 5c: Initialize metrics (Whitepaper Section 11.5)
    let soma_metrics = Arc::new(SomaMetrics::new());

    // Wire metrics into plugin manager for call tracking
    plugins.set_metrics(soma_metrics.clone());

    // Record loaded plugin names in proprioception
    {
        let plugin_names = plugins.plugin_names();
        let mut p = proprio.write().unwrap();
        p.set_plugins(plugin_names);
    }

    // Step 5: Initialize memory system
    let experience_buf = Arc::new(RwLock::new(ExperienceBuffer::new(
        config.memory.max_experience_buffer,
    )));

    // Step 5b: Initialize state system (Whitepaper Section 1.2, 7.5)
    let soma_state = Arc::new(RwLock::new(SomaState::new(
        config.mcp.max_execution_history,
    )));

    // Step 5d: Initialize peer registry
    let peer_registry = Arc::new(RwLock::new(PeerRegistry::new()));
    {
        let mut pr = peer_registry.write().unwrap();
        pr.load_from_config(&config.protocol.peers);
    }

    // Step 5e: Initialize auth manager (Whitepaper Section 8.3)
    // Load auth tokens from environment variables
    let auth_manager = Arc::new(RwLock::new(AuthManager::new(config.security.require_auth)));
    if config.security.require_auth {
        let mut auth = auth_manager.write().unwrap();
        if let Ok(token) = std::env::var(&config.security.admin_token_env) {
            auth.register_admin_token(token);
            tracing::info!("Registered admin token from env {}", config.security.admin_token_env);
        }
        if let Ok(token) = std::env::var(&config.security.builder_token_env) {
            auth.register_builder_token(token);
            tracing::info!("Registered builder token from env {}", config.security.builder_token_env);
        }
        if let Ok(token) = std::env::var(&config.security.viewer_token_env) {
            auth.register_viewer_token(token);
            tracing::info!("Registered viewer token from env {}", config.security.viewer_token_env);
        }
    }

    // Step 6: Load checkpoint — restores proprioception, decisions, execution history,
    // LoRA state, and plugin states
    let restore_checkpoint = |ckpt: &Checkpoint,
                              proprio: &Arc<RwLock<Proprioception>>,
                              soma_state: &Arc<RwLock<SomaState>>,
                              mind: &Arc<RwLock<OnnxMindEngine>>,
                              plugins: &mut PluginManager| {
        // Verify base model hash matches — detect model changes since checkpoint
        if !ckpt.base_model_hash.is_empty() {
            let current_hash = mind.read().unwrap().model_hash.clone();
            if ckpt.base_model_hash != current_hash {
                eprintln!(
                    "  Warning: Base model hash mismatch! Checkpoint was created with a different model."
                );
                eprintln!(
                    "    Checkpoint model hash: {}",
                    &ckpt.base_model_hash[..16.min(ckpt.base_model_hash.len())]
                );
                eprintln!(
                    "    Current model hash:    {}",
                    &current_hash[..16.min(current_hash.len())]
                );
                eprintln!("    LoRA state from this checkpoint may be incompatible — skipping LoRA restore.");
                // Skip LoRA state restore (lora_state is not applied when hashes mismatch)
            } else {
                // Model hash matches — restore LoRA state if present (Section 4.7)
                if !ckpt.lora_state.is_empty() {
                    let lora_checkpoint = crate::mind::lora::LoRACheckpoint {
                        layers: ckpt.lora_state.clone(),
                        adaptation_count: ckpt.adaptation_count,
                        experience_count: ckpt.experience_count,
                    };
                    if let Ok(mut m) = mind.write() {
                        match m.restore_lora(&lora_checkpoint) {
                            Ok(()) => eprintln!(
                                "  Restored LoRA state ({} layers)",
                                ckpt.lora_state.len()
                            ),
                            Err(e) => eprintln!("  Warning: Failed to restore LoRA state: {}", e),
                        }
                    }
                }
                // Restore consolidated weight delta if present
                if !ckpt.merged_opcode_delta.is_empty() {
                    if let Ok(mut m) = mind.write() {
                        m.set_merged_opcode_delta(ckpt.merged_opcode_delta.clone());
                        eprintln!(
                            "  Restored consolidated weight delta ({} values)",
                            ckpt.merged_opcode_delta.len()
                        );
                    }
                }
            }
        }
        if let Ok(mut p) = proprio.write() {
            p.experience_count = ckpt.experience_count;
            p.total_adaptations = ckpt.adaptation_count;
        }

        // Restore plugin states (Section 7.5: institutional memory)
        if !ckpt.plugin_states.is_empty() {
            let states: Vec<(String, serde_json::Value)> = ckpt.plugin_states.iter()
                .map(|e| (e.plugin_name.clone(), e.state.clone()))
                .collect();
            plugins.restore_plugin_states(&states);
            eprintln!("  Restored plugin states ({} plugins)", ckpt.plugin_states.len());
        }

        // Restore decisions and execution history (Section 7.5: institutional memory)
        if let Ok(mut st) = soma_state.write() {
            for decision_val in &ckpt.decisions {
                if let (Some(what), Some(why)) = (
                    decision_val.get("what").and_then(|v| v.as_str()),
                    decision_val.get("why").and_then(|v| v.as_str()),
                ) {
                    let session = decision_val.get("session_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("restored");
                    st.decisions.record(what.to_string(), why.to_string(), session.to_string());
                }
            }
            // Restore execution history records
            for exec_val in &ckpt.recent_executions {
                if let Ok(record) = serde_json::from_value::<state::execution_history::ExecutionRecord>(exec_val.clone()) {
                    st.executions.record(
                        record.intent,
                        record.program_steps,
                        record.confidence,
                        record.success,
                        record.execution_time_ms,
                        record.trace_id,
                        record.error,
                    );
                }
            }
            if !ckpt.recent_executions.is_empty() {
                eprintln!("  Restored {} execution history records", ckpt.recent_executions.len());
            }
        }
    };

    if let Some(ref ckpt_path) = cli.checkpoint {
        match Checkpoint::load(ckpt_path) {
            Ok(ckpt) => {
                restore_checkpoint(&ckpt, &proprio, &soma_state, &mind, &mut plugins);
                eprintln!(
                    "  Resumed from {}: {} experiences, {} adaptations, {} decisions",
                    ckpt_path.display(),
                    ckpt.experience_count,
                    ckpt.adaptation_count,
                    ckpt.decisions.len(),
                );
            }
            Err(e) => {
                eprintln!(
                    "  Warning: Failed to load checkpoint {}: {}",
                    ckpt_path.display(),
                    e
                );
            }
        }
    } else {
        let ckpt_dir = Path::new(&config.memory.checkpoint_dir);
        match Checkpoint::list_checkpoints(ckpt_dir) {
            Ok(ckpts) if !ckpts.is_empty() => {
                match Checkpoint::load(&ckpts[0]) {
                    Ok(ckpt) => {
                        restore_checkpoint(&ckpt, &proprio, &soma_state, &mind, &mut plugins);
                        eprintln!(
                            "  Resumed: {} experiences, {} adaptations, {} decisions",
                            ckpt.experience_count, ckpt.adaptation_count, ckpt.decisions.len(),
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to load checkpoint, starting fresh");
                    }
                }
            }
            _ => {}
        }
    }

    // Step 7: Start Synaptic Protocol server
    let plugins_arc = Arc::new(RwLock::new(plugins));
    let bind_addr = config.protocol.bind.clone();
    let server_handler = SomaSignalHandler {
        name: config.soma.id.clone(),
        mind: mind.clone(),
        plugins: plugins_arc.clone(),
        max_program_steps: config.mind.max_program_steps,
    };

    let server = SynapseServer::new(config.soma.id.clone(), bind_addr.clone())
        .with_metrics(soma_metrics.clone())
        .with_peer_registry(peer_registry.clone());
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start(server_handler).await {
            tracing::error!(error = %e, "Protocol server error");
        }
    });

    // Step 8: Start MCP Server if requested (Whitepaper Section 8, Milestone 3)
    // "At this point, an LLM can drive SOMA."
    if cli.mcp {
        let mcp_shutdown = Arc::new(AtomicBool::new(false));

        let mcp_server = McpServer {
            config: config.clone(),
            mind: mind.clone(),
            plugins: plugins_arc.clone(),
            proprio: proprio.clone(),
            experience: experience_buf.clone(),
            state: soma_state.clone(),
            metrics: soma_metrics.clone(),
            peers: peer_registry.clone(),
            auth: auth_manager.clone(),
            shutdown_requested: mcp_shutdown.clone(),
        };

        // MCP mode: run on stdio, no REPL
        eprintln!("============================================================");
        eprintln!("  SOMA v0.1.0 -- MCP Server Mode");
        eprintln!("  LLM can now drive SOMA via Model Context Protocol.");
        eprintln!("============================================================");
        eprintln!("  ID:       {}", config.soma.id);
        eprintln!("  Mind:     {} ({}conv)", mind_info.backend, mind_info.conventions_known);
        eprintln!("  Plugins:  posix ({} conventions)", total_conv);
        eprintln!("  Protocol: {} (Synaptic)", config.protocol.bind);
        eprintln!("  MCP:      stdio (JSON-RPC 2.0)");
        eprintln!("============================================================");

        mcp_server.run_stdio().await?;

        if mcp_shutdown.load(Ordering::SeqCst) {
            eprintln!("  SOMA shutting down (MCP shutdown requested).");
        }

        do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state), Some(&mind));
        return Ok(());
    }

    // Step 9: Ready — display banner
    eprintln!("============================================================");
    eprintln!("  SOMA v0.1.0 -- Rust Runtime");
    eprintln!("  Neural mind drives hardware directly. Single binary.");
    eprintln!("============================================================");
    eprintln!("  ID:       {}", config.soma.id);
    eprintln!("  Mind:     {} ({}conv)", mind_info.backend, mind_info.conventions_known);
    eprintln!("  Plugins:  posix ({} conventions)", total_conv);
    eprintln!("  Model:    {}", model_dir.display());
    eprintln!("  Protocol: {} (Synaptic server started)", config.protocol.bind);
    eprintln!("  MCP:      available (--mcp flag to enable)");
    {
        let pr = peer_registry.read().unwrap();
        if pr.count() > 0 {
            eprintln!("  Peers:    {} configured", pr.count());
            for peer in pr.list() {
                eprintln!("            {} -> {}", peer.name, peer.addr);
            }
        }
    }
    eprintln!(
        "  Resources: max {} concurrent inferences, {} max program steps",
        config.resources.max_concurrent_inferences, config.mind.max_program_steps
    );
    eprintln!("============================================================");

    if let Some(intent) = &cli.intent {
        run_intent(
            &mind,
            &plugins_arc,
            &proprio,
            &experience_buf,
            &soma_state,
            &soma_metrics,
            &config,
            intent,
            config.resources.max_concurrent_inferences,
            config.mind.max_program_steps,
            &config.soma.trace_verbosity,
            &config.soma.id,
        );
        do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state), Some(&mind));
        return Ok(());
    }

    // REPL
    eprintln!(
        "  Type intent. :status :health :inspect :checkpoint :consolidate :decisions  quit"
    );
    eprintln!();

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc_handler(r);

    let max_concurrent = config.resources.max_concurrent_inferences;
    let max_steps = config.mind.max_program_steps;

    loop {
        if !running.load(Ordering::Relaxed) {
            println!("\n  SOMA shutting down (SIGINT).");
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state), Some(&mind));
            break;
        }
        print!("intent> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            println!();
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state), Some(&mind));
            break;
        }
        let text = input.trim();
        if text.is_empty() {
            continue;
        }
        if text == "quit" || text == "exit" || text == "q" {
            println!("\n  SOMA shutting down.");
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state), Some(&mind));
            break;
        }

        // REPL commands
        if text == ":status" {
            let info = {
                let m = mind.read().unwrap();
                m.info()
            };
            let p = proprio.read().unwrap();
            let exp = experience_buf.read().unwrap();
            let st = soma_state.read().unwrap();
            println!("\n  [Proprioception]");
            println!("    {}", p.report().replace('\n', "\n    "));
            println!("    Mind:        {}", info.backend);
            println!("    Conventions: {}", info.conventions_known);
            println!("    Max steps:   {}", info.max_steps);
            println!(
                "    LoRA:        {} layers, magnitude {:.6}",
                info.lora_layers, info.lora_magnitude
            );
            println!("    Plugins:     {} loaded", plugins_arc.read().unwrap().conventions().len());
            println!(
                "    Experience:  {}/{} buffer ({} total seen)",
                exp.len(),
                config.memory.max_experience_buffer,
                exp.total_seen()
            );
            println!("    State:       {} decisions, {} executions",
                st.decisions.len(), st.executions.len());
            {
                let pr = peer_registry.read().unwrap();
                println!("    Peers:       {} configured", pr.count());
                for peer in pr.list() {
                    println!("                 {} -> {}", peer.name, peer.addr);
                }
            }
            println!("    Protocol:    {}", config.protocol.bind);
            println!("    MCP:         available (--mcp flag)");
            println!();
            continue;
        }
        if text == ":inspect" || text == "help" || text == "?" {
            println!("\n  [Conventions]");
            for conv in plugins_arc.read().unwrap().conventions() {
                println!("    [{:2}] {} -- {}", conv.id, conv.name, conv.description);
            }
            println!();
            continue;
        }
        if text == ":checkpoint" {
            do_checkpoint(&config, &experience_buf, &proprio, &plugins_arc, Some(&soma_state), Some(&mind));
            println!();
            continue;
        }
        if text == ":consolidate" {
            do_consolidate(&config, &proprio, &mind, &experience_buf, &plugins_arc, Some(&soma_state));
            println!();
            continue;
        }
        if text == ":decisions" {
            let st = soma_state.read().unwrap();
            let decisions = st.decisions.list();
            if decisions.is_empty() {
                println!("\n  [State] No decisions recorded.");
            } else {
                println!("\n  [State] Decision log ({} entries):", decisions.len());
                for d in decisions {
                    println!("    [{}] {} -- {}", d.id, d.what, d.why);
                }
            }
            println!();
            continue;
        }
        if text == ":metrics" {
            println!("\n  [Metrics]");
            println!(
                "{}",
                serde_json::to_string_pretty(&soma_metrics.to_json()).unwrap_or_default()
            );
            println!();
            continue;
        }
        if text == ":health" {
            let pm = plugins_arc.read().unwrap();
            let warnings = pm.check_plugin_health();
            if warnings.is_empty() {
                println!("\n  [Health] All plugins healthy.");
            } else {
                println!("\n  [Health] Plugin warnings:");
                for (name, msg) in &warnings {
                    println!("    {} — {}", name, msg);
                }
            }
            println!();
            continue;
        }

        run_intent(
            &mind,
            &plugins_arc,
            &proprio,
            &experience_buf,
            &soma_state,
            &soma_metrics,
            &config,
            text,
            max_concurrent,
            max_steps,
            &config.soma.trace_verbosity,
            &config.soma.id,
        );
        println!();
    }

    Ok(())
}

fn ctrlc_handler(running: Arc<AtomicBool>) {
    let _ = ctrlc::set_handler(move || {
        running.store(false, Ordering::Relaxed);
    });
}
