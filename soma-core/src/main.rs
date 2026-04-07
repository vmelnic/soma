mod config;
mod errors;
mod mind;
mod plugin;
mod protocol;
mod memory;
mod proprioception;

use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use config::SomaConfig;
use memory::checkpoint::Checkpoint;
use memory::consolidation::ConsolidationConfig;
use memory::experience::{Experience, ExperienceBuffer};
use mind::{MindEngine, ProgramStep, ArgType, STOP_ID};
use mind::onnx_engine::OnnxMindEngine;
use plugin::builtin::PosixPlugin;
use plugin::manager::PluginManager;
use proprioception::Proprioception;
use protocol::discovery::PeerRegistry;
use protocol::server::{SynapseServer, SomaSignalHandler};

// --- Gap 7: CLI flags ---
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
}

/// Active inference counter for resource limiting (Gap 4).
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
    text: &str,
    max_concurrent: usize,
    max_program_steps: usize,
) {
    // --- Gap 4: Enforce max_concurrent_inferences ---
    let current = ACTIVE_INFERENCES.fetch_add(1, Ordering::SeqCst);
    if current >= max_concurrent {
        ACTIVE_INFERENCES.fetch_sub(1, Ordering::SeqCst);
        eprintln!("  [Resource] Error: max concurrent inferences exceeded ({}/{})", current + 1, max_concurrent);
        return;
    }

    let exec_start = std::time::Instant::now();
    // Generate trace_id for this request (Section 18.1.2)
    let trace_id = uuid::Uuid::new_v4().to_string()[..12].to_string();

    let mind_guard = mind.read().unwrap();
    match mind_guard.infer(text) {
        Ok(program) => {
            let real: Vec<&ProgramStep> = program.steps.iter()
                .filter(|s| s.conv_id != STOP_ID)
                .collect();
            println!("\n  [Mind] Program ({} steps, {:.0}%):", real.len(), program.confidence * 100.0);
            for (i, step) in program.steps.iter().enumerate() {
                println!("    {}", step.format(i, &mind_guard.meta().catalog));
                if step.conv_id == STOP_ID { break; }
            }
            println!();

            // --- Gap 4/5: Pass max_program_steps to execute_program ---
            let result = plugins.execute_program(&program.steps, max_program_steps);
            for entry in &result.trace {
                if entry.op != "STOP" && entry.op != "EMIT" && !entry.summary.is_empty() {
                    println!("    [{}] {} ... {}", entry.step, entry.op, entry.summary);
                }
            }
            display_result(&result, &mind_guard.meta().catalog);

            let execution_time_ms = exec_start.elapsed().as_millis() as u64;

            // --- Structured logging (Section 18.1.1) ---
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

            // --- Gap 9: Record full program info ---
            let tokens: Vec<u32> = mind_guard.tokenizer.encode(text)
                .iter()
                .map(|&t| t as u32)
                .collect();
            let program_data: Vec<(i32, u8, u8)> = program.steps.iter().map(|s| {
                let a0 = match s.arg0_type { ArgType::None => 0u8, ArgType::Span => 1u8, ArgType::Ref => 2u8 };
                let a1 = match s.arg1_type { ArgType::None => 0u8, ArgType::Span => 1u8, ArgType::Ref => 2u8 };
                (s.conv_id, a0, a1)
            }).collect();
            let exp = Experience {
                intent_tokens: tokens,
                program: program_data,
                success: result.success,
                execution_time_ms,
                timestamp: std::time::Instant::now(),
            };
            if let Ok(mut buf) = experience_buf.write() {
                buf.record(exp);
            }

            // Update proprioception
            if let Ok(mut p) = proprio.write() {
                if result.success {
                    p.record_success();
                } else {
                    p.record_failure();
                }
            }
        }
        Err(e) => {
            tracing::info!(
                component = "mind",
                trace_id = %trace_id,
                intent = %text,
                success = false,
                "Intent failed: {}", e
            );
            println!("  [Mind] Error: {}", e);
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
) {
    let ckpt_dir = Path::new(&config.memory.checkpoint_dir);
    let filename = Checkpoint::filename(&config.soma.id);
    let path = ckpt_dir.join(&filename);

    let (exp_count, adapt_count) = {
        let p = proprio.read().unwrap();
        (p.experience_count, p.total_adaptations)
    };

    let ckpt = Checkpoint::new(
        config.soma.id.clone(),
        Vec::new(), // No LoRA layers in the ONNX engine yet
        exp_count,
        adapt_count,
    );

    match ckpt.save(&path) {
        Ok(()) => {
            println!("  [Memory] Checkpoint saved: {}", path.display());
            // Prune old checkpoints
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
        println!("  [Memory] Consolidation criteria met ({} adaptations)", p.total_adaptations);
        println!("  [Memory] No LoRA layers to consolidate (ONNX engine has no active LoRA).");
    } else {
        println!(
            "  [Memory] Consolidation not needed (adaptations: {}/{}, magnitude below threshold)",
            p.total_adaptations, consolidation.threshold
        );
    }
}

/// Format uptime duration as human-readable string.
fn format_uptime(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}

/// --- Gap 6: Proper shutdown sequence ---
fn do_shutdown(
    config: &SomaConfig,
    experience_buf: &Arc<RwLock<ExperienceBuffer>>,
    proprio: &Arc<RwLock<Proprioception>>,
    server_handle: Option<&tokio::task::JoinHandle<()>>,
) {
    // 1. Auto-checkpoint
    if config.memory.auto_checkpoint {
        do_checkpoint(config, experience_buf, proprio);
    }

    // 2. Log final stats
    let p = proprio.read().unwrap();
    let uptime = format_uptime(p.uptime());
    println!("  SOMA shutdown. Uptime: {}, Inferences: {}, Experiences: {}",
        uptime, p.total_inferences, p.experience_count);

    // 3. Stop protocol server (if running)
    if let Some(handle) = server_handle {
        handle.abort();
        tracing::info!("Protocol server stopped");
    }
}

// --- Gap 1: Async main with tokio ---
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // --- Step 1: Load configuration (Spec Section 15) ---
    let mut config = SomaConfig::load(&cli.config)?;

    // --- Gap 7: Apply CLI overrides ---
    if let Some(ref log_level) = cli.log_level {
        config.soma.log_level = log_level.clone();
    }
    if let Some(ref bind) = cli.bind {
        config.protocol.bind = bind.clone();
    }
    // Parse --peer flags (name:host:port format)
    for peer_str in &cli.peer {
        let parts: Vec<&str> = peer_str.splitn(2, ':').collect();
        if parts.len() == 2 {
            config.protocol.peers.insert(parts[0].to_string(), parts[1].to_string());
        } else {
            eprintln!("  Warning: ignoring invalid peer format '{}' (expected name:host:port)", peer_str);
        }
    }

    // Initialize logging with configured level (Section 18.1)
    // Development: pretty-print. Production (SOMA_LOG_JSON=1): JSON lines with soma_id.
    let log_filter = format!("soma={}", config.soma.log_level);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(log_filter.parse()?);

    if std::env::var("SOMA_LOG_JSON").is_ok() {
        // Production: JSON lines format (Section 18.1.1)
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(env_filter)
            .init();
    } else {
        // Development: pretty-print
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }

    // --- Step 2: Initialize proprioception (Spec Section 7) ---
    let proprio = Arc::new(RwLock::new(Proprioception::new()));

    // --- Step 3: Load Mind Engine (concurrency-ready with Arc<RwLock<>>) ---
    let model_dir = cli.model
        .unwrap_or_else(|| PathBuf::from(&config.mind.model_dir));
    let mind = Arc::new(RwLock::new(OnnxMindEngine::load(&model_dir)?));

    let mind_info = {
        let m = mind.read().unwrap();
        m.info()
    };

    // --- Step 4: Load Plugins ---
    let mut plugins = PluginManager::new();
    plugins.register(Box::new(PosixPlugin::new()));
    let total_conv = plugins.conventions().len();

    // --- Step 5: Initialize memory system ---
    let experience_buf = Arc::new(RwLock::new(
        ExperienceBuffer::new(config.memory.max_experience_buffer)
    ));

    // --- Step 5b: Initialize peer registry (Gap 5: config.protocol.peers) ---
    let mut peer_registry = PeerRegistry::new();
    peer_registry.load_from_config(&config.protocol.peers);

    // --- Step 6: Load checkpoint ---
    if let Some(ref ckpt_path) = cli.checkpoint {
        // Gap 7: --checkpoint flag: load specific checkpoint file
        match Checkpoint::load(ckpt_path) {
            Ok(ckpt) => {
                if let Ok(mut p) = proprio.write() {
                    p.experience_count = ckpt.experience_count;
                    p.total_adaptations = ckpt.adaptation_count;
                }
                eprintln!("  Resumed from {}: {} experiences, {} adaptations",
                    ckpt_path.display(), ckpt.experience_count, ckpt.adaptation_count);
            }
            Err(e) => {
                eprintln!("  Warning: Failed to load checkpoint {}: {}", ckpt_path.display(), e);
            }
        }
    } else {
        let ckpt_dir = Path::new(&config.memory.checkpoint_dir);
        match Checkpoint::list_checkpoints(ckpt_dir) {
            Ok(ckpts) if !ckpts.is_empty() => {
                match Checkpoint::load(&ckpts[0]) {
                    Ok(ckpt) => {
                        if let Ok(mut p) = proprio.write() {
                            p.experience_count = ckpt.experience_count;
                            p.total_adaptations = ckpt.adaptation_count;
                        }
                        eprintln!("  Resumed: {} experiences, {} adaptations",
                            ckpt.experience_count, ckpt.adaptation_count);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to load checkpoint, starting fresh");
                    }
                }
            }
            _ => {}
        }
    }

    // --- Gap 1/5: Start protocol server (SynapseServer) ---
    let plugins_arc = Arc::new(plugins);
    let bind_addr = config.protocol.bind.clone();
    let server_handler = SomaSignalHandler {
        name: config.soma.id.clone(),
        mind: mind.clone(),
        plugins: plugins_arc.clone(),
        max_program_steps: config.mind.max_program_steps,
    };

    let server = SynapseServer::new(config.soma.id.clone(), bind_addr.clone());
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start(server_handler).await {
            tracing::error!(error = %e, "Protocol server error");
        }
    });

    // --- Step 7: Ready ---
    eprintln!("============================================================");
    eprintln!("  SOMA v0.1.0 -- Rust Runtime");
    eprintln!("  Neural mind drives libc directly. Single binary.");
    eprintln!("============================================================");
    eprintln!("  ID:       {}", config.soma.id);
    eprintln!("  Mind:     {} ({}conv)", mind_info.backend, mind_info.conventions_known);
    eprintln!("  Plugins:  posix ({} conventions)", total_conv);
    eprintln!("  Model:    {}", model_dir.display());
    eprintln!("  Protocol: {} (server started)", config.protocol.bind);
    if peer_registry.count() > 0 {
        eprintln!("  Peers:    {} configured", peer_registry.count());
        for peer in peer_registry.list() {
            eprintln!("            {} -> {}", peer.name, peer.addr);
        }
    }
    eprintln!("  Resources: max {} concurrent inferences, {} max program steps",
        config.resources.max_concurrent_inferences, config.mind.max_program_steps);
    eprintln!("============================================================");

    if let Some(intent) = &cli.intent {
        run_intent(&mind, &plugins_arc, &proprio, &experience_buf, intent,
            config.resources.max_concurrent_inferences, config.mind.max_program_steps);
        do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle));
        return Ok(());
    }

    // REPL (Spec Section 18.3)
    eprintln!("  Type intent. :status :inspect :checkpoint :consolidate  quit");
    eprintln!();

    // Ctrl+C handler for graceful shutdown (Spec Section 16)
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc_handler(r);

    let max_concurrent = config.resources.max_concurrent_inferences;
    let max_steps = config.mind.max_program_steps;

    loop {
        if !running.load(Ordering::Relaxed) {
            println!("\n  SOMA shutting down (SIGINT).");
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle));
            break;
        }
        print!("intent> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            // EOF
            println!();
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle));
            break;
        }
        let text = input.trim();
        if text.is_empty() { continue; }
        if text == "quit" || text == "exit" || text == "q" {
            println!("\n  SOMA shutting down.");
            do_shutdown(&config, &experience_buf, &proprio, Some(&server_handle));
            break;
        }

        // Debug REPL commands (Spec Section 18.3)
        if text == ":status" {
            let info = {
                let m = mind.read().unwrap();
                m.info()
            };
            let p = proprio.read().unwrap();
            let exp = experience_buf.read().unwrap();
            println!("\n  [Proprioception]");
            println!("    {}", p.report().replace('\n', "\n    "));
            println!("    Mind:        {}", info.backend);
            println!("    Conventions: {}", info.conventions_known);
            println!("    Max steps:   {}", info.max_steps);
            println!("    LoRA:        {} layers, magnitude {:.6}", info.lora_layers, info.lora_magnitude);
            println!("    Plugins:     {} loaded", plugins_arc.conventions().len());
            println!("    Experience:  {}/{} buffer ({} total seen)",
                exp.len(), config.memory.max_experience_buffer, exp.total_seen());
            // Gap 5: Display peers in :status
            println!("    Peers:       {} configured", peer_registry.count());
            for peer in peer_registry.list() {
                println!("                 {} -> {}", peer.name, peer.addr);
            }
            println!("    Protocol:    {}", config.protocol.bind);
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
            do_checkpoint(&config, &experience_buf, &proprio);
            println!();
            continue;
        }
        if text == ":consolidate" {
            do_consolidate(&config, &proprio);
            println!();
            continue;
        }

        run_intent(&mind, &plugins_arc, &proprio, &experience_buf, text,
            max_concurrent, max_steps);
        println!();
    }

    Ok(())
}

fn ctrlc_handler(running: Arc<AtomicBool>) {
    let _ = ctrlc::set_handler(move || {
        running.store(false, Ordering::Relaxed);
    });
}
