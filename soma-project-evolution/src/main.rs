// soma-project-evolution — end-to-end proof that mutation + selection produce
// adaptation on REAL routine execution through a real port (no human authoring
// the winner).
//
// Run: `cd soma-project-evolution && cargo run --release`
// Exit code: 0 iff every phase PASSes. Any failure exits 1 with a message.
//
// The unit of evolution is the routine. Fitness is the real outcome of
// executing the routine's skill through the reference filesystem port — every
// run yields a PortCallRecord whose `success` is read straight off the session
// trace. The heritable trait under selection is *which skill* a routine calls,
// which is exactly what the point-mutation operator perturbs.
//
// Environment controls fitness through one path that the harness flips:
//   target is a FILE → readfile✓  stat✓  readdir✗
//   target is a DIR  → readdir✓  stat✓  readfile✗
// so `stat` is the generalist, `readfile`/`readdir` are specialists.
//
//   Phase 1 — Variation:        a sustained-success seed breeds Mutated offspring.
//   Phase 2 — Selection-against: a routine that keeps failing is invalidated.
//   Phase 3 — Selection-for:     a succeeding routine's confidence climbs, capped.
//   Phase 4 — Adaptation:        flip the environment; the seed dies and a
//                                mutated descendant survives where the seed fails.
//
// The selection loop here mirrors the reactive monitor's feedback block
// (soma-next/src/runtime/world_state.rs) exactly — same decay/reinforce
// constants, same public mutation functions — driven as an explicit,
// deterministic generation loop instead of the wall-clock background thread.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::json;
use uuid::Uuid;

use soma_next::bootstrap::{bootstrap, Runtime};
use soma_next::config::SomaConfig;
use soma_next::memory::mutation::{self, BreedingPolicy, MutationConfig};
use soma_next::runtime::session::{SessionRuntime, StepResult};
use soma_next::types::belief::Binding;
use soma_next::types::common::Precondition;
use soma_next::types::goal::{GoalSource, GoalSourceType, GoalSpec, Objective, Priority};
use soma_next::types::routine::{CompiledStep, NextStep, Routine, RoutineOrigin};

const READFILE: &str = "soma.ports.reference.readfile";
const STAT: &str = "soma.ports.reference.stat";
const READDIR: &str = "soma.ports.reference.readdir";

// Same constants as the reactive monitor's feedback block (world_state.rs).
const DECAY: f64 = 0.7;
const INVALIDATE_BELOW: f64 = 0.3;
const MAX_FAILS: u32 = 3;
const REINFORCE: f64 = 1.15;
const CEILING: f64 = 0.95;

type Phase = fn() -> Result<String, String>;

fn main() {
    let phases: &[(&str, Phase)] = &[
        ("Phase 1: variation — seed breeds Mutated offspring", phase1_variation),
        ("Phase 2: selection-against — failing routine invalidated", phase2_selection_against),
        ("Phase 3: selection-for — confidence climbs, capped", phase3_selection_for),
        ("Phase 4: adaptation — seed dies, mutant survives env shift", phase4_adaptation),
        ("Phase 5: directed rescue — failing skill is replaced before death", phase5_directed_rescue),
    ];

    println!("SOMA evolution end-to-end proof");
    println!("  unit of evolution = routine; fitness = real port outcome\n");

    let mut any_failed = false;
    for (name, f) in phases {
        println!("--- {name} ---");
        match f() {
            Ok(detail) => println!("  PASS: {detail}\n"),
            Err(e) => {
                println!("  FAIL: {e}\n");
                any_failed = true;
            }
        }
    }

    println!("==================================================");
    if any_failed {
        println!("RESULT: at least one phase failed");
        println!("==================================================");
        std::process::exit(1);
    }
    println!("RESULT: ALL {} PHASES PASSED", phases.len());
    println!("==================================================");
}

// ---------------------------------------------------------------------------
// Shared scaffolding
// ---------------------------------------------------------------------------

/// Per-routine selection state, mirroring the reactive monitor's counters.
struct Evo {
    success_counts: HashMap<String, u32>,
    failure_counts: HashMap<String, u32>,
    breed_tick: u64,
    policy: BreedingPolicy,
}

impl Evo {
    fn new() -> Self {
        Self {
            success_counts: HashMap::new(),
            failure_counts: HashMap::new(),
            breed_tick: 0,
            policy: BreedingPolicy {
                breed_threshold: 3,
                confidence_breed_floor: 0.7,
                population_cap: 256,
                mutation: MutationConfig {
                    probation_confidence: 0.5,
                    max_offspring: 2,
                },
            },
        }
    }
}

fn reference_pack() -> String {
    std::env::var("SOMA_REFERENCE_PACK")
        .unwrap_or_else(|_| "../soma-next/packs/reference/manifest.json".to_string())
}

fn boot(data_dir: &Path) -> Result<Runtime, String> {
    let mut config = SomaConfig::default();
    config.soma.data_dir = data_dir.to_string_lossy().to_string();
    config.runtime.max_steps = 100;
    bootstrap(&config, &[reference_pack()]).map_err(|e| format!("bootstrap failed: {e}"))
}

fn make_goal() -> GoalSpec {
    GoalSpec {
        goal_id: Uuid::new_v4(),
        source: GoalSource {
            source_type: GoalSourceType::Internal,
            identity: Some("evolution".into()),
            session_id: None,
            peer_id: None,
        },
        objective: Objective {
            description: "evolution probe".into(),
            structured: None,
        },
        constraints: vec![],
        success_conditions: vec![],
        risk_budget: 1.0,
        latency_budget_ms: 60_000,
        resource_budget: 100.0,
        deadline: None,
        permissions_scope: vec!["read_only".into()],
        priority: Priority::Normal,
        max_steps: None,
        exploration: soma_next::types::goal::ExplorationStrategy::Greedy,
    }
}

/// A single-step routine that calls one reference skill. The skill reads its
/// `path` from the session belief binding the harness injects.
fn routine(id: &str, skill: &str, autonomous: bool, confidence: f64) -> Routine {
    Routine {
        routine_id: id.into(),
        namespace: "evolution".into(),
        origin: RoutineOrigin::PackAuthored,
        match_conditions: vec![Precondition {
            condition_type: "world_state".into(),
            expression: json!({ "probe.requested": true }),
            description: "fires when a probe is requested".into(),
        }],
        compiled_skill_path: vec![],
        compiled_steps: vec![CompiledStep::Skill {
            skill_id: skill.into(),
            on_success: NextStep::Complete,
            on_failure: NextStep::Abandon,
            conditions: vec![],
            input_overrides: Default::default(),
        }],
        guard_conditions: vec![],
        expected_cost: 0.1,
        expected_effect: vec![],
        confidence,
        autonomous,
        priority: 5,
        exclusive: false,
        policy_scope: None,
        version: 0,
        model_evidence: 0.0,
    }
}

fn skill_of(r: &Routine) -> Option<String> {
    match r.effective_steps().into_iter().next() {
        Some(CompiledStep::Skill { skill_id, .. }) => Some(skill_id),
        _ => None,
    }
}

/// Execute a routine through the real session controller, exactly as the
/// reactive monitor does (inject the routine's steps, follow the plan). Success
/// is the real port outcome of the routine's OWN skill, read off the trace —
/// so a deliberation fallback that runs a different skill can never be mistaken
/// for this routine succeeding.
fn run_routine(rt: &mut Runtime, r: &Routine, target: &str) -> bool {
    let skill_id = match skill_of(r) {
        Some(s) => s,
        None => return false,
    };
    let mut session = match rt.session_controller.create_session(make_goal()) {
        Ok(s) => s,
        Err(_) => return false,
    };
    session.working_memory.active_steps = Some(r.effective_steps());
    session.working_memory.plan_step = 0;
    session.working_memory.used_plan_following = true;
    session.belief.active_bindings.push(Binding {
        name: "path".into(),
        value: json!(target),
        source: "evolution_env".into(),
        confidence: 1.0,
    });
    for _ in 0..4 {
        match rt.session_controller.run_step(&mut session) {
            Ok(StepResult::Continue) => {}
            _ => break,
        }
    }
    session.trace.steps.iter().any(|s| {
        s.selected_skill == skill_id
            && !s.port_calls.is_empty()
            && s.port_calls.iter().all(|p| p.success)
    })
}

/// One selection tick for a single routine — the reactive monitor's feedback
/// block, line for line, using the same public mutation functions. Returns the
/// execution outcome.
fn tick(rt: &mut Runtime, ev: &mut Evo, routine_id: &str, target: &str) -> bool {
    let parent = {
        let store = rt.routine_store.lock().unwrap();
        match store.get(routine_id) {
            Some(r) => r.clone(),
            None => return false,
        }
    };
    let success = run_routine(rt, &parent, target);

    let mut store = rt.routine_store.lock().unwrap();
    if success {
        ev.failure_counts.remove(routine_id);
        let consecutive = {
            let c = ev.success_counts.entry(routine_id.to_string()).or_insert(0);
            *c += 1;
            *c
        };
        if let Some(parent) = store.get(routine_id).cloned() {
            // Reinforce on success — symmetric to the monitor's decay.
            let boosted = mutation::reinforced_confidence(parent.confidence, REINFORCE, CEILING);
            if boosted > parent.confidence {
                let mut u = parent.clone();
                u.confidence = boosted;
                let _ = store.register(u);
            }
            let population = store.list_all().iter().filter(|r| r.autonomous).count();
            if mutation::should_breed(&ev.policy, &parent, consecutive, population) {
                let alphabet = mutation::skill_alphabet(&store.list_all());
                ev.breed_tick += 1;
                let seed = mutation::seed_for(routine_id, ev.breed_tick);
                for child in mutation::mutate(&parent, &alphabet, &ev.policy.mutation, seed) {
                    // Admit only single-step offspring, so skill identity is the
                    // sole trait under selection in this environment. Multi-step
                    // variants exist but don't establish here.
                    if child.effective_steps().len() == 1 {
                        let _ = store.register(child);
                    }
                }
                ev.success_counts.remove(routine_id);
            }
        }
    } else {
        ev.success_counts.remove(routine_id);
        let count = {
            let c = ev.failure_counts.entry(routine_id.to_string()).or_insert(0);
            *c += 1;
            *c
        };
        if count >= MAX_FAILS
            && let Some(parent) = store.get(routine_id).cloned()
        {
            let nc = parent.confidence * DECAY;
            if nc < INVALIDATE_BELOW {
                // Directed rescue before death (mirrors world_state.rs): spawn
                // variants that replace the skill that failed. For these
                // single-step routines the failed skill is the routine's skill.
                if let Some(failed) = skill_of(&parent) {
                    let alphabet = mutation::skill_alphabet(&store.list_all());
                    ev.breed_tick += 1;
                    let seed = mutation::seed_for(routine_id, ev.breed_tick);
                    for child in
                        mutation::guided_mutate(&parent, &failed, &alphabet, &ev.policy.mutation, seed)
                    {
                        if child.effective_steps().len() == 1 {
                            let _ = store.register(child);
                        }
                    }
                }
                let _ = store.invalidate(routine_id);
                ev.failure_counts.remove(routine_id);
            } else {
                let mut u = parent;
                u.confidence = nc;
                let _ = store.register(u);
            }
        }
    }
    success
}

/// Tick every live autonomous routine once.
fn generation(rt: &mut Runtime, ev: &mut Evo, target: &str) {
    let ids: Vec<String> = {
        let store = rt.routine_store.lock().unwrap();
        store
            .list_all()
            .iter()
            .filter(|r| r.autonomous)
            .map(|r| r.routine_id.clone())
            .collect()
    };
    for id in ids {
        tick(rt, ev, &id, target);
    }
}

fn live_autonomous(rt: &Runtime) -> Vec<Routine> {
    let store = rt.routine_store.lock().unwrap();
    store
        .list_all()
        .iter()
        .filter(|r| r.autonomous)
        .map(|r| (*r).clone())
        .collect()
}

fn env_file(target: &Path) {
    let _ = fs::remove_dir_all(target);
    let _ = fs::remove_file(target);
    fs::write(target, b"payload").expect("write env file");
}

fn env_dir(target: &Path) {
    let _ = fs::remove_file(target);
    let _ = fs::remove_dir_all(target);
    fs::create_dir_all(target).expect("create env dir");
    fs::write(target.join("child.txt"), b"x").expect("write child");
}

// ---------------------------------------------------------------------------
// Phase 1 — Variation
// ---------------------------------------------------------------------------

fn phase1_variation() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = work.path().join("target");
    env_file(&target); // readfile succeeds here
    let target = target.to_string_lossy().to_string();

    let mut rt = boot(tmp.path())?;
    {
        let mut store = rt.routine_store.lock().unwrap();
        store
            .register(routine("evo.seed", READFILE, true, 0.9))
            .map_err(|e| e.to_string())?;
        // Library routines (not autonomous) enrich the mutation alphabet, just
        // as a real instance's many routines do. They never fire or breed.
        store.register(routine("lib.stat", STAT, false, 0.9)).ok();
        store.register(routine("lib.readdir", READDIR, false, 0.9)).ok();
    }

    let mut ev = Evo::new();
    // Confirm the seed actually succeeds before we expect it to breed.
    if !run_routine(&mut rt, &routine("evo.seed", READFILE, true, 0.9), &target) {
        return Err("seed readfile did not succeed on a file target".into());
    }
    for _ in 0..6 {
        generation(&mut rt, &mut ev, &target);
    }

    let mutants: Vec<Routine> = live_autonomous(&rt)
        .into_iter()
        .filter(|r| r.origin == RoutineOrigin::Mutated)
        .collect();
    if mutants.is_empty() {
        return Err("seed sustained success but produced no Mutated offspring".into());
    }
    let traced = mutants
        .iter()
        .any(|m| mutation::parent_of(&m.routine_id).is_some());
    if !traced {
        return Err("mutant offspring lack recoverable lineage".into());
    }
    Ok(format!(
        "seed bred {} Mutated offspring (e.g. {}), all with traceable lineage",
        mutants.len(),
        mutants[0].routine_id
    ))
}

// ---------------------------------------------------------------------------
// Phase 2 — Selection-against
// ---------------------------------------------------------------------------

fn phase2_selection_against() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = work.path().join("target");
    env_file(&target); // a FILE — so readdir always fails
    let target = target.to_string_lossy().to_string();

    let mut rt = boot(tmp.path())?;
    {
        let mut store = rt.routine_store.lock().unwrap();
        store
            .register(routine("evo.broken", READDIR, true, 0.9))
            .map_err(|e| e.to_string())?;
    }

    let mut ev = Evo::new();
    if run_routine(&mut rt, &routine("evo.broken", READDIR, true, 0.9), &target) {
        return Err("readdir unexpectedly succeeded on a file target".into());
    }
    // Decay multiplies 0.9 by 0.7 once per failing tick past the threshold:
    // 0.9 → 0.63 → 0.44 → 0.31 → 0.21 (< 0.3 ⇒ invalidated). ~6 failures.
    for _ in 0..10 {
        generation(&mut rt, &mut ev, &target);
        let alive = {
            let store = rt.routine_store.lock().unwrap();
            store.get("evo.broken").is_some()
        };
        if !alive {
            return Ok("routine that never succeeds was decayed and invalidated".into());
        }
    }
    Err("a never-succeeding routine survived 10 generations".into())
}

// ---------------------------------------------------------------------------
// Phase 3 — Selection-for
// ---------------------------------------------------------------------------

fn phase3_selection_for() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = work.path().join("target");
    env_file(&target);
    let target = target.to_string_lossy().to_string();

    let mut rt = boot(tmp.path())?;
    {
        let mut store = rt.routine_store.lock().unwrap();
        // Start below the ceiling so we can watch confidence climb.
        store
            .register(routine("evo.fit", READFILE, true, 0.7))
            .map_err(|e| e.to_string())?;
    }

    let mut ev = Evo::new();
    let mut last = 0.7_f64;
    let mut rose = false;
    for _ in 0..5 {
        generation(&mut rt, &mut ev, &target);
        let c = {
            let store = rt.routine_store.lock().unwrap();
            store.get("evo.fit").map(|r| r.confidence).unwrap_or(0.0)
        };
        if c > last + 1e-9 {
            rose = true;
        }
        if c > CEILING + 1e-9 {
            return Err(format!("confidence {c} exceeded ceiling {CEILING}"));
        }
        last = c;
    }
    if !rose {
        return Err("confidence never rose despite repeated success".into());
    }
    if (last - CEILING).abs() > 1e-9 {
        return Err(format!("confidence settled at {last}, expected ceiling {CEILING}"));
    }
    Ok(format!("confidence rose to the {CEILING} ceiling under repeated success"))
}

// ---------------------------------------------------------------------------
// Phase 4 — Adaptation (the headline)
// ---------------------------------------------------------------------------

fn phase4_adaptation() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target_path = work.path().join("target");
    let target = target_path.to_string_lossy().to_string();

    let mut rt = boot(tmp.path())?;
    {
        let mut store = rt.routine_store.lock().unwrap();
        // The seed is a readfile specialist — optimal while target is a file.
        store
            .register(routine("evo.seed", READFILE, true, 0.9))
            .map_err(|e| e.to_string())?;
        // Gene-pool vocabulary: stat (generalist) and readdir (dir specialist)
        // are available to mutation but no autonomous routine uses them yet.
        store.register(routine("lib.stat", STAT, false, 0.9)).ok();
        store.register(routine("lib.readdir", READDIR, false, 0.9)).ok();
    }
    let mut ev = Evo::new();

    // --- Environment A: target is a FILE. The seed thrives and diversifies. ---
    env_file(&target_path);
    for _ in 0..8 {
        generation(&mut rt, &mut ev, &target);
    }

    // A stat-using mutant (the generalist) must have emerged and survived A.
    let stat_mutant = live_autonomous(&rt).into_iter().find(|r| {
        r.origin == RoutineOrigin::Mutated && skill_of(r).as_deref() == Some(STAT)
    });
    let stat_mutant = match stat_mutant {
        Some(m) => m,
        None => return Err("no surviving stat-using mutant emerged in environment A".into()),
    };

    // Seed must still be alive at the end of A (readfile works on a file).
    {
        let store = rt.routine_store.lock().unwrap();
        if store.get("evo.seed").is_none() {
            return Err("seed died in environment A, where readfile should succeed".into());
        }
    }

    // --- Environment shift: target becomes a DIR. readfile now fails. ---
    env_dir(&target_path);

    // Sanity: in env B the seed's skill fails and the mutant's skill succeeds.
    if run_routine(&mut rt, &routine("evo.seed", READFILE, true, 0.9), &target) {
        return Err("readfile unexpectedly succeeded on a directory".into());
    }
    if !run_routine(&mut rt, &stat_mutant, &target) {
        return Err("stat mutant failed on a directory — env B misconfigured".into());
    }

    for _ in 0..8 {
        generation(&mut rt, &mut ev, &target);
    }

    // The seed must have died; a Mutated descendant must survive and succeed.
    let seed_dead = {
        let store = rt.routine_store.lock().unwrap();
        store.get("evo.seed").is_none()
    };
    if !seed_dead {
        return Err("seed survived environment B, where readfile always fails".into());
    }

    // `survivors` is owned, so executing each through a fresh session does not
    // conflict with the search. Find a Mutated survivor that succeeds in env B.
    let survivors = live_autonomous(&rt);
    let mut evolved: Option<Routine> = None;
    for r in &survivors {
        if r.origin == RoutineOrigin::Mutated && run_routine(&mut rt, r, &target) {
            evolved = Some(r.clone());
            break;
        }
    }
    let evolved = match evolved {
        Some(r) => r,
        None => return Err("no Mutated survivor succeeds in environment B".into()),
    };

    Ok(format!(
        "environment flipped file→dir: seed (readfile) was invalidated; \
         mutant {} (skill {}) survives and succeeds where the seed fails — \
         adaptation with no human authoring",
        evolved.routine_id,
        skill_of(&evolved).unwrap_or_default()
    ))
}

// ---------------------------------------------------------------------------
// Phase 5 — Directed rescue
// ---------------------------------------------------------------------------

fn phase5_directed_rescue() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = work.path().join("target");
    env_file(&target); // a FILE — so readdir always fails, stat/readfile succeed
    let target = target.to_string_lossy().to_string();

    let mut rt = boot(tmp.path())?;
    {
        let mut store = rt.routine_store.lock().unwrap();
        // The seed uses a skill that cannot work in this environment.
        store
            .register(routine("evo.readdir", READDIR, true, 0.9))
            .map_err(|e| e.to_string())?;
        // Alphabet vocabulary: skills that *do* work on a file.
        store.register(routine("lib.stat", STAT, false, 0.9)).ok();
        store.register(routine("lib.readfile", READFILE, false, 0.9)).ok();
    }

    let mut ev = Evo::new();
    if run_routine(&mut rt, &routine("evo.readdir", READDIR, true, 0.9), &target) {
        return Err("readdir unexpectedly succeeded on a file".into());
    }
    // Let the seed fail, decay, hit the rescue point, then let the rescue
    // variants run and establish.
    for _ in 0..12 {
        generation(&mut rt, &mut ev, &target);
    }

    // The seed must be gone.
    {
        let store = rt.routine_store.lock().unwrap();
        if store.get("evo.readdir").is_some() {
            return Err("the always-failing seed was never invalidated".into());
        }
    }

    // A directed rescue variant must exist: Mutated, no longer using readdir,
    // and actually succeeding in this environment.
    let survivors = live_autonomous(&rt);
    let mut rescued: Option<Routine> = None;
    for r in &survivors {
        let is_rescue = r.origin == RoutineOrigin::Mutated
            && mutation::parent_of(&r.routine_id) == Some("evo.readdir")
            && skill_of(r).as_deref() != Some(READDIR);
        if is_rescue && run_routine(&mut rt, r, &target) {
            rescued = Some(r.clone());
            break;
        }
    }
    match rescued {
        Some(r) => Ok(format!(
            "seed (readdir, doomed on a file) was invalidated; directed rescue bred {} \
             (skill {} — the failed skill replaced) which succeeds in its place",
            r.routine_id,
            skill_of(&r).unwrap_or_default()
        )),
        None => Err("no directed rescue variant survived the failing seed".into()),
    }
}
