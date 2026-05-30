// soma-project-swarm — end-to-end proof of horizontal gene transfer: a routine
// evolved on one node propagates over the real transport to peers and runs
// there, an instinct none of them started with and no human wrote.
//
// Run: `cd soma-project-swarm && cargo run --release`
// Exit code: 0 iff every phase PASSes. Any failure exits 1 with a message.
//
// Transfer goes over real TCP between in-process nodes: a `TcpRemoteExecutor`
// client sends `TransferRoutine`; a `LocalDispatchHandler` listener (the
// runtime's real receive path) registers it as `RoutineOrigin::PeerTransferred`.
// Phase 2/3 use the production `RemoteRoutineBroadcaster` — the same type the
// reactive monitor's breeding loop calls.
//
//   Phase 1 — Wire:       a routine sent over TCP lands in the peer's store
//                         as PeerTransferred.
//   Phase 2 — Gene flow:  a mutated `stat` routine evolved + validated on node A
//                         is broadcast to node B, which runs it successfully.
//   Phase 3 — Swarm:      one broadcast reaches two peers (B and C); both run it.

use std::collections::HashMap;
use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use soma_next::bootstrap::{bootstrap, Runtime};
use soma_next::config::SomaConfig;
use soma_next::distributed::transport::{
    start_listener_background, IncomingHandler, LocalDispatchHandler, PeerAddressMap,
    TcpRemoteExecutor,
};
use soma_next::memory::mutation::{self, MutationConfig};
use soma_next::runtime::remote::{RemoteExecutor, RemoteRoutineBroadcaster};
use soma_next::runtime::session::{SessionRuntime, StepResult};
use soma_next::runtime::world_state::RoutineBroadcaster;
use soma_next::types::belief::Binding;
use soma_next::types::common::Precondition;
use soma_next::types::goal::{GoalSource, GoalSourceType, GoalSpec, Objective, Priority};
use soma_next::types::peer::RoutineTransfer;
use soma_next::types::routine::{CompiledStep, NextStep, Routine, RoutineOrigin};

const READFILE: &str = "soma.ports.reference.readfile";
const STAT: &str = "soma.ports.reference.stat";
const READDIR: &str = "soma.ports.reference.readdir";

type Phase = fn() -> Result<String, String>;

fn main() {
    let phases: &[(&str, Phase)] = &[
        ("Phase 1: wire — transferred routine lands as PeerTransferred", phase1_wire),
        ("Phase 2: gene flow — evolved routine runs on a peer", phase2_gene_flow),
        ("Phase 3: swarm — one broadcast reaches two peers", phase3_swarm),
    ];

    println!("SOMA swarm end-to-end proof");
    println!("  horizontal gene transfer over real TCP\n");

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
// Scaffolding
// ---------------------------------------------------------------------------

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
            identity: Some("swarm".into()),
            session_id: None,
            peer_id: None,
        },
        objective: Objective { description: "swarm probe".into(), structured: None },
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

fn routine(id: &str, skill: &str) -> Routine {
    Routine {
        routine_id: id.into(),
        namespace: "swarm".into(),
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
        confidence: 0.9,
        autonomous: true,
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

/// Execute a routine through the real session controller; success is the real
/// port outcome of the routine's own skill, read off the trace.
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
        source: "swarm_env".into(),
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

fn env_file(target: &Path) {
    let _ = fs::remove_dir_all(target);
    let _ = fs::remove_file(target);
    fs::write(target, b"payload").expect("write env file");
}

/// An OS-assigned free localhost address. Small bind/drop race, same trick the
/// runtime's own transport tests use.
fn free_addr() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let addr = l.local_addr().expect("local addr");
    drop(l);
    addr
}

/// A booted peer: its node handle, routine store, and listening address.
type PeerNode = (
    Arc<Mutex<Runtime>>,
    Arc<Mutex<dyn soma_next::memory::routines::RoutineStore + Send>>,
    SocketAddr,
);

/// Boot a peer node, start the runtime's real TCP receive path on a fresh
/// address, and return the node handle, its routine store, and its address.
/// The returned `JoinHandle` is leaked into the background (detached listener).
fn start_peer(data_dir: &Path) -> Result<PeerNode, String> {
    let node = boot(data_dir)?;
    let routines = Arc::clone(&node.routine_store);
    let schemas = Arc::clone(&node.schema_store);
    let node = Arc::new(Mutex::new(node));

    let handler: Arc<dyn IncomingHandler> = Arc::new(LocalDispatchHandler::with_stores(
        Arc::clone(&node),
        schemas,
        Arc::clone(&routines),
    ));
    let addr = free_addr();
    let _listener = start_listener_background(addr, handler);
    thread::sleep(Duration::from_millis(200)); // let the listener bind
    Ok((node, routines, addr))
}

fn peer_map(entries: &[(&str, SocketAddr)]) -> PeerAddressMap {
    let map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
    {
        let mut g = map.lock().unwrap();
        for (id, addr) in entries {
            g.insert((*id).to_string(), *addr);
        }
    }
    map
}

/// A genuine evolved gene: apply the real mutation operator to a `readfile`
/// seed until a `stat`-using offspring (origin Mutated) appears.
fn evolve_stat_mutant() -> Result<Routine, String> {
    let seed = routine("evo.seed", READFILE);
    let alphabet: Vec<String> = [READFILE, STAT, READDIR].iter().map(|s| s.to_string()).collect();
    let cfg = MutationConfig::default();
    for s in 0..256u64 {
        for child in mutation::mutate(&seed, &alphabet, &cfg, s) {
            if child.origin == RoutineOrigin::Mutated && skill_of(&child).as_deref() == Some(STAT) {
                return Ok(child);
            }
        }
    }
    Err("mutation never produced a stat-using offspring".into())
}

// ---------------------------------------------------------------------------
// Phase 1 — Wire
// ---------------------------------------------------------------------------

fn phase1_wire() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let (_node_b, b_routines, addr_b) = start_peer(tmp.path())?;

    let r = routine("wire.probe", STAT);
    if b_routines.lock().unwrap().get(&r.routine_id).is_some() {
        return Err("peer already had the routine before transfer".into());
    }

    let map = peer_map(&[("node-b", addr_b)]);
    let exec = TcpRemoteExecutor::new(map);
    exec.transfer_routine("node-b", &RoutineTransfer::from_routine(&r))
        .map_err(|e| format!("transfer failed: {e}"))?;
    thread::sleep(Duration::from_millis(200)); // let the listener register it

    let stored = b_routines.lock().unwrap().get(&r.routine_id).cloned();
    match stored {
        Some(got) if got.origin == RoutineOrigin::PeerTransferred => {
            Ok(format!("routine '{}' arrived over TCP as PeerTransferred", got.routine_id))
        }
        Some(got) => Err(format!("routine stored with wrong origin {:?}", got.origin)),
        None => Err("routine never reached the peer's store".into()),
    }
}

// ---------------------------------------------------------------------------
// Phase 2 — Gene flow
// ---------------------------------------------------------------------------

fn phase2_gene_flow() -> Result<String, String> {
    // Node A evolves a stat gene and confirms it works locally.
    let tmp_a = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work_a = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target_a = work_a.path().join("target");
    env_file(&target_a);
    let mut node_a = boot(tmp_a.path())?;
    let gene = evolve_stat_mutant()?;
    if !run_routine(&mut node_a, &gene, &target_a.to_string_lossy()) {
        return Err("evolved gene did not run successfully on node A".into());
    }

    // Node B has never seen it.
    let tmp_b = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work_b = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target_b = work_b.path().join("target");
    env_file(&target_b);
    let (node_b, b_routines, addr_b) = start_peer(tmp_b.path())?;
    if b_routines.lock().unwrap().get(&gene.routine_id).is_some() {
        return Err("node B already had the evolved gene".into());
    }

    // Broadcast it with the production broadcaster — the same type the breeding
    // loop uses — over real TCP.
    let exec: Arc<dyn RemoteExecutor> =
        Arc::new(TcpRemoteExecutor::new(peer_map(&[("node-b", addr_b)])));
    let broadcaster = RemoteRoutineBroadcaster::new(exec, vec!["node-b".into()]);
    broadcaster.broadcast(&gene);
    thread::sleep(Duration::from_millis(200));

    // Node B now possesses the gene (PeerTransferred) and runs it successfully.
    let got = b_routines
        .lock()
        .unwrap()
        .get(&gene.routine_id)
        .cloned()
        .ok_or("evolved gene never reached node B")?;
    if got.origin != RoutineOrigin::PeerTransferred {
        return Err(format!("gene on B has wrong origin {:?}", got.origin));
    }
    let ran = {
        let mut g = node_b.lock().unwrap();
        run_routine(&mut g, &got, &target_b.to_string_lossy())
    };
    if !ran {
        return Err("node B failed to run the transferred gene".into());
    }
    Ok(format!(
        "gene {} (skill {}) evolved on A, propagated to B, and runs there — \
         an instinct B never evolved and no human wrote",
        got.routine_id,
        skill_of(&got).unwrap_or_default()
    ))
}

// ---------------------------------------------------------------------------
// Phase 3 — Swarm
// ---------------------------------------------------------------------------

fn phase3_swarm() -> Result<String, String> {
    let gene = evolve_stat_mutant()?;

    let tmp_b = tempfile::tempdir().map_err(|e| e.to_string())?;
    let tmp_c = tempfile::tempdir().map_err(|e| e.to_string())?;
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = work.path().join("target");
    env_file(&target);
    let target = target.to_string_lossy().to_string();

    let (node_b, b_routines, addr_b) = start_peer(tmp_b.path())?;
    let (node_c, c_routines, addr_c) = start_peer(tmp_c.path())?;

    // One broadcast, two peers.
    let exec: Arc<dyn RemoteExecutor> = Arc::new(TcpRemoteExecutor::new(peer_map(&[
        ("node-b", addr_b),
        ("node-c", addr_c),
    ])));
    let broadcaster =
        RemoteRoutineBroadcaster::new(exec, vec!["node-b".into(), "node-c".into()]);
    broadcaster.broadcast(&gene);
    thread::sleep(Duration::from_millis(300));

    for (label, store, node) in [
        ("B", &b_routines, &node_b),
        ("C", &c_routines, &node_c),
    ] {
        let got = store
            .lock()
            .unwrap()
            .get(&gene.routine_id)
            .cloned()
            .ok_or_else(|| format!("gene never reached node {label}"))?;
        if got.origin != RoutineOrigin::PeerTransferred {
            return Err(format!("gene on {label} has wrong origin {:?}", got.origin));
        }
        let ran = {
            let mut g = node.lock().unwrap();
            run_routine(&mut g, &got, &target)
        };
        if !ran {
            return Err(format!("node {label} failed to run the transferred gene"));
        }
    }
    Ok(format!(
        "one broadcast of {} reached both peers; each runs the unauthored gene",
        gene.routine_id
    ))
}
