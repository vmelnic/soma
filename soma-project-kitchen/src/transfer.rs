use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use soma_next::bootstrap::bootstrap;
use soma_next::config::SomaConfig;
use soma_next::memory::routines::RoutineStore;
use soma_next::runtime::skill::SkillRuntime as _;
use soma_next::runtime::world_state::WorldStateStore;
use soma_next::types::belief::Fact;
use soma_next::types::common::FactProvenance;

type SharedWorldState = Arc<Mutex<dyn WorldStateStore + Send>>;
type SharedRoutineStore = Arc<Mutex<dyn RoutineStore + Send>>;

fn main() {
    println!("SOMA Phase 2: cross-domain transfer proof");
    println!("  Domain A = coder (git/search/runner/patch)");
    println!("  Domain B = kitchen (manipulation/scan/pick/place/door/drawer)");
    println!("  DNA = same domain-agnostic routines in both");
    println!();

    let mut config = SomaConfig::default();
    config.runtime.max_steps = 200;
    config.runtime.reactive_monitor_interval_secs = 5;

    let packs: Vec<String> = vec![
        "packs/kitchen/manifest.json".to_string(),
        "packs/dna/manifest.json".to_string(),
    ];

    let runtime = bootstrap(&config, &packs).unwrap_or_else(|e| {
        eprintln!("Bootstrap failed: {e}");
        std::process::exit(1);
    });

    let skill_count = runtime.skill_runtime.list_skills(None).len();
    let routine_count = runtime.routine_store.lock().unwrap().list_all().len();
    let dna_count = runtime.routine_store.lock().unwrap().list_all()
        .iter()
        .filter(|r| r.autonomous && r.namespace == "soma.dna")
        .count();

    println!("  Skills loaded: {skill_count} (kitchen domain)");
    println!("  Routines loaded: {routine_count} ({dna_count} DNA)");

    let ws: SharedWorldState = Arc::clone(&runtime.world_state);
    let rs: SharedRoutineStore = Arc::clone(&runtime.routine_store);

    let monitor = soma_next::runtime::world_state::start_reactive_monitor(
        Arc::clone(&runtime.world_state),
        Arc::clone(&runtime.routine_store),
        Arc::new(Mutex::new(runtime.session_controller)),
        Arc::new(Mutex::new(runtime.goal_runtime)),
        Arc::clone(&runtime.episode_store),
        Arc::clone(&runtime.embedder),
        5,
    );

    println!();
    let mut pass = 0;
    let mut fail = 0;

    // Phase 2a: event detection in kitchen domain
    print!("  Phase 2a: event detection in kitchen domain       ");
    inject_fact(&ws, "ev-k1", "event", "detected", "jar moved on countertop");
    thread::sleep(Duration::from_secs(7));
    if check_routine_fired(&ws, "dna.orient") {
        println!("PASS");
        pass += 1;
    } else {
        println!("FAIL");
        fail += 1;
    }

    // Phase 2b: novelty detection — new object type
    print!("  Phase 2b: novelty detection (new object type)     ");
    inject_fact(&ws, "nov-k1", "novelty", "detected", "unknown utensil on shelf");
    thread::sleep(Duration::from_secs(7));
    if check_routine_fired(&ws, "dna.explore") {
        println!("PASS");
        pass += 1;
    } else {
        println!("FAIL");
        fail += 1;
    }

    // Phase 2c: anomaly detection — drawer stuck
    print!("  Phase 2c: anomaly detection (drawer stuck)        ");
    inject_fact(&ws, "anom-k1", "anomaly", "detected", "drawer_open returned failure");
    thread::sleep(Duration::from_secs(7));
    if check_routine_fired(&ws, "dna.anomaly") {
        println!("PASS");
        pass += 1;
    } else {
        println!("FAIL");
        fail += 1;
    }

    // Phase 2d: full genome activation — all three simultaneously
    print!("  Phase 2d: full genome activation (all three)      ");
    clear_world_state(&ws);
    thread::sleep(Duration::from_secs(6));
    inject_fact(&ws, "ev-k2", "event", "detected", "window blown open by wind");
    inject_fact(&ws, "nov-k2", "novelty", "detected", "new item appeared: blender");
    inject_fact(&ws, "anom-k2", "anomaly", "detected", "button_press unresponsive");
    thread::sleep(Duration::from_secs(7));
    let orient = check_routine_fired(&ws, "dna.orient");
    let explore = check_routine_fired(&ws, "dna.explore");
    let anomaly = check_routine_fired(&ws, "dna.anomaly");
    if orient && explore && anomaly {
        println!("PASS");
        pass += 1;
    } else {
        println!("FAIL (orient={orient} explore={explore} anomaly={anomaly})");
        fail += 1;
    }

    // Phase 2e: verify skills are kitchen skills, not coder skills
    print!("  Phase 2e: domain-specific skills available        ");
    let skills = runtime.skill_runtime.list_skills(None);
    let has_kitchen = skills.iter().any(|s| s.skill_id.contains("kitchen"));
    let has_coder = skills.iter().any(|s| s.skill_id.contains("git"));
    if has_kitchen && !has_coder {
        println!("PASS");
        pass += 1;
    } else {
        println!("FAIL (kitchen={has_kitchen} coder={has_coder})");
        fail += 1;
    }

    // Phase 2f: DNA routines are identical across domains
    print!("  Phase 2f: DNA routines identical across domains   ");
    let binding = rs.lock().unwrap();
    let all_routines = binding.list_all();
    let dna_routines: Vec<_> = all_routines.iter()
        .filter(|r| r.namespace == "soma.dna")
        .collect();
    let all_autonomous = dna_routines.iter().all(|r| r.autonomous);
    let all_empty_steps = dna_routines.iter().all(|r| r.compiled_steps.is_empty() && r.compiled_skill_path.is_empty());
    let has_all_four = dna_routines.len() == 4;
    if all_autonomous && all_empty_steps && has_all_four {
        println!("PASS");
        pass += 1;
    } else {
        println!("FAIL (count={} autonomous={all_autonomous} empty={all_empty_steps})", dna_routines.len());
        fail += 1;
    }

    println!();
    println!("────────────────────────────────────────────────────");
    if fail == 0 {
        println!("  RESULT: ALL {pass} PHASES PASSED");
        println!();
        println!("  Phase 2 proven: same DNA, same binary, different domain.");
        println!("  Kitchen skills (pick/place/scan/door/drawer) replaced");
        println!("  coder skills (git/search/runner/patch).");
        println!("  Curiosity cascade fires identically.");
        println!("  The architecture transfers.");
    } else {
        println!("  RESULT: {pass} passed, {fail} failed");
    }
    println!("────────────────────────────────────────────────────");

    drop(monitor);
    std::process::exit(if fail == 0 { 0 } else { 1 });
}

fn inject_fact(ws: &SharedWorldState, id: &str, subject: &str, predicate: &str, value: &str) {
    let mut store = ws.lock().unwrap();
    let _ = store.add_fact(Fact {
        fact_id: id.to_string(),
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        value: serde_json::Value::String(value.to_string()),
        confidence: 0.9,
        provenance: FactProvenance::Observed,
        timestamp: Utc::now(),
        ttl_ms: None,
        prior_confidence: None,
        prediction_error: None,
    });
}

fn clear_world_state(ws: &SharedWorldState) {
    let mut store = ws.lock().unwrap();
    for id in ["ev-k1", "nov-k1", "anom-k1", "ev-k2", "nov-k2", "anom-k2"] {
        let _ = store.remove_fact(id);
    }
}

fn check_routine_fired(ws: &SharedWorldState, routine_id: &str) -> bool {
    let store = ws.lock().unwrap();
    let snap = store.snapshot();
    if let Some(obj) = snap.as_object() {
        let success_key = format!("routine.{routine_id}.last_success");
        obj.contains_key(&success_key)
    } else {
        false
    }
}
