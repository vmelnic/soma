// Real-world learning proof.
//
// Runs the ACTUAL autonomous loop on REAL filesystem data.
// No synthetic episodes. No planted patterns. Whatever happens, happens.
//
// The question: does PrefixSpan find structure in real execution traces?
// Does BMR compile a routine? Does the routine fire on a novel instance?

use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use soma_next::bootstrap::bootstrap;
use soma_next::config::SomaConfig;
use soma_next::memory::embedder::{GoalEmbedder, HashEmbedder};
use soma_next::runtime::session::{SessionRuntime, StepResult};
use soma_next::types::belief::Binding;
use soma_next::types::goal::{
    ExplorationStrategy, GoalSource, GoalSourceType, GoalSpec, Objective, Priority,
};

const GOAL_FINGERPRINT: &str = "catalog_directory";

pub fn run_real_proof() -> Result<String, String> {
    let config = SomaConfig::default();
    let pack_path = std::env::var("SOMA_REFERENCE_PACK")
        .unwrap_or_else(|_| "../soma-next/packs/reference/manifest.json".to_string());

    let mut runtime = bootstrap(&config, &[pack_path])
        .map_err(|e| format!("bootstrap: {e}"))?;

    let embedder = Arc::new(HashEmbedder::new());

    // Real directories that exist on macOS and have varied contents.
    let directories = [
        "/tmp",
        "/var",
        "/usr",
        "/usr/local",
        "/usr/bin",
        "/etc",
        "/var/log",
        "/usr/lib",
        "/usr/share",
        "/private/tmp",
    ];

    println!("  Running {} goals through the REAL autonomous loop", directories.len());
    println!("  Goal fingerprint: {GOAL_FINGERPRINT}");
    println!("  No synthetic episodes. No planted patterns.\n");

    // ─── Phase 1: Run goals, collect real episodes ────────────────────────

    println!("  Phase A: Real execution (10 goals)");
    let mut total_steps = 0usize;
    let mut total_successes = 0usize;
    let mut skill_histogram: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (i, dir) in directories.iter().enumerate() {
        let goal = GoalSpec {
            goal_id: Uuid::new_v4(),
            source: GoalSource {
                source_type: GoalSourceType::Internal,
                identity: Some("real-proof".to_string()),
                session_id: None,
                peer_id: None,
            },
            objective: Objective {
                description: GOAL_FINGERPRINT.to_string(),
                structured: Some(json!({ "path": dir })),
            },
            constraints: Vec::new(),
            success_conditions: Vec::new(),
            risk_budget: 1.0,
            latency_budget_ms: 30_000,
            resource_budget: 1.0,
            deadline: None,
            permissions_scope: vec!["read_only".to_string()],
            priority: Priority::Normal,
            max_steps: Some(6),
            exploration: ExplorationStrategy::Greedy,
        };

        let mut session = runtime
            .session_controller
            .create_session(goal)
            .map_err(|e| format!("create_session: {e}"))?;

        session.belief.active_bindings.push(Binding {
            name: "path".to_string(),
            value: json!(dir),
            source: "goal_structured".to_string(),
            confidence: 1.0,
        });

        // Run the real control loop.
        for _ in 0..10 {
            match runtime.session_controller.run_step(&mut session) {
                Ok(StepResult::Continue) => continue,
                Ok(_) => break,
                Err(_) => break,
            }
        }

        // Extract what actually happened.
        let skills: Vec<&str> = session.trace.steps.iter()
            .map(|s| s.selected_skill.as_str())
            .collect();
        let successes: Vec<bool> = session.trace.steps.iter()
            .map(|s| s.port_calls.iter().all(|pc| pc.success))
            .collect();
        let step_count = session.trace.steps.len();

        total_steps += step_count;
        total_successes += successes.iter().filter(|&&s| s).count();
        for skill in &skills {
            *skill_histogram.entry(skill.to_string()).or_insert(0) += 1;
        }

        println!("    goal {i:2}: dir={dir:<15} steps={step_count} skills={skills:?} success={successes:?}");

        // Store episode via the real pipeline.
        let episode = soma_next::interfaces::cli::build_episode_from_session(
            &session,
            Some(&*embedder as &dyn GoalEmbedder),
        );
        {
            let mut es = runtime.episode_store.lock().unwrap();
            if let Err(e) = es.store(episode) {
                println!("    (episode store error: {e})");
            }
        }
    }

    println!("\n  Skill histogram from real execution:");
    let mut sorted_skills: Vec<_> = skill_histogram.iter().collect();
    sorted_skills.sort_by(|a, b| b.1.cmp(a.1));
    for (skill, count) in &sorted_skills {
        println!("    {skill}: {count}");
    }
    println!("  Total steps: {total_steps}, total successes: {total_successes}");

    // ─── Phase 2: Trigger learning pipeline on real episodes ──────────────

    println!("\n  Phase B: Learning pipeline on real episodes");

    soma_next::interfaces::cli::attempt_learning(
        &runtime.episode_store,
        &runtime.schema_store,
        &runtime.routine_store,
        GOAL_FINGERPRINT,
        &*embedder,
    );

    // Check what was learned.
    let schemas: Vec<soma_next::types::schema::Schema> = {
        let ss = runtime.schema_store.lock().unwrap();
        ss.list_all().into_iter().cloned().collect()
    };
    let routines: Vec<soma_next::types::routine::Routine> = {
        let rs = runtime.routine_store.lock().unwrap();
        rs.list_all().into_iter().cloned().collect()
    };

    println!("  Schemas induced: {}", schemas.len());
    for s in &schemas {
        println!("    schema: {} skills={:?} confidence={:.3}",
            s.schema_id, s.candidate_skill_ordering, s.confidence);
    }
    println!("  Routines compiled: {}", routines.len());
    for r in &routines {
        println!("    routine: {} path={:?} confidence={:.3} model_evidence={:.4}",
            r.routine_id, r.compiled_skill_path, r.confidence, r.model_evidence);
    }

    if routines.is_empty() && schemas.is_empty() {
        println!("\n  Phase C: SKIPPED (nothing learned)");
        return Ok(format!(
            "HONEST RESULT: {total_steps} real steps, {} unique skills, \
             {} schemas, {} routines. The learning pipeline found NO structure \
             in real execution traces. This is the gap.",
            skill_histogram.len(), schemas.len(), routines.len()
        ));
    }

    // ─── Phase 3: Test if compiled routines fire on novel instance ────────

    println!("\n  Phase C: Novel instance test");

    let novel_dirs = ["/var/tmp", "/usr/local/bin", "/private/var"];
    let mut routine_fired = 0usize;

    for dir in &novel_dirs {
        let goal = GoalSpec {
            goal_id: Uuid::new_v4(),
            source: GoalSource {
                source_type: GoalSourceType::Internal,
                identity: Some("real-proof-novel".to_string()),
                session_id: None,
                peer_id: None,
            },
            objective: Objective {
                description: GOAL_FINGERPRINT.to_string(),
                structured: Some(json!({ "path": dir })),
            },
            constraints: Vec::new(),
            success_conditions: Vec::new(),
            risk_budget: 1.0,
            latency_budget_ms: 30_000,
            resource_budget: 1.0,
            deadline: None,
            permissions_scope: vec!["read_only".to_string()],
            priority: Priority::Normal,
            max_steps: Some(6),
            exploration: ExplorationStrategy::Greedy,
        };

        let mut session = runtime
            .session_controller
            .create_session(goal)
            .map_err(|e| format!("create_session novel: {e}"))?;

        session.belief.active_bindings.push(Binding {
            name: "path".to_string(),
            value: json!(dir),
            source: "goal_structured".to_string(),
            confidence: 1.0,
        });

        for _ in 0..10 {
            match runtime.session_controller.run_step(&mut session) {
                Ok(StepResult::Continue) => continue,
                Ok(_) => break,
                Err(_) => break,
            }
        }

        let used_plan = session.working_memory.used_plan_following;
        let skills: Vec<&str> = session.trace.steps.iter()
            .map(|s| s.selected_skill.as_str())
            .collect();

        if used_plan {
            routine_fired += 1;
        }

        println!("    novel {dir:<20} plan_following={used_plan} steps={} skills={skills:?}",
            skills.len());
    }

    Ok(format!(
        "HONEST RESULT: {total_steps} real steps across {} dirs, \
         {} unique skills observed, {} schemas induced, {} routines compiled, \
         routine fired on {}/{} novel instances.",
        directories.len(),
        skill_histogram.len(),
        schemas.len(),
        routines.len(),
        routine_fired,
        novel_dirs.len()
    ))
}
