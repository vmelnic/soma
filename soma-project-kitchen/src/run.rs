use std::path::{Path, PathBuf};

use soma_next::bootstrap::bootstrap;
use soma_next::config::SomaConfig;
use soma_next::memory::embedder::{GoalEmbedder, HashEmbedder};

use kitchen::session::run_scenario;
use kitchen::world::ScenarioSpec;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: run-kitchen <scenario.json|scenarios-dir/> [--output <dir>]");
        std::process::exit(1);
    }

    let input = PathBuf::from(&args[0]);
    let output_dir = args.iter()
        .position(|a| a == "--output")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output"));

    std::fs::create_dir_all(&output_dir).unwrap();

    let scenarios = if input.is_dir() {
        ScenarioSpec::load_dir(&input).unwrap_or_else(|e| {
            eprintln!("Failed to load scenarios: {e}");
            std::process::exit(1);
        })
    } else {
        vec![ScenarioSpec::load(&input).unwrap_or_else(|e| {
            eprintln!("Failed to load scenario: {e}");
            std::process::exit(1);
        })]
    };

    if scenarios.is_empty() {
        eprintln!("No scenario files found");
        std::process::exit(1);
    }

    println!("soma-project-kitchen: running {} scenario(s)\n", scenarios.len());

    let mut runtime = bootstrap_runtime();

    let routine_count = runtime.routine_store.lock().unwrap().list_all().len();
    let episode_count = runtime.episode_store.lock().unwrap().count();
    if routine_count > 0 || episode_count > 0 {
        println!("  Loaded {episode_count} episode(s), {routine_count} routine(s) from disk\n");
    }

    let mut solved_count = 0;
    let mut total_count = 0;
    let mut stored_episodes = 0;

    for scenario in &scenarios {
        total_count += 1;
        print!("  {:<30}", scenario.name);

        match run_scenario(&mut runtime, scenario) {
            Ok(result) => {
                let status = if result.trace.solved { "SOLVED" } else { "FAILED" };
                let plan = if result.trace.plan_following { " [PLAN]" } else { "" };
                println!("  steps={}  {status}{plan}", result.trace.step_count);
                println!("    skills: {:?}", result.trace.skills);

                if result.trace.solved { solved_count += 1; }

                if runtime.episode_store.lock().unwrap().store(result.episode).is_ok() {
                    stored_episodes += 1;
                }

                let trace_path = output_dir.join(format!("{}.trace.json", scenario.name));
                match write_trace(&result.trace, &trace_path) {
                    Ok(()) => println!("    trace: {}", trace_path.display()),
                    Err(e) => println!("    trace: FAILED ({e})"),
                }
                println!();
            }
            Err(e) => {
                println!("  ERROR: {e}\n");
            }
        }
    }

    if stored_episodes > 0 {
        let embedder = HashEmbedder::new();
        let mut fingerprints: Vec<String> = scenarios.iter()
            .map(|s| s.goal_fingerprint())
            .collect();
        fingerprints.sort();
        fingerprints.dedup();
        for fp in &fingerprints {
            soma_next::interfaces::cli::attempt_learning(
                &runtime.episode_store,
                &runtime.schema_store,
                &runtime.routine_store,
                fp,
                &embedder as &dyn GoalEmbedder,
            );
        }
        let routine_count = runtime.routine_store.lock().unwrap().list_all().len();
        println!("  Learning: {routine_count} routine(s) after {stored_episodes} new episode(s)\n");
    }

    println!("───────────────────────────��────────────────────");
    println!("  {solved_count}/{total_count} solved");
    println!("  output: {}", output_dir.display());
    println!("───────────────────────────────────────────��────");

    if solved_count < total_count {
        std::process::exit(1);
    }
}

fn bootstrap_runtime() -> soma_next::bootstrap::Runtime {
    let mut config = SomaConfig::default();
    let data_dir = std::env::current_dir().unwrap_or_default().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    config.soma.data_dir = data_dir.to_string_lossy().to_string();
    config.runtime.max_steps = 200;

    let pack_path = std::env::var("SOMA_KITCHEN_PACK")
        .unwrap_or_else(|_| "packs/kitchen/manifest.json".to_string());

    let runtime = bootstrap(&config, &[pack_path])
        .unwrap_or_else(|e| {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        });

    for pack in &runtime.pack_specs {
        for schema in &pack.schemas {
            let _ = runtime.schema_store.lock().unwrap().register(schema.clone());
        }
        for routine in &pack.routines {
            let _ = runtime.routine_store.lock().unwrap().register(routine.clone());
        }
    }

    runtime
}

fn write_trace(
    trace: &kitchen::session::ScenarioTrace,
    path: &Path,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(trace).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))
}
