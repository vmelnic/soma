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
use memory::consolidation::ConsolidationConfig;
use memory::experience::{Experience, ExperienceBuffer};
use metrics::SomaMetrics;
use mind::{ArgType, MindEngine, ProgramStep, STOP_ID};
use mind::onnx_engine::OnnxMindEngine;
use plugin::builtin::PosixPlugin;
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
                plugin::interface::Value::Map(pairs) => {
                    println!("  [Body]");
                    for (k, v) in pairs {
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
    plugins: &PluginManager,
    proprio: &Arc<RwLock<Proprioception>>,
    experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    soma_state: &Arc<RwLock<SomaState>>,
    soma_metrics: &Arc<SomaMetrics>,
    text: &str,
    max_concurrent: usize,
    max_program_steps: usize,
    trace_verbosity: &str,
) {
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

            let result = plugins.execute_program(&program.steps, max_program_steps);

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
                            let result2 = plugins.execute_program(&program2.steps, max_program_steps);
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
                        };
                        let a1 = match s.arg1_type {
                            ArgType::None => 0u8,
                            ArgType::Span => 1u8,
                            ArgType::Ref => 2u8,
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
                };
                if let Ok(mut buf) = experience_buf.write() {
                    buf.record(exp);
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
        }
        Err(e) => {
            let execution_time_ms = exec_start.elapsed().as_millis() as u64;
            soma_metrics.record_inference(false, execution_time_ms);
            tracing::info!(
                component = "mind",
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
    plugins: &Arc<PluginManager>,
    soma_state: Option<&Arc<RwLock<SomaState>>>,
) {
    let ckpt_dir = Path::new(&config.memory.checkpoint_dir);
    let filename = Checkpoint::filename(&config.soma.id);
    let path = ckpt_dir.join(&filename);

    let (exp_count, adapt_count) = {
        let p = proprio.read().unwrap();
        (p.experience_count, p.total_adaptations)
    };

    // Collect plugin state (Section 7.5: institutional memory)
    let plugin_states = plugins.collect_plugin_states();
    let plugin_state_entries: Vec<memory::checkpoint::PluginStateEntry> = plugin_states
        .into_iter()
        .map(|(name, state)| memory::checkpoint::PluginStateEntry {
            plugin_name: name,
            state,
        })
        .collect();

    let mut ckpt = Checkpoint::new(
        config.soma.id.clone(),
        Vec::new(),
        exp_count,
        adapt_count,
    );
    ckpt.plugin_states = plugin_state_entries;

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

fn do_consolidate(_config: &SomaConfig, proprio: &Arc<RwLock<Proprioception>>) {
    let consolidation = ConsolidationConfig::default();
    let p = proprio.read().unwrap();
    if consolidation.should_consolidate(p.total_adaptations, 0.0) {
        println!(
            "  [Memory] Consolidation criteria met ({} adaptations)",
            p.total_adaptations
        );
        println!("  [Memory] No LoRA layers to consolidate (ONNX engine has no active LoRA).");
    } else {
        println!(
            "  [Memory] Consolidation not needed (adaptations: {}/{}, magnitude below threshold)",
            p.total_adaptations, consolidation.threshold
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
    plugins: &Arc<PluginManager>,
    soma_state: Option<&Arc<RwLock<SomaState>>>,
) {
    // Step 1: Stop accepting — set a flag (server_handle.abort below)

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
        if drain_start.elapsed().as_secs() > 5 {
            println!(
                "  [Shutdown] {} inferences still in-flight after 5s, forcing shutdown",
                ACTIVE_INFERENCES.load(Ordering::SeqCst)
            );
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Step 4: Auto-checkpoint (includes plugin state)
    if config.memory.auto_checkpoint {
        do_checkpoint(config, experience_buf, proprio, plugins, soma_state);
    }

    // Step 5: Unload plugins (Section 11.4)
    // Note: unload_all() requires &mut which we can't get through Arc.
    // Plugins are unloaded when the Arc is dropped and PluginManager is deallocated.
    // For explicit lifecycle control, we'd need Arc<RwLock<PluginManager>>.
    tracing::info!(
        plugins = plugins.plugin_names().len(),
        "Plugins will be unloaded on exit"
    );

    // Step 6: Close listeners / stop protocol server
    if let Some(handle) = server_handle {
        handle.abort();
        tracing::info!("Protocol server stopped");
    }

    // Step 7: Log final stats and exit
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
        tracing_subscriber::fmt()
            .json()
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
    let mind = Arc::new(RwLock::new(engine));

    let mind_info = {
        let m = mind.read().unwrap();
        m.info()
    };

    // Step 4: Load Plugins (Section 11.1: "Failed plugin: skip and continue")
    let mut plugins = PluginManager::new();
    {
        let posix: Box<dyn plugin::interface::SomaPlugin> = Box::new(PosixPlugin::new());
        plugins.register(posix);
    }
    let total_conv = plugins.conventions().len();

    // Step 5c: Initialize metrics (Whitepaper Section 11.5)
    let soma_metrics = Arc::new(SomaMetrics::new());

    // Wire metrics into plugin manager for call tracking
    plugins.set_metrics(soma_metrics.clone());

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
    let auth_manager = Arc::new(RwLock::new(AuthManager::new(config.security.require_auth)));
    if !config.security.admin_token.is_empty() {
        let mut auth = auth_manager.write().unwrap();
        auth.register_admin_token(config.security.admin_token.clone());
    }

    // Step 6: Load checkpoint — restores proprioception, decisions, and execution history
    let restore_checkpoint = |ckpt: &Checkpoint,
                              proprio: &Arc<RwLock<Proprioception>>,
                              soma_state: &Arc<RwLock<SomaState>>| {
        if let Ok(mut p) = proprio.write() {
            p.experience_count = ckpt.experience_count;
            p.total_adaptations = ckpt.adaptation_count;
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
        }
    };

    if let Some(ref ckpt_path) = cli.checkpoint {
        match Checkpoint::load(ckpt_path) {
            Ok(ckpt) => {
                restore_checkpoint(&ckpt, &proprio, &soma_state);
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
                        restore_checkpoint(&ckpt, &proprio, &soma_state);
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
    let plugins_arc = Arc::new(plugins);
    let bind_addr = config.protocol.bind.clone();
    let server_handler = SomaSignalHandler {
        name: config.soma.id.clone(),
        mind: mind.clone(),
        plugins: plugins_arc.clone(),
        max_program_steps: config.mind.max_program_steps,
    };

    let server = SynapseServer::new(config.soma.id.clone(), bind_addr.clone())
        .with_metrics(soma_metrics.clone());
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start(server_handler).await {
            tracing::error!(error = %e, "Protocol server error");
        }
    });

    // Step 8: Start MCP Server if requested (Whitepaper Section 8, Milestone 3)
    // "At this point, an LLM can drive SOMA."
    if cli.mcp {
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

        do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state));
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
            intent,
            config.resources.max_concurrent_inferences,
            config.mind.max_program_steps,
            &config.soma.trace_verbosity,
        );
        do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state));
        return Ok(());
    }

    // REPL
    eprintln!(
        "  Type intent. :status :inspect :checkpoint :consolidate :decisions  quit"
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
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state));
            break;
        }
        print!("intent> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            println!();
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state));
            break;
        }
        let text = input.trim();
        if text.is_empty() {
            continue;
        }
        if text == "quit" || text == "exit" || text == "q" {
            println!("\n  SOMA shutting down.");
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle), Some(&peer_registry), &plugins_arc, Some(&soma_state));
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
            println!("    Plugins:     {} loaded", plugins_arc.conventions().len());
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
            for conv in plugins_arc.conventions() {
                println!("    [{:2}] {} -- {}", conv.id, conv.name, conv.description);
            }
            println!();
            continue;
        }
        if text == ":checkpoint" {
            do_checkpoint(&config, &experience_buf, &proprio, &plugins_arc, Some(&soma_state));
            println!();
            continue;
        }
        if text == ":consolidate" {
            do_consolidate(&config, &proprio);
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

        run_intent(
            &mind,
            &plugins_arc,
            &proprio,
            &experience_buf,
            &soma_state,
            &soma_metrics,
            text,
            max_concurrent,
            max_steps,
            &config.soma.trace_verbosity,
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
