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
use std::sync::{Arc, RwLock};

use config::SomaConfig;
use memory::checkpoint::Checkpoint;
use memory::consolidation::ConsolidationConfig;
use memory::experience::{Experience, ExperienceBuffer};
use mind::{MindEngine, ProgramStep, STOP_ID};
use mind::onnx_engine::OnnxMindEngine;
use plugin::builtin::PosixPlugin;
use plugin::manager::PluginManager;
use proprioception::Proprioception;

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
}

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
) {
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

            let result = plugins.execute_program(&program.steps);
            for entry in &result.trace {
                if entry.op != "STOP" && entry.op != "EMIT" && !entry.summary.is_empty() {
                    println!("    [{}] {} ... {}", entry.step, entry.op, entry.summary);
                }
            }
            display_result(&result, &mind_guard.meta().catalog);

            // Record experience
            let tokens: Vec<u32> = mind_guard.tokenizer.encode(text)
                .iter()
                .map(|&t| t as u32)
                .collect();
            let exp = Experience {
                intent_tokens: tokens,
                program_steps: program.steps.len(),
                success: result.success,
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
            println!("  [Mind] Error: {}", e);
            if let Ok(mut p) = proprio.write() {
                p.record_failure();
            }
        }
    }
}

fn do_checkpoint(
    config: &SomaConfig,
    experience_buf: &Arc<RwLock<ExperienceBuffer>>,
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

fn do_consolidate(config: &SomaConfig, proprio: &Arc<RwLock<Proprioception>>) {
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    // --- Step 1: Load configuration (Spec Section 15) ---
    let config = SomaConfig::load(&cli.config)?;

    // Initialize logging with configured level
    let log_filter = format!("soma={}", config.soma.log_level);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(log_filter.parse()?)
        )
        .init();

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

    // --- Step 6: Load last checkpoint if available ---
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

    // --- Step 7: Ready ---
    eprintln!("============================================================");
    eprintln!("  SOMA v0.1.0 -- Rust Runtime");
    eprintln!("  Neural mind drives libc directly. Single binary.");
    eprintln!("============================================================");
    eprintln!("  ID:       {}", config.soma.id);
    eprintln!("  Mind:     {} ({}conv)", mind_info.backend, mind_info.conventions_known);
    eprintln!("  Plugins:  posix ({} conventions)", total_conv);
    eprintln!("  Model:    {}", model_dir.display());
    eprintln!("  Protocol: {}", config.protocol.bind);
    eprintln!("============================================================");

    if let Some(intent) = &cli.intent {
        run_intent(&mind, &plugins, &proprio, &experience_buf, intent);
        return Ok(());
    }

    // REPL (Spec Section 18.3)
    eprintln!("  Type intent. :status :inspect :checkpoint :consolidate  quit");
    eprintln!();

    // Ctrl+C handler for graceful shutdown (Spec Section 16)
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();
    ctrlc_handler(r);

    loop {
        if !running.load(std::sync::atomic::Ordering::Relaxed) {
            println!("\n  SOMA shutting down (SIGINT).");
            // Auto-checkpoint on shutdown
            if config.memory.auto_checkpoint {
                do_checkpoint(&config, &experience_buf, &proprio);
            }
            break;
        }
        print!("intent> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            // EOF — auto-checkpoint
            if config.memory.auto_checkpoint {
                do_checkpoint(&config, &experience_buf, &proprio);
            }
            break;
        }
        let text = input.trim();
        if text.is_empty() { continue; }
        if text == "quit" || text == "exit" || text == "q" {
            println!("\n  SOMA shutting down.");
            if config.memory.auto_checkpoint {
                do_checkpoint(&config, &experience_buf, &proprio);
            }
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
            println!("    Plugins:     {} loaded", plugins.conventions().len());
            println!("    Experience:  {}/{} buffer ({} total seen)",
                exp.len(), config.memory.max_experience_buffer, exp.total_seen());
            println!();
            continue;
        }
        if text == ":inspect" || text == "help" || text == "?" {
            println!("\n  [Conventions]");
            for conv in plugins.conventions() {
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

        run_intent(&mind, &plugins, &proprio, &experience_buf, text);
        println!();
    }

    Ok(())
}

fn ctrlc_handler(running: Arc<std::sync::atomic::AtomicBool>) {
    let _ = ctrlc::set_handler(move || {
        running.store(false, std::sync::atomic::Ordering::Relaxed);
    });
}
