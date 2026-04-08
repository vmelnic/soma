//! SOMA runtime entry point.
//!
//! Orchestrates startup (config, Mind engine, plugins, protocol, MCP), runs either
//! a single intent, an interactive REPL, or an MCP server on stdio, then performs
//! graceful shutdown with checkpoint persistence.

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
use mind::{ArgType, MindEngine, STOP_ID};
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

    /// Override `soma.log_level` (trace, debug, info, warn, error)
    #[arg(long)]
    log_level: Option<String>,

    /// Run MCP server on stdio (for LLM integration)
    #[arg(long)]
    mcp: bool,
}

/// Guards against exceeding `max_concurrent_inferences` from config.
static ACTIVE_INFERENCES: AtomicUsize = AtomicUsize::new(0);

/// Set to `false` during shutdown to reject new requests while draining in-flight ones.
static ACCEPTING_CONNECTIONS: AtomicBool = AtomicBool::new(true);

/// Formats and prints a program execution result to stdout.
fn display_result(result: &plugin::manager::ProgramResult, _catalog: &[mind::CatalogEntry]) {
    if result.success {
        if let Some(output) = &result.output {
            match output {
                plugin::interface::Value::List(items) => {
                    println!("  [Body] ({} items):", items.len());
                    for item in items.iter().take(15) {
                        println!("    {item}");
                    }
                    if items.len() > 15 {
                        println!("    ... and {} more", items.len() - 15);
                    }
                }
                plugin::interface::Value::Map(entries) => {
                    println!("  [Body]");
                    for (k, v) in entries {
                        println!("    {k}: {v}");
                    }
                }
                _ => println!("  [Body] {output}"),
            }
        } else {
            println!("  [Body] Done.");
        }
    } else {
        println!("  [Body] Error: {}", result.error.as_deref().unwrap_or("unknown"));
    }
}

/// Runs the full intent lifecycle: inference, execution, retry-on-failure, experience
/// recording, metrics, proprioception updates, and `LoRA` adaptation triggering.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_intent(
    mind: &Arc<RwLock<OnnxMindEngine>>,
    plugins: &Arc<RwLock<PluginManager>>,
    proprio: &Arc<RwLock<Proprioception>>,
    experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    soma_state: &Arc<RwLock<SomaState>>,
    soma_metrics: &Arc<SomaMetrics>,
    reflex_layer: &Arc<RwLock<mind::reflex::ReflexLayer>>,
    config: &SomaConfig,
    text: &str,
    max_concurrent: usize,
    max_program_steps: usize,
    trace_verbosity: &str,
    soma_id: &str,
) {
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

    let _intent_span = tracing::info_span!(
        "intent",
        soma_id = %soma_id,
        trace_id = %trace_id,
    )
    .entered();

    // Reflex check: try to match against cached (intent -> program) pairs before Mind inference.
    let mind_guard = mind.read().unwrap();
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let reflex_tokens: Vec<u32> = mind_guard.tokenizer.encode(text)
        .iter().map(|&t| t as u32).collect();
    drop(mind_guard);

    let reflex_hit = {
        let mut rl = reflex_layer.write().unwrap();
        rl.try_match(&reflex_tokens).map(|(prog, conf)| (prog.clone(), conf))
    };
    let used_reflex = reflex_hit.is_some();

    let mind_guard = mind.read().unwrap();
    let infer_result = if let Some((cached_program, conf)) = reflex_hit {
        soma_metrics.record_reflex_hit(reflex_layer.read().unwrap().len() as u64);
        let real_count = cached_program.steps.iter().filter(|s| s.conv_id != STOP_ID).count();
        println!(
            "\n  [Reflex] Cached program ({real_count} steps, {:.0}%):",
            conf * 100.0
        );
        for (i, step) in cached_program.steps.iter().enumerate() {
            println!("    {}", step.format(i, &mind_guard.meta().catalog));
            if step.conv_id == STOP_ID { break; }
        }
        println!();
        Ok(cached_program)
    } else {
        soma_metrics.record_reflex_miss(reflex_layer.read().unwrap().len() as u64);
        mind_guard.infer(text)
    };

    match infer_result {
        Ok(program) => {
            let real_count = program
                .steps
                .iter()
                .filter(|s| s.conv_id != STOP_ID)
                .count();
            if !used_reflex {
                println!(
                    "\n  [Mind] Program ({real_count} steps, {:.0}%):",
                    program.confidence * 100.0
                );
                for (i, step) in program.steps.iter().enumerate() {
                    println!("    {}", step.format(i, &mind_guard.meta().catalog));
                    if step.conv_id == STOP_ID {
                        break;
                    }
                }
                println!();
            }

            let result = plugins.read().unwrap().execute_program(&program.steps, max_program_steps);

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

            #[allow(clippy::cast_possible_truncation)] // millis will not exceed u64
            let execution_time_ms = exec_start.elapsed().as_millis() as u64;

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

            // Retry-with-variation: re-infer before recording so the experience
            // buffer captures the final outcome, not the failed first attempt.
            let (final_success, final_steps, final_confidence, final_error) =
                if !result.success && program.confidence > 0.3 {
                    drop(mind_guard);
                    let mind_guard2 = mind.read().unwrap();
                    let retry_result = if let Ok(program2) = mind_guard2.infer(text) {
                        if program2.steps == program.steps {
                            None
                        } else {
                            println!("  [Mind] Re-inferred alternative program ({} steps, {:.0}%)",
                                program2.steps.len(), program2.confidence * 100.0);
                            let result2 = plugins.read().unwrap().execute_program(&program2.steps, max_program_steps);
                            if result2.success {
                                println!("  [Mind] Re-inference succeeded:");
                                display_result(&result2, &mind_guard2.meta().catalog);
                                Some((true, program2.steps.len(), program2.confidence, None))
                            } else {
                                None
                            }
                        }
                    } else {
                        None
                    };
                    drop(mind_guard2);
                    retry_result.unwrap_or(
                        (false, program.steps.len(), program.confidence, result.error)
                    )
                } else {
                    (result.success, program.steps.len(), program.confidence, result.error)
                };

            {
                let mind_r = mind.read().unwrap();
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // token IDs are small positive
                let tokens: Vec<u32> = mind_r
                    .tokenizer
                    .encode(text)
                    .iter()
                    .map(|&t| t as u32)
                    .collect();
                drop(mind_r);
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
                // Record successful (intent -> program) pair as a reflex for future lookups.
                // Must happen before experience recording which consumes cached_states.
                if final_success && !used_reflex {
                    if let Ok(mut rl) = reflex_layer.write() {
                        rl.record(reflex_tokens.clone(), program.clone(), program.confidence);
                        soma_metrics.reflex_entries.store(
                            rl.len() as u64,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                }

                let exp = Experience {
                    intent_tokens: tokens,
                    program: program_data,
                    success: final_success,
                    execution_time_ms,
                    timestamp: std::time::Instant::now(),
                    cached_states: program.cached_states,
                };
                if let Ok(mut buf) = experience_buf.write() {
                    // Only successful executions are recorded to avoid reinforcing bad programs.
                    if final_success {
                        buf.record(exp);
                    }
                    soma_metrics.experience_buffer_size.store(
                        buf.len() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }

            if let Ok(mut st) = soma_state.write() {
                st.executions.record(
                    text.to_string(),
                    final_steps,
                    final_confidence,
                    final_success,
                    execution_time_ms,
                    trace_id,
                    final_error,
                );
            }

            if let Ok(mut p) = proprio.write() {
                if final_success {
                    p.record_success();
                } else {
                    p.record_failure();
                }
            }

            if config.mind.lora.adaptation_enabled && final_success {
                let success_count = experience_buf.read().map(|buf| buf.success_count()).unwrap_or(0);
                if success_count > 0 && success_count.is_multiple_of(config.mind.lora.adapt_every_n_successes) {
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
                    // Background thread so adaptation doesn't block the REPL or MCP server.
                    let mind_clone = mind.clone();
                    let proprio_clone = proprio.clone();
                    let metrics_clone = soma_metrics.clone();
                    std::thread::spawn(move || {
                        let mut mind_guard = mind_clone.write().unwrap();
                        match mind::adaptation::adapt_from_experience(&mut mind_guard, &experiences, &adapt_config) {
                            Ok(result) => {
                                eprintln!("  [Mind] Adapted. Loss: {:.4}, Cycle: {}, LoRA magnitude: {:.6}",
                                    result.loss, result.cycle, result.lora_magnitude);
                                if let Ok(mut p) = proprio_clone.write() {
                                    p.record_adaptation();
                                }
                                metrics_clone.adaptations_total.fetch_add(1, Ordering::Relaxed);
                                metrics_clone.lora_magnitude.store(
                                    u64::from(result.lora_magnitude.to_bits()),
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
            #[allow(clippy::cast_possible_truncation)] // millis will not exceed u64
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
            println!("  [Mind] Error: {e}");

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

/// Persists a checkpoint containing `LoRA` state, plugin states, decisions, execution
/// history, and reflex layer cache. Prunes old checkpoints beyond `config.memory.max_checkpoints`.
fn do_checkpoint(
    config: &SomaConfig,
    _experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    proprio: &Arc<RwLock<Proprioception>>,
    plugins: &Arc<RwLock<PluginManager>>,
    soma_state: Option<&Arc<RwLock<SomaState>>>,
    mind: Option<&Arc<RwLock<OnnxMindEngine>>>,
    reflex_layer: Option<&Arc<RwLock<mind::reflex::ReflexLayer>>>,
) {
    let ckpt_dir = Path::new(&config.memory.checkpoint_dir);
    let filename = Checkpoint::filename(&config.soma.id);
    let path = ckpt_dir.join(&filename);

    let (exp_count, adapt_count) = {
        let p = proprio.read().unwrap();
        (p.experience_count, p.total_adaptations)
    };

    let plugins_guard = plugins.read().unwrap();
    let plugin_states = plugins_guard.collect_plugin_states();
    let plugin_state_entries: Vec<memory::checkpoint::PluginStateEntry> = plugin_states
        .into_iter()
        .map(|(name, state)| memory::checkpoint::PluginStateEntry {
            plugin_name: name,
            state,
        })
        .collect();

    #[allow(clippy::option_if_let_else)]
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
    drop(plugins_guard);

    if let Some(mind_ref) = mind
        && let Ok(m) = mind_ref.read()
    {
        ckpt.base_model_hash.clone_from(&m.model_hash);
        if !m.merged_opcode_delta.is_empty() {
            ckpt.merged_opcode_delta.clone_from(&m.merged_opcode_delta);
        }
    }

    if let Some(state_ref) = soma_state
        && let Ok(st) = state_ref.read()
    {
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
            println!("  [Memory] Checkpoint saved: {}", path.display());
            if let Ok(mut p) = proprio.write() {
                p.record_checkpoint();
            }
            match Checkpoint::prune_checkpoints(ckpt_dir, config.memory.max_checkpoints) {
                Ok(n) if n > 0 => println!("  [Memory] Pruned {n} old checkpoint(s)"),
                _ => {}
            }
        }
        Err(e) => {
            println!("  [Memory] Checkpoint failed: {e}");
        }
    }
}

/// Merges accumulated `LoRA` deltas into the base model weights when magnitude and
/// adaptation count thresholds are met. Clears the experience buffer afterward since
/// learned knowledge is now permanent in the weights.
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
    let max_magnitude = {
        let m = mind.read().unwrap();
        m.active_lora().iter()
            .map(mind::lora::LoRALayer::magnitude)
            .fold(0.0f32, f32::max)
    };
    if consolidation.should_consolidate(adaptation_count, max_magnitude) {
        println!(
            "  [Memory] Consolidation criteria met ({adaptation_count} adaptations, magnitude {max_magnitude:.4})"
        );
        drop(p);
        let mut mind_guard = mind.write().unwrap();
        let result = consolidation.consolidate(&mut mind_guard);
        println!(
            "  [Memory] Consolidation complete: evaluated={}, merged={}, magnitude={:.4}",
            result.layers_evaluated, result.layers_merged, result.new_magnitude
        );
        drop(mind_guard);
        if let Ok(mut p) = proprio.write() {
            p.consolidations += result.layers_merged as u64;
        }

        do_checkpoint(config, experience_buf, proprio, plugins, soma_state, Some(mind));

        if let Ok(mut buf) = experience_buf.write() {
            let cleared = buf.len();
            buf.clear();
            println!("  [Memory] Experience buffer cleared ({cleared} entries)");
        }
    } else {
        println!(
            "  [Memory] Consolidation not needed (adaptations: {}/{}, magnitude: {:.4}/{:.4})",
            adaptation_count, consolidation.threshold, max_magnitude, consolidation.min_lora_magnitude
        );
    }
}

/// Formats a duration as a human-readable string (e.g., "2h 14m 7s").
fn format_uptime(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s}s")
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        format!("{h}h {m}m {s}s")
    }
}

/// Graceful shutdown sequence:
/// 1. Stop accepting new requests
/// 2. Notify peers with CLOSE signals
/// 3. Drain in-flight requests (wait for `ACTIVE_INFERENCES` to reach 0)
/// 4. Auto-checkpoint
/// 5. Unload plugins (`on_unload` lifecycle)
/// 6. Close listeners
/// 7. Exit
#[allow(clippy::too_many_arguments, clippy::significant_drop_tightening)]
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
    ACCEPTING_CONNECTIONS.store(false, Ordering::SeqCst);
    tracing::info!("Stopped accepting new connections");

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
                        rt.block_on(async {
                            drop(
                                protocol::client::SynapseClient::send(
                                    &addr, &sender, &signal,
                                ).await
                            );
                        });
                    }
                });
                handles.push(handle);
            }
            for handle in handles {
                let _ = handle.join();
            }
        }
    }

    // Wait up to 10s for in-flight inferences to complete before forcing shutdown.
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

    tracing::debug!("Outbound queues flushed (per-connection, drained on close)");

    if config.memory.auto_checkpoint {
        do_checkpoint(config, experience_buf, proprio, plugins, soma_state, mind);
    }

    // Unload plugins before closing listeners so plugin cleanup can still use the network.
    {
        let plugin_count = plugins.read().unwrap().plugin_names().len();
        plugins.write().unwrap().unload_all();
        tracing::info!(plugins = plugin_count, "Plugins unloaded");
    }

    if let Some(handle) = server_handle {
        handle.abort();
        tracing::info!("Protocol server stopped");
    }

    tracing::info!("MCP server closed");

    let p = proprio.read().unwrap();
    let uptime = format_uptime(p.uptime());
    let total_inferences = p.total_inferences;
    let experience_count = p.experience_count;
    let total_decisions = p.total_decisions_recorded;
    drop(p);
    println!(
        "  SOMA shutdown. Uptime: {uptime}, Inferences: {total_inferences}, Experiences: {experience_count}, Decisions: {total_decisions}"
    );
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut config = SomaConfig::load(&cli.config)?;
    config.apply_env_overrides();

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
                "  Warning: ignoring invalid peer format '{peer_str}' (expected name:host:port)"
            );
        }
    }

    let log_filter = format!("soma={}", config.soma.log_level);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(log_filter.parse()?);

    if std::env::var("SOMA_LOG_JSON").is_ok() {
        // with_span_list embeds the active span stack into each JSON log line,
        // giving structured trace_id/span_id/parent_span_id for observability.
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

    let proprio = Arc::new(RwLock::new(Proprioception::new()));

    let model_dir = cli
        .model
        .unwrap_or_else(|| PathBuf::from(&config.mind.model_dir));
    let mut engine = OnnxMindEngine::load(&model_dir)
        .inspect_err(|_| {
            eprintln!("  Error: Failed to load Mind model from {}", model_dir.display());
            eprintln!("  Ensure encoder.onnx, decoder.onnx, tokenizer.json, and meta.json exist.");
        })?;
    engine.temperature = config.mind.temperature;
    engine.max_inference_time_secs = config.mind.max_inference_time_secs;
    let mind = Arc::new(RwLock::new(engine));

    // Verify the model can produce a non-empty program before accepting real intents.
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
                anyhow::bail!("Model verification failed: {e}");
            }
        }
    }

    let mind_info = {
        let m = mind.read().unwrap();
        m.info()
    };

    let mut plugins = PluginManager::new();

    // Extracts per-plugin settings from [plugins.<name>] TOML sections into PluginConfig.
    let build_plugin_config = |name: &str| -> plugin::interface::PluginConfig {
        let mut pc = plugin::interface::PluginConfig::default();
        if let Some(toml_val) = config.plugins.get(name)
            && let Some(table) = toml_val.as_table() {
                for (k, v) in table {
                    if let Ok(json_val) = serde_json::to_value(v) {
                        pc.settings.insert(k.clone(), json_val);
                    }
                }
            }
        pc
    };

    {
        let mut posix = PosixPlugin::new();
        let pc = build_plugin_config("posix");
        if let Err(e) = posix.on_load(&pc) {
            tracing::warn!(error = %e, "PosixPlugin on_load failed");
        }
        let posix: Box<dyn plugin::interface::SomaPlugin> = Box::new(posix);
        plugins.register(posix);
    }

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

    // Plugins can ship LoRA weights that teach the Mind their conventions at load time.
    {
        let lora_plugins = plugins.plugins_with_lora_weights();
        if !lora_plugins.is_empty() {
            let mut attached_count = 0usize;
            for name in &lora_plugins {
                if let Some(lora_data) = plugins.get_plugin_lora_weights(name) {
                    let attach_result = mind.write().unwrap().attach_lora_bytes(name, &lora_data);
                    match attach_result {
                        Ok(()) => {
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

    // Map model catalog convention IDs to plugin manager IDs and warn about any gaps.
    {
        let m = mind.read().unwrap();
        let catalog = m.meta().catalog.clone();
        drop(m);
        plugins.build_catalog_routing(&catalog);

        let mut missing = Vec::new();
        for entry in &catalog {
            if entry.name == "EMIT" || entry.name == "STOP" {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)] // catalog IDs are small
            if plugins.resolve_catalog_id(entry.id as u32) == entry.id as u32
                && plugins.resolve_by_name(&entry.name).is_none() {
                    missing.push((entry.id, entry.name.clone()));
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

    let soma_metrics = Arc::new(SomaMetrics::new());
    plugins.set_metrics(soma_metrics.clone());

    {
        let plugin_names = plugins.plugin_names();
        let mut p = proprio.write().unwrap();
        p.set_plugins(plugin_names);
    }

    let experience_buf = Arc::new(RwLock::new(ExperienceBuffer::new(
        config.memory.max_experience_buffer,
    )));

    let reflex_layer = Arc::new(RwLock::new(mind::reflex::ReflexLayer::new(10_000, 0.9)));

    let soma_state = Arc::new(RwLock::new(SomaState::new(
        config.mcp.max_execution_history,
    )));

    let peer_registry = Arc::new(RwLock::new(PeerRegistry::new()));
    {
        let mut pr = peer_registry.write().unwrap();
        pr.load_from_config(&config.protocol.peers);
    }

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
        drop(auth);
    }

    // Restores accumulated state from a checkpoint: LoRA weights, plugin states,
    // decisions, and execution history. Validates base model hash to prevent applying
    // LoRA weights from a different model version.
    let restore_checkpoint = |ckpt: &Checkpoint,
                              proprio: &Arc<RwLock<Proprioception>>,
                              soma_state: &Arc<RwLock<SomaState>>,
                              mind: &Arc<RwLock<OnnxMindEngine>>,
                              plugins: &mut PluginManager| {
        if !ckpt.base_model_hash.is_empty() {
            let current_hash = mind.read().unwrap().model_hash.clone();
            if ckpt.base_model_hash == current_hash {
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
                            Err(e) => eprintln!("  Warning: Failed to restore LoRA state: {e}"),
                        }
                    }
                }
                if !ckpt.merged_opcode_delta.is_empty()
                    && let Ok(mut m) = mind.write()
                {
                    m.set_merged_opcode_delta(ckpt.merged_opcode_delta.clone());
                    eprintln!(
                        "  Restored consolidated weight delta ({} values)",
                        ckpt.merged_opcode_delta.len()
                    );
                }
            } else {
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
            }
        }
        if let Ok(mut p) = proprio.write() {
            p.experience_count = ckpt.experience_count;
            p.total_adaptations = ckpt.adaptation_count;
        }

        if !ckpt.plugin_states.is_empty() {
            let states: Vec<(String, serde_json::Value)> = ckpt.plugin_states.iter()
                .map(|e| (e.plugin_name.clone(), e.state.clone()))
                .collect();
            plugins.restore_plugin_states(&states);
            eprintln!("  Restored plugin states ({} plugins)", ckpt.plugin_states.len());
        }

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

    let plugins_arc = Arc::new(RwLock::new(plugins));
    let bind_addr = config.protocol.bind.clone();
    let server_handler = SomaSignalHandler {
        name: config.soma.id.clone(),
        mind: mind.clone(),
        plugins: plugins_arc.clone(),
        max_program_steps: config.mind.max_program_steps,
        reflex_layer: reflex_layer.clone(),
    };

    let server = SynapseServer::new(config.soma.id.clone(), bind_addr.clone())
        .with_metrics(soma_metrics.clone())
        .with_peer_registry(peer_registry.clone());
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start(server_handler).await {
            tracing::error!(error = %e, "Protocol server error");
        }
    });

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
            reflex_layer: reflex_layer.clone(),
        };

        eprintln!("============================================================");
        eprintln!("  SOMA v0.1.0 -- MCP Server Mode");
        eprintln!("  LLM can now drive SOMA via Model Context Protocol.");
        eprintln!("============================================================");
        eprintln!("  ID:       {}", config.soma.id);
        eprintln!("  Mind:     {} ({}conv)", mind_info.backend, mind_info.conventions_known);
        eprintln!("  Plugins:  posix ({total_conv} conventions)");
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

    eprintln!("============================================================");
    eprintln!("  SOMA v0.1.0 -- Rust Runtime");
    eprintln!("  Neural mind drives hardware directly. Single binary.");
    eprintln!("============================================================");
    eprintln!("  ID:       {}", config.soma.id);
    eprintln!("  Mind:     {} ({}conv)", mind_info.backend, mind_info.conventions_known);
    eprintln!("  Plugins:  posix ({total_conv} conventions)");
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
            &reflex_layer,
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

        // spawn_blocking keeps the tokio reactor free for async plugin I/O.
        let input = tokio::task::spawn_blocking(|| {
            let mut buf = String::new();
            match io::stdin().read_line(&mut buf) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(buf),
            }
        }).await?;
        let Some(input) = input else {
            println!();
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state), Some(&mind));
            break;
        };
        let text = input.trim();
        if text.is_empty() {
            continue;
        }
        if text == "quit" || text == "exit" || text == "q" {
            println!("\n  SOMA shutting down.");
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state), Some(&mind));
            break;
        }

        if text == ":status" {
            let info = {
                let m = mind.read().unwrap();
                m.info()
            };
            let p = proprio.read().unwrap();
            let report = p.report();
            drop(p);
            let exp = experience_buf.read().unwrap();
            let exp_len = exp.len();
            let exp_total = exp.total_seen();
            drop(exp);
            let st = soma_state.read().unwrap();
            let decisions_len = st.decisions.len();
            let executions_len = st.executions.len();
            drop(st);
            let conv_count = plugins_arc.read().unwrap().conventions().len();
            println!("\n  [Proprioception]");
            println!("    {}", report.replace('\n', "\n    "));
            println!("    Mind:        {}", info.backend);
            println!("    Conventions: {}", info.conventions_known);
            println!("    Max steps:   {}", info.max_steps);
            println!(
                "    LoRA:        {} layers, magnitude {:.6}",
                info.lora_layers, info.lora_magnitude
            );
            println!("    Plugins:     {conv_count} loaded");
            println!(
                "    Experience:  {exp_len}/{} buffer ({exp_total} total seen)",
                config.memory.max_experience_buffer,
            );
            println!("    State:       {decisions_len} decisions, {executions_len} executions");
            #[allow(clippy::significant_drop_tightening)]
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
            let conventions = plugins_arc.read().unwrap().conventions();
            for conv in conventions {
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
            let decisions = st.decisions.list().to_vec();
            drop(st);
            if decisions.is_empty() {
                println!("\n  [State] No decisions recorded.");
            } else {
                println!("\n  [State] Decision log ({} entries):", decisions.len());
                for d in &decisions {
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
            let warnings: Vec<(String, String)> = pm.check_plugin_health()
                .into_iter()
                .map(|(n, m)| (n, m.to_string()))
                .collect();
            drop(pm);
            if warnings.is_empty() {
                println!("\n  [Health] All plugins healthy.");
            } else {
                println!("\n  [Health] Plugin warnings:");
                for (name, msg) in &warnings {
                    println!("    {name} — {msg}");
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
            &reflex_layer,
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
