mod mind;
mod plugin;
mod protocol;
mod memory;
mod proprioception;

use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;

use mind::{MindEngine, ProgramStep, STOP_ID};
use mind::onnx_engine::OnnxMindEngine;
use plugin::builtin::PosixPlugin;
use plugin::manager::PluginManager;

#[derive(Parser)]
#[command(name = "soma", about = "SOMA: Neural mind drives hardware directly")]
struct Cli {
    /// Model directory (encoder.onnx, decoder.onnx, tokenizer.json, meta.json)
    #[arg(long, default_value = "models")]
    model: PathBuf,

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

fn run_intent(mind: &dyn MindEngine, plugins: &PluginManager, text: &str) {
    match mind.infer(text) {
        Ok(program) => {
            let real: Vec<&ProgramStep> = program.steps.iter()
                .filter(|s| s.conv_id != STOP_ID)
                .collect();
            println!("\n  [Mind] Program ({} steps, {:.0}%):", real.len(), program.confidence * 100.0);
            for (i, step) in program.steps.iter().enumerate() {
                println!("    {}", step.format(i, &mind.meta().catalog));
                if step.conv_id == STOP_ID { break; }
            }
            println!();

            let result = plugins.execute_program(&program.steps);
            for entry in &result.trace {
                if entry.op != "STOP" && entry.op != "EMIT" && !entry.summary.is_empty() {
                    println!("    [{}] {} ... {}", entry.step, entry.op, entry.summary);
                }
            }
            display_result(&result, &mind.meta().catalog);
        }
        Err(e) => {
            println!("  [Mind] Error: {}", e);
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("soma=info".parse()?)
        )
        .init();

    let cli = Cli::parse();

    // --- Boot sequence (Spec Section 11) ---

    // Step 3: Load Mind Engine
    let mind = OnnxMindEngine::load(&cli.model)?;
    let mind_info = mind.info();

    // Step 4: Load Plugins
    let mut plugins = PluginManager::new();
    plugins.register(Box::new(PosixPlugin::new()));
    let total_conv = plugins.conventions().len();

    // Step 7: Ready
    eprintln!("============================================================");
    eprintln!("  SOMA v0.1.0 -- Rust Runtime");
    eprintln!("  Neural mind drives libc directly. Single binary.");
    eprintln!("============================================================");
    eprintln!("  Mind:     {} ({}conv)", mind_info.backend, mind_info.conventions_known);
    eprintln!("  Plugins:  posix ({} conventions)", total_conv);
    eprintln!("  Model:    {}", cli.model.display());
    eprintln!("============================================================");

    if let Some(intent) = &cli.intent {
        run_intent(&mind, &plugins, intent);
        return Ok(());
    }

    // REPL (Spec Section 18.3)
    eprintln!("  Type intent. :status :inspect  quit");
    eprintln!();

    // Ctrl+C handler for graceful shutdown (Spec Section 16)
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();
    ctrlc_handler(r);

    loop {
        if !running.load(std::sync::atomic::Ordering::Relaxed) {
            println!("\n  SOMA shutting down (SIGINT).");
            break;
        }
        print!("intent> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 { break; }
        let text = input.trim();
        if text.is_empty() { continue; }
        if text == "quit" || text == "exit" || text == "q" {
            println!("\n  SOMA shutting down.");
            break;
        }

        // Debug REPL commands (Spec Section 18.3)
        if text == ":status" {
            let info = mind.info();
            println!("\n  [Proprioception]");
            println!("    Mind:        {}", info.backend);
            println!("    Conventions: {}", info.conventions_known);
            println!("    Max steps:   {}", info.max_steps);
            println!("    LoRA:        {} layers, magnitude {:.6}", info.lora_layers, info.lora_magnitude);
            println!("    Plugins:     {} loaded", plugins.conventions().len());
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

        run_intent(&mind, &plugins, text);
        println!();
    }

    Ok(())
}

fn ctrlc_handler(running: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let _ = ctrlc::set_handler(move || {
        running.store(false, std::sync::atomic::Ordering::Relaxed);
    });
}
