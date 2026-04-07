mod mind;
mod plugin;
mod protocol;
mod memory;
mod proprioception;

use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;

use mind::engine::{MindEngine, ProgramStep, EMIT_ID, STOP_ID};
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

fn display_result(result: &plugin::manager::ProgramResult, catalog: &[mind::engine::CatalogEntry]) {
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

fn run_intent(mind: &MindEngine, plugins: &PluginManager, text: &str) {
    match mind.predict(text) {
        Ok((steps, confidence)) => {
            let real: Vec<&ProgramStep> = steps.iter()
                .filter(|s| s.conv_id != STOP_ID)
                .collect();
            println!("\n  [Mind] Program ({} steps, {:.0}%):", real.len(), confidence * 100.0);
            for (i, step) in steps.iter().enumerate() {
                println!("    {}", step.format(i, &mind.meta.catalog));
                if step.conv_id == STOP_ID { break; }
            }
            println!();

            // Execute with trace
            let result = plugins.execute_program(&steps);
            for entry in &result.trace {
                if entry.op != "STOP" && entry.op != "EMIT" && !entry.summary.is_empty() {
                    println!("    [{}] {} ... {}", entry.step, entry.op, entry.summary);
                }
            }
            display_result(&result, &mind.meta.catalog);
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

    // Load mind
    let mind = MindEngine::load(&cli.model)?;

    // Register plugins
    let mut plugins = PluginManager::new();
    plugins.register(Box::new(PosixPlugin::new()));

    let total_conv = plugins.conventions().len();
    eprintln!("============================================================");
    eprintln!("  SOMA v0.1.0 — Rust Runtime");
    eprintln!("  Neural mind drives libc directly. Single binary.");
    eprintln!("============================================================");
    eprintln!("  Mind:     {} conventions, ONNX inference", mind.meta.num_conventions);
    eprintln!("  Plugins:  posix ({} conventions)", total_conv);
    eprintln!("  Model:    {}", cli.model.display());
    eprintln!("============================================================");

    if let Some(intent) = &cli.intent {
        run_intent(&mind, &plugins, intent);
        return Ok(());
    }

    // REPL
    eprintln!("  Type intent. 'quit' to exit.\n");
    loop {
        print!("intent> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        let text = input.trim();
        if text.is_empty() { continue; }
        if text == "quit" || text == "exit" || text == "q" {
            println!("\nSOMA shutting down.");
            break;
        }
        if text == "help" || text == "?" {
            println!("\n  [Proprioception]");
            for conv in plugins.conventions() {
                println!("    [{:2}] {} — {}", conv.id, conv.name, conv.description);
            }
            println!();
            continue;
        }

        run_intent(&mind, &plugins, text);
        println!();
    }

    Ok(())
}
