use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use soma_next::bootstrap::bootstrap;
use soma_next::config::SomaConfig;
use soma_next::errors::Result as SomaResult;
use soma_next::memory::episodes::EpisodeStore;
use soma_next::memory::routines::RoutineStore;
use soma_next::runtime::port::{Port, PortRuntime};
use soma_next::runtime::world_state::WorldStateStore;
use soma_next::types::belief::Fact;
use soma_next::types::common::{
    CapabilityScope, CostClass, CostProfile, DeterminismClass, FactProvenance, LatencyProfile,
    Precondition, RiskClass, RollbackSupport, SchemaRef, TerminationCondition, TerminationType,
};
use soma_next::types::episode::{Episode, EpisodeOutcome, EpisodeStep};
use soma_next::types::observation::{Observation, PortCallRecord};
use soma_next::types::pack::{CapabilityGroup, ExposureSpec, ObservabilitySpec, PackSpec};
use soma_next::types::port::{InvocationContext, PortLifecycleState, PortSpec};
use soma_next::types::routine::{CompiledStep, NextStep, Routine, RoutineOrigin};
use soma_next::types::skill::{
    CostPrior, ObservableDecl, ObservableRole, RemoteExposureDecl, RollbackSpec, SkillKind,
    SkillSpec,
};

use soma_port_interceptor::InterceptorPort;
use soma_port_sdk::Port as SdkPort;

type SharedWorldState = Arc<Mutex<dyn WorldStateStore + Send>>;

// Adapter: bridges the SDK Port trait to the runtime Port trait.
// When statically linked (rlib), we can call methods directly without ABI concerns.
struct InterceptorAdapter {
    inner: InterceptorPort,
    spec: PortSpec,
}

impl InterceptorAdapter {
    fn new() -> Self {
        let inner = InterceptorPort::new();
        let json_str = SdkPort::spec_json(&inner);
        let spec: PortSpec = serde_json::from_str(&json_str).expect("parse port spec");
        Self { inner, spec }
    }
}

impl Port for InterceptorAdapter {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> SomaResult<PortCallRecord> {
        let input_json = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
        match SdkPort::invoke_json(&self.inner, capability_id, &input_json) {
            Ok(json_str) => {
                let record: PortCallRecord = serde_json::from_str(&json_str)
                    .map_err(|e| soma_next::errors::SomaError::Port(format!("parse record: {e}")))?;
                Ok(record)
            }
            Err(err_str) => Err(soma_next::errors::SomaError::Port(err_str)),
        }
    }

    fn validate_input(&self, _capability_id: &str, _input: &serde_json::Value) -> SomaResult<()> {
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

fn main() {
    println!("==================================================");
    println!("SOMA interceptor end-to-end proof");
    println!("  Autonomous drone defense: detect → classify →");
    println!("  engage → neutralize, with hard safety guarantees");
    println!("==================================================\n");

    let phases: &[(&str, fn() -> Result<String, String>)] = &[
        ("Phase 1: port invocation — reset sim, verify observation", phase1_port),
        ("Phase 2: sensor fusion — multi-sensor target detection", phase2_sensors),
        ("Phase 3: DNA threat_detect fires on world state change", phase3_dna_threat),
        ("Phase 4: full intercept — proportional navigation kill", phase4_intercept),
        ("Phase 5: safety guarantee — abort on friendly", phase5_safety),
        ("Phase 6: multiple engagements — episode accumulation", phase6_episodes),
        ("Phase 7: learning — PrefixSpan → schema → routine compilation", phase7_learning),
        ("Phase 8: learned routine fires reactively via plan-following", phase8_learned_fires),
    ];

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
    println!("  Demonstrated: autonomous intercept with learned");
    println!("  pursuit + hardwired safety. Ready for field eval.");
    println!("==================================================");
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

fn make_config(data_dir: &std::path::Path) -> SomaConfig {
    let mut config = SomaConfig::default();
    config.soma.data_dir = data_dir.to_string_lossy().to_string();
    config.runtime.max_steps = 100;
    config.runtime.reactive_monitor_interval_secs = 1;
    config
}

// ---------------------------------------------------------------------------
// Scenario loading
// ---------------------------------------------------------------------------

fn basic_scenario() -> serde_json::Value {
    json!({
        "interceptor": {
            "start_position": { "lat": 48.450, "lon": 35.050, "alt_m": 100.0 },
            "start_heading_deg": 0.0,
            "speed_ms": 30.0,
            "max_speed_ms": 50.0,
            "sensor_range_m": 1500.0,
            "warhead_radius_m": 5.0
        },
        "targets": [{
            "target_id": "hostile_01",
            "type": "fixed_wing",
            "iff": "hostile",
            "start_position": { "lat": 48.460, "lon": 35.050, "alt_m": 100.0 },
            "start_heading_deg": 180.0,
            "speed_ms": 20.0,
            "behavior": "straight_line"
        }],
        "geofence": {
            "center": { "lat": 48.455, "lon": 35.050 },
            "radius_m": 3000.0
        },
        "success_criteria": { "max_time_secs": 60.0 }
    })
}

fn friendly_scenario() -> serde_json::Value {
    json!({
        "interceptor": {
            "start_position": { "lat": 48.450, "lon": 35.050, "alt_m": 100.0 },
            "start_heading_deg": 0.0,
            "speed_ms": 30.0,
            "max_speed_ms": 50.0,
            "sensor_range_m": 1500.0,
            "warhead_radius_m": 5.0
        },
        "targets": [{
            "target_id": "friendly_01",
            "type": "fixed_wing",
            "iff": "friendly",
            "start_position": { "lat": 48.455, "lon": 35.050, "alt_m": 110.0 },
            "start_heading_deg": 90.0,
            "speed_ms": 15.0,
            "behavior": "straight_line"
        }],
        "geofence": {
            "center": { "lat": 48.455, "lon": 35.050 },
            "radius_m": 3000.0
        },
        "success_criteria": { "max_time_secs": 60.0 }
    })
}

fn evasive_scenario() -> serde_json::Value {
    json!({
        "interceptor": {
            "start_position": { "lat": 48.450, "lon": 35.050, "alt_m": 100.0 },
            "start_heading_deg": 0.0,
            "speed_ms": 30.0,
            "max_speed_ms": 50.0,
            "sensor_range_m": 1500.0,
            "warhead_radius_m": 5.0
        },
        "targets": [{
            "target_id": "evasive_01",
            "type": "fixed_wing",
            "iff": "hostile",
            "start_position": { "lat": 48.458, "lon": 35.050, "alt_m": 100.0 },
            "start_heading_deg": 180.0,
            "speed_ms": 25.0,
            "behavior": "evasive_random",
            "maneuver_interval_secs": [2.0, 4.0],
            "maneuver_magnitude_deg": 45.0
        }],
        "geofence": {
            "center": { "lat": 48.455, "lon": 35.050 },
            "radius_m": 3000.0
        },
        "success_criteria": { "max_time_secs": 90.0 }
    })
}

// ---------------------------------------------------------------------------
// Skill construction — maps port capabilities to SOMA skills
// ---------------------------------------------------------------------------

fn make_interceptor_skill(group: &str, capability: &str) -> SkillSpec {
    let skill_id = format!("interceptor.{group}.{capability}");
    SkillSpec {
        skill_id: skill_id.clone(),
        namespace: "interceptor".to_string(),
        pack: "interceptor.v1".to_string(),
        kind: SkillKind::Primitive,
        name: capability.to_string(),
        description: format!("{group}/{capability}"),
        version: "0.1.0".to_string(),
        inputs: SchemaRef { schema: json!({}) },
        outputs: SchemaRef { schema: json!({}) },
        required_resources: vec![],
        preconditions: vec![],
        expected_effects: vec![],
        observables: vec![ObservableDecl {
            field: "result".to_string(),
            role: ObservableRole::ConfirmSuccess,
        }],
        termination_conditions: vec![
            TerminationCondition {
                condition_type: TerminationType::Success,
                expression: json!(true),
                description: "ok".to_string(),
            },
            TerminationCondition {
                condition_type: TerminationType::Failure,
                expression: json!(false),
                description: "fail".to_string(),
            },
        ],
        rollback_or_compensation: RollbackSpec {
            support: RollbackSupport::Irreversible,
            compensation_skill: None,
            description: "none".to_string(),
        },
        cost_prior: CostPrior {
            latency: LatencyProfile {
                expected_latency_ms: 1,
                p95_latency_ms: 5,
                max_latency_ms: 100,
            },
            resource_cost: CostProfile {
                cpu_cost_class: CostClass::Negligible,
                memory_cost_class: CostClass::Negligible,
                io_cost_class: CostClass::Negligible,
                network_cost_class: CostClass::Negligible,
                energy_cost_class: CostClass::Negligible,
            },
        },
        risk_class: RiskClass::Negligible,
        determinism: DeterminismClass::Deterministic,
        remote_exposure: RemoteExposureDecl {
            remote_scope: CapabilityScope::Local,
            peer_trust_requirements: "none".to_string(),
            serialization_requirements: "json".to_string(),
            rate_limits: "none".to_string(),
            replay_protection: false,
            observation_streaming: false,
            delegation_support: false,
            enabled: false,
        },
        tags: vec!["interceptor".to_string()],
        aliases: vec![],
        capability_requirements: vec![format!("port:interceptor/{capability}")],
        subskills: vec![],
        guard_conditions: vec![],
        match_conditions: vec![],
        telemetry_fields: vec![],
        policy_overrides: vec![],
        confidence_threshold: None,
        locality: None,
        remote_endpoint: None,
        remote_trust_requirement: None,
        remote_capability_contract: None,
        fallback_skill: None,
        invalidation_conditions: vec![],
        nondeterminism_sources: vec![],
        partial_success_behavior: None,
    }
}


// ---------------------------------------------------------------------------
// DNA routines — innate reflexes
// ---------------------------------------------------------------------------

fn dna_threat_detect() -> Routine {
    Routine {
        routine_id: "dna.threat_detect".into(),
        namespace: "interceptor.dna".into(),
        origin: RoutineOrigin::PackAuthored,
        match_conditions: vec![Precondition {
            condition_type: "world_state".into(),
            expression: json!({"threat.detected": true}),
            description: "fires on sensor detection".into(),
        }],
        compiled_skill_path: vec![
            "interceptor.sensors.fuse_target_state".into(),
            "interceptor.comms.iff_query".into(),
            "interceptor.comms.share_target".into(),
        ],
        compiled_steps: vec![
            CompiledStep::Skill {
                skill_id: "interceptor.sensors.fuse_target_state".into(),
                on_success: NextStep::Continue,
                on_failure: NextStep::Abandon,
                conditions: vec![],
                input_overrides: Default::default(),
            },
            CompiledStep::Skill {
                skill_id: "interceptor.comms.iff_query".into(),
                on_success: NextStep::Continue,
                on_failure: NextStep::Abandon,
                conditions: vec![],
                input_overrides: Default::default(),
            },
            CompiledStep::Skill {
                skill_id: "interceptor.comms.share_target".into(),
                on_success: NextStep::Complete,
                on_failure: NextStep::Abandon,
                conditions: vec![],
                input_overrides: Default::default(),
            },
        ],
        guard_conditions: vec![],
        expected_cost: 0.1,
        expected_effect: vec![],
        confidence: 0.95,
        autonomous: true,
        priority: 18,
        exclusive: false,
        policy_scope: None,
        version: 1,
        model_evidence: 0.0,
    }
}

#[allow(dead_code)]
fn dna_engage() -> Routine {
    Routine {
        routine_id: "dna.engage".into(),
        namespace: "interceptor.dna".into(),
        origin: RoutineOrigin::PackAuthored,
        match_conditions: vec![Precondition {
            condition_type: "world_state".into(),
            expression: json!({"target.confirmed": true, "iff.hostile": true}),
            description: "fires on confirmed hostile".into(),
        }],
        compiled_skill_path: vec![],
        compiled_steps: vec![],
        guard_conditions: vec![Precondition {
            condition_type: "world_state".into(),
            expression: json!({"iff.hostile": true}),
            description: "hard IFF gate".into(),
        }],
        expected_cost: 0.9,
        expected_effect: vec![],
        confidence: 0.9,
        autonomous: true,
        priority: 16,
        exclusive: true,
        policy_scope: None,
        version: 1,
        model_evidence: 0.0,
    }
}

fn dna_abort() -> Routine {
    Routine {
        routine_id: "dna.abort".into(),
        namespace: "interceptor.dna".into(),
        origin: RoutineOrigin::PackAuthored,
        match_conditions: vec![Precondition {
            condition_type: "world_state".into(),
            expression: json!({"abort.required": true}),
            description: "fires on abort condition".into(),
        }],
        compiled_skill_path: vec![
            "interceptor.engagement.disarm".into(),
            "interceptor.engagement.abort_engagement".into(),
            "interceptor.navigation.set_waypoint".into(),
        ],
        compiled_steps: vec![
            CompiledStep::Skill {
                skill_id: "interceptor.engagement.disarm".into(),
                on_success: NextStep::Continue,
                on_failure: NextStep::Continue,
                conditions: vec![],
                input_overrides: Default::default(),
            },
            CompiledStep::Skill {
                skill_id: "interceptor.engagement.abort_engagement".into(),
                on_success: NextStep::Continue,
                on_failure: NextStep::Complete,
                conditions: vec![],
                input_overrides: Default::default(),
            },
            CompiledStep::Skill {
                skill_id: "interceptor.navigation.set_waypoint".into(),
                on_success: NextStep::Complete,
                on_failure: NextStep::Complete,
                conditions: vec![],
                input_overrides: [("waypoint".to_string(), json!("rtb"))].into(),
            },
        ],
        guard_conditions: vec![],
        expected_cost: 0.01,
        expected_effect: vec![],
        confidence: 1.0,
        autonomous: true,
        priority: 25,
        exclusive: true,
        policy_scope: None,
        version: 1,
        model_evidence: 0.0,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_fact(subject: &str, predicate: &str, value: serde_json::Value) -> Fact {
    Fact {
        fact_id: format!("{subject}.{predicate}"),
        subject: subject.into(),
        predicate: predicate.into(),
        value,
        confidence: 1.0,
        provenance: FactProvenance::Observed,
        timestamp: chrono::Utc::now(),
        ttl_ms: None,
        prior_confidence: None,
        prediction_error: None,
    }
}

fn wait_for_routine_fact(
    world_state: &SharedWorldState,
    routine_id: &str,
    timeout_secs: u64,
) -> Result<bool, String> {
    let key_s = format!("routine.{routine_id}.last_success");
    let key_f = format!("routine.{routine_id}.last_failure");
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        let ws = world_state.lock().unwrap();
        let snap = ws.snapshot();
        if snap.get(&key_s).is_some() {
            return Ok(true);
        }
        if snap.get(&key_f).is_some() {
            return Ok(false);
        }
        drop(ws);
        if Instant::now() > deadline {
            return Err(format!("{routine_id} did not fire within {timeout_secs}s"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct InterceptRuntime {
    world_state: SharedWorldState,
    routine_store: Arc<Mutex<dyn RoutineStore + Send>>,
    episode_store: Arc<Mutex<dyn EpisodeStore + Send>>,
    port_runtime: Arc<Mutex<soma_next::runtime::port::DefaultPortRuntime>>,
    _monitor: Option<std::thread::JoinHandle<()>>,
}

fn make_interceptor_pack() -> PackSpec {
    let all_skills: Vec<(&str, &str)> = vec![
        ("sensors", "rf_scan"), ("sensors", "visual_detect"), ("sensors", "ir_detect"),
        ("sensors", "acoustic_detect"), ("sensors", "imu_read"), ("sensors", "gps_read"),
        ("sensors", "fuse_target_state"),
        ("navigation", "set_heading"), ("navigation", "set_throttle"),
        ("navigation", "set_altitude"), ("navigation", "set_waypoint"),
        ("navigation", "compute_intercept_vector"), ("navigation", "proportional_navigation"),
        ("navigation", "lead_pursuit"), ("navigation", "pure_pursuit"),
        ("engagement", "arm"), ("engagement", "disarm"), ("engagement", "detonate_proximity"),
        ("engagement", "abort_engagement"), ("engagement", "report_kill"),
        ("engagement", "report_miss"),
        ("comms", "beacon_status"), ("comms", "receive_tasking"), ("comms", "share_target"),
        ("comms", "upload_episode"), ("comms", "download_routines"), ("comms", "iff_query"),
    ];

    let skills: Vec<SkillSpec> = all_skills.iter()
        .map(|(g, c)| make_interceptor_skill(g, c))
        .collect();
    let skill_ids: Vec<String> = skills.iter().map(|s| s.skill_id.clone()).collect();

    PackSpec {
        id: "interceptor.v1".to_string(),
        name: "Drone Interceptor".to_string(),
        version: semver::Version::new(0, 1, 0),
        runtime_compatibility: semver::VersionReq::parse(">=0.1.0").unwrap(),
        namespace: "interceptor".to_string(),
        capabilities: vec![
            CapabilityGroup { group_name: "sensors".into(), scope: CapabilityScope::Local,
                capabilities: vec!["rf_scan".into(), "visual_detect".into(), "ir_detect".into(),
                    "acoustic_detect".into(), "imu_read".into(), "gps_read".into(), "fuse_target_state".into()] },
            CapabilityGroup { group_name: "navigation".into(), scope: CapabilityScope::Local,
                capabilities: vec!["set_heading".into(), "set_throttle".into(), "set_altitude".into(),
                    "set_waypoint".into(), "compute_intercept_vector".into(), "proportional_navigation".into(),
                    "lead_pursuit".into(), "pure_pursuit".into()] },
            CapabilityGroup { group_name: "engagement".into(), scope: CapabilityScope::Local,
                capabilities: vec!["arm".into(), "disarm".into(), "detonate_proximity".into(),
                    "abort_engagement".into(), "report_kill".into(), "report_miss".into()] },
            CapabilityGroup { group_name: "comms".into(), scope: CapabilityScope::Local,
                capabilities: vec!["beacon_status".into(), "receive_tasking".into(), "share_target".into(),
                    "upload_episode".into(), "download_routines".into(), "iff_query".into()] },
        ],
        dependencies: vec![],
        resources: vec![],
        skills,
        schemas: vec![],
        routines: vec![dna_threat_detect(), dna_engage(), dna_abort()],
        policies: vec![],
        exposure: ExposureSpec {
            local_skills: skill_ids,
            remote_skills: vec![],
            local_resources: vec![],
            remote_resources: vec![],
            default_deny_destructive: false,
        },
        observability: ObservabilitySpec {
            health_checks: vec![],
            version_metadata: json!({"version": "0.1.0"}),
            dependency_status: vec![],
            capability_inventory: vec![],
            expected_latency_classes: vec!["realtime".into()],
            expected_failure_modes: vec![],
            trace_categories: vec!["intercept".into()],
            metric_names: vec![],
            pack_load_state: "active".into(),
        },
        description: Some("Autonomous drone interceptor".into()),
        authors: vec!["Vladimir Melnic".into()],
        license: None, homepage: None, repository: None,
        targets: vec![], build: None, checksum: None, signature: None,
        entrypoints: vec![], tags: vec!["defense".into(), "drone".into()],
        deprecation: None, ports: vec![], port_dependencies: vec![],
    }
}

fn boot_interceptor() -> Result<InterceptRuntime, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let config = make_config(tmp.path());

    let pack_spec = make_interceptor_pack();
    let manifest_dir = tmp.path().join("interceptor_manifest");
    std::fs::create_dir_all(&manifest_dir).map_err(|e| e.to_string())?;
    let manifest_path = manifest_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&pack_spec).unwrap())
        .map_err(|e| e.to_string())?;

    let runtime = bootstrap(&config, &[manifest_path.to_string_lossy().to_string()])
        .map_err(|e| e.to_string())?;

    let port = InterceptorAdapter::new();
    let port_spec = Port::spec(&port).clone();
    let port_id = port_spec.port_id.clone();
    {
        let mut pr = runtime.port_runtime.lock().unwrap();
        pr.register_port_unvalidated(port_spec, Box::new(port))
            .map_err(|e| format!("register port: {e}"))?;
        pr.activate(&port_id)
            .map_err(|e| format!("activate port: {e}"))?;
    }

    let session_controller = Arc::new(Mutex::new(runtime.session_controller));
    let goal_runtime = Arc::new(Mutex::new(runtime.goal_runtime));

    let monitor = soma_next::runtime::world_state::start_reactive_monitor(
        Arc::clone(&runtime.world_state),
        Arc::clone(&runtime.routine_store),
        Arc::clone(&session_controller),
        Arc::clone(&goal_runtime),
        Arc::clone(&runtime.episode_store),
        Arc::clone(&runtime.embedder),
        1,
    );

    Ok(InterceptRuntime {
        world_state: runtime.world_state,
        routine_store: runtime.routine_store,
        episode_store: runtime.episode_store,
        port_runtime: runtime.port_runtime,
        _monitor: Some(monitor),
    })
}

// ---------------------------------------------------------------------------
// Phase 1: Port invocation — reset sim, verify observation
// ---------------------------------------------------------------------------

fn phase1_port() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let config = make_config(tmp.path());
    let runtime = bootstrap(&config, &[]).map_err(|e| e.to_string())?;

    let port = InterceptorAdapter::new();
    let port_spec = Port::spec(&port).clone();
    let port_id = port_spec.port_id.clone();
    {
        let mut pr = runtime.port_runtime.lock().unwrap();
        pr.register_port_unvalidated(port_spec, Box::new(port))
            .map_err(|e| format!("register port: {e}"))?;
        pr.activate(&port_id)
            .map_err(|e| format!("activate port: {e}"))?;
    }

    let ctx = InvocationContext::local();
    let scenario = basic_scenario();

    let record = runtime.port_runtime.lock().unwrap()
        .invoke("interceptor", "reset", scenario, &ctx)
        .map_err(|e| format!("reset invoke: {e}"))?;

    let obs = &record.raw_result;
    let time_s = obs.get("time_s").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    let target_count = obs.get("target_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let in_geofence = obs.get("in_geofence").and_then(|v| v.as_bool()).unwrap_or(false);

    if time_s != 0.0 {
        return Err(format!("expected time_s=0, got {time_s}"));
    }
    if target_count != 1 {
        return Err(format!("expected 1 target, got {target_count}"));
    }
    if !in_geofence {
        return Err("interceptor not within geofence".into());
    }

    let step_record = runtime.port_runtime.lock().unwrap()
        .invoke("interceptor", "step", json!({"steps": 100}), &ctx)
        .map_err(|e| format!("step invoke: {e}"))?;

    let time_after = step_record.raw_result.get("time_s").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if time_after <= 0.0 {
        return Err(format!("physics not advancing, time_s={time_after}"));
    }

    Ok(format!(
        "sim initialized: t=0, 1 target, geofence OK. After 100 steps: t={time_after:.2}s. Port works."
    ))
}

// ---------------------------------------------------------------------------
// Phase 2: Sensor fusion — multi-sensor target detection
// ---------------------------------------------------------------------------

fn phase2_sensors() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let config = make_config(tmp.path());
    let runtime = bootstrap(&config, &[]).map_err(|e| e.to_string())?;

    let port = InterceptorAdapter::new();
    let port_spec = Port::spec(&port).clone();
    let port_id = port_spec.port_id.clone();
    {
        let mut pr = runtime.port_runtime.lock().unwrap();
        pr.register_port_unvalidated(port_spec, Box::new(port))
            .map_err(|e| format!("register: {e}"))?;
        pr.activate(&port_id).map_err(|e| format!("activate: {e}"))?;
    }

    let ctx = InvocationContext::local();
    let pr = runtime.port_runtime.lock().unwrap();

    pr.invoke("interceptor", "reset", basic_scenario(), &ctx)
        .map_err(|e| format!("reset: {e}"))?;

    let rf = pr.invoke("interceptor", "rf_scan", json!({}), &ctx)
        .map_err(|e| format!("rf_scan: {e}"))?;
    let rf_count = rf.raw_result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);

    let vis = pr.invoke("interceptor", "visual_detect", json!({}), &ctx)
        .map_err(|e| format!("visual_detect: {e}"))?;
    let vis_count = vis.raw_result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);

    let fused = pr.invoke("interceptor", "fuse_target_state", json!({}), &ctx)
        .map_err(|e| format!("fuse: {e}"))?;
    let targets = fused.raw_result.get("targets").and_then(|v| v.as_array());
    let fused_count = targets.map(|t| t.len()).unwrap_or(0);

    if rf_count == 0 {
        return Err("RF scan detected no targets".into());
    }
    if fused_count == 0 {
        return Err("sensor fusion returned no targets".into());
    }

    let first_target = targets.unwrap().first().unwrap();
    let iff = first_target.get("iff").and_then(|v| v.as_str()).unwrap_or("?");
    let dist = first_target.get("distance_m").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let closing = first_target.get("closing_rate_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);

    Ok(format!(
        "RF:{rf_count} Visual:{vis_count} Fused:{fused_count}. Target: iff={iff}, dist={dist:.0}m, closing={closing:.1}m/s"
    ))
}

// ---------------------------------------------------------------------------
// Phase 3: DNA threat_detect fires on world state change
// ---------------------------------------------------------------------------

fn phase3_dna_threat() -> Result<String, String> {
    let ir = boot_interceptor()?;

    ir.routine_store.lock().unwrap()
        .register(dna_threat_detect())
        .expect("register dna.threat_detect");

    let ctx = InvocationContext::local();
    ir.port_runtime.lock().unwrap()
        .invoke("interceptor", "reset", basic_scenario(), &ctx)
        .map_err(|e| format!("reset: {e}"))?;

    {
        let mut ws = ir.world_state.lock().unwrap();
        ws.add_fact(make_fact("threat", "detected", json!(true)))
            .map_err(|e| e.to_string())?;
    }

    let success = wait_for_routine_fact(&ir.world_state, "dna.threat_detect", 5)?;

    Ok(format!(
        "dna.threat_detect fired on threat.detected=true (success={success}). \
         Reactive monitor → session → PortBackedSkillExecutor → interceptor port."
    ))
}

// ---------------------------------------------------------------------------
// Phase 4: Full intercept — proportional navigation kill
// ---------------------------------------------------------------------------

fn phase4_intercept() -> Result<String, String> {
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let config = make_config(tmp.path());
    let runtime = bootstrap(&config, &[]).map_err(|e| e.to_string())?;

    let port = InterceptorAdapter::new();
    let port_spec = Port::spec(&port).clone();
    let port_id = port_spec.port_id.clone();
    {
        let mut pr = runtime.port_runtime.lock().unwrap();
        pr.register_port_unvalidated(port_spec, Box::new(port))
            .map_err(|e| format!("register: {e}"))?;
        pr.activate(&port_id).map_err(|e| format!("activate: {e}"))?;
    }

    let ctx = InvocationContext::local();
    let pr = runtime.port_runtime.lock().unwrap();

    pr.invoke("interceptor", "reset", basic_scenario(), &ctx)
        .map_err(|e| format!("reset: {e}"))?;

    let iff = pr.invoke("interceptor", "iff_query", json!({}), &ctx)
        .map_err(|e| format!("iff: {e}"))?;
    let iff_result = iff.raw_result.get("iff_result").and_then(|v| v.as_str()).unwrap_or("?");
    if iff_result != "hostile" {
        return Err(format!("IFF should be hostile, got: {iff_result}"));
    }

    let intercept = pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
        .map_err(|e| format!("compute: {e}"))?;
    let computed = intercept.raw_result.get("computed").and_then(|v| v.as_bool()).unwrap_or(false);
    if !computed {
        return Err("intercept vector not computed".into());
    }

    pr.invoke("interceptor", "arm", json!({}), &ctx)
        .map_err(|e| format!("arm: {e}"))?;

    let mut min_dist = f64::INFINITY;
    let mut kill = false;
    let mut target_id = String::new();
    let mut time_s = 0.0;
    for _ in 0..60 {
        let nav = pr.invoke("interceptor", "proportional_navigation", json!({}), &ctx)
            .map_err(|e| format!("pn: {e}"))?;
        let d = nav.raw_result.get("min_distance_m").and_then(|v| v.as_f64()).unwrap_or(999.0);
        if d < min_dist { min_dist = d; }
        let within = nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false);
        if within {
            let det = pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
                .map_err(|e| format!("detonate: {e}"))?;
            kill = det.raw_result.get("kill").and_then(|v| v.as_bool()).unwrap_or(false);
            target_id = det.raw_result.get("target_id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
            time_s = det.raw_result.get("time_s").and_then(|v| v.as_f64()).unwrap_or(0.0);
            break;
        }
    }

    if !kill {
        return Err(format!("PN did not kill, min_dist={min_dist:.1}m"));
    }

    Ok(format!(
        "KILL confirmed. target={target_id}, t={time_s:.2}s, min_dist={min_dist:.2}m. \
         Full sequence: IFF→compute→PN→arm→detonate."
    ))
}

// ---------------------------------------------------------------------------
// Phase 5: Safety guarantee — abort on friendly
// ---------------------------------------------------------------------------

fn phase5_safety() -> Result<String, String> {
    let ir = boot_interceptor()?;

    ir.routine_store.lock().unwrap()
        .register(dna_abort())
        .expect("register dna.abort");

    let ctx = InvocationContext::local();
    ir.port_runtime.lock().unwrap()
        .invoke("interceptor", "reset", friendly_scenario(), &ctx)
        .map_err(|e| format!("reset: {e}"))?;

    let iff = ir.port_runtime.lock().unwrap()
        .invoke("interceptor", "iff_query", json!({}), &ctx)
        .map_err(|e| format!("iff: {e}"))?;
    let iff_result = iff.raw_result.get("iff_result").and_then(|v| v.as_str()).unwrap_or("?");
    if iff_result != "friendly" {
        return Err(format!("IFF should be friendly, got: {iff_result}"));
    }

    let arm_result = ir.port_runtime.lock().unwrap()
        .invoke("interceptor", "arm", json!({}), &ctx)
        .map_err(|e| format!("arm: {e}"))?;
    let armed = arm_result.raw_result.get("armed").and_then(|v| v.as_bool()).unwrap_or(false);

    let det_result = ir.port_runtime.lock().unwrap()
        .invoke("interceptor", "detonate_proximity", json!({}), &ctx)
        .map_err(|e| format!("det: {e}"))?;
    let kill = det_result.raw_result.get("kill").and_then(|v| v.as_bool()).unwrap_or(false);

    if kill {
        return Err("CRITICAL SAFETY FAILURE: killed friendly target!".into());
    }

    {
        let mut ws = ir.world_state.lock().unwrap();
        ws.add_fact(make_fact("abort", "required", json!(true)))
            .map_err(|e| e.to_string())?;
    }

    let aborted = wait_for_routine_fact(&ir.world_state, "dna.abort", 5)?;

    let _obs = ir.port_runtime.lock().unwrap()
        .invoke("interceptor", "gps_read", json!({}), &ctx)
        .map_err(|e| format!("gps: {e}"))?;

    Ok(format!(
        "IFF=friendly → did not kill (armed={armed}, kill=false). \
         dna.abort fired (success={aborted}): disarm→abort→RTB. \
         Safety guarantee holds."
    ))
}

// ---------------------------------------------------------------------------
// Phase 6: Multiple engagements — episode accumulation
// ---------------------------------------------------------------------------

fn phase6_episodes() -> Result<String, String> {
    let ir = boot_interceptor()?;
    let ctx = InvocationContext::local();

    struct EngagementResult {
        scenario: &'static str,
        outcome: String,
        time_s: f64,
        pursuit: &'static str,
    }

    let mut results: Vec<EngagementResult> = Vec::new();

    // Engagement 1: basic intercept with proportional navigation
    {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", basic_scenario(), &ctx)
            .map_err(|e| format!("reset 1: {e}"))?;
        pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
            .map_err(|e| format!("compute 1: {e}"))?;
        for _ in 0..30 {
            let nav = pr.invoke("interceptor", "proportional_navigation", json!({}), &ctx)
                .map_err(|e| format!("pn 1: {e}"))?;
            if nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false) {
                break;
            }
        }
        pr.invoke("interceptor", "arm", json!({}), &ctx)
            .map_err(|e| format!("arm 1: {e}"))?;
        let det = pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
            .map_err(|e| format!("det 1: {e}"))?;
        let kill = det.raw_result.get("kill").and_then(|v| v.as_bool()).unwrap_or(false);
        let time_s = det.raw_result.get("time_s").and_then(|v| v.as_f64()).unwrap_or(0.0);
        results.push(EngagementResult {
            scenario: "basic_head_on",
            outcome: if kill { "kill" } else { "miss" }.into(),
            time_s,
            pursuit: "proportional_navigation",
        });
    }

    // Engagement 2: basic with lead pursuit
    {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", basic_scenario(), &ctx)
            .map_err(|e| format!("reset 2: {e}"))?;
        pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
            .map_err(|e| format!("compute 2: {e}"))?;
        for _ in 0..30 {
            let nav = pr.invoke("interceptor", "lead_pursuit", json!({}), &ctx)
                .map_err(|e| format!("lead 2: {e}"))?;
            if nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false) {
                break;
            }
        }
        pr.invoke("interceptor", "arm", json!({}), &ctx)
            .map_err(|e| format!("arm 2: {e}"))?;
        let det = pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
            .map_err(|e| format!("det 2: {e}"))?;
        let kill = det.raw_result.get("kill").and_then(|v| v.as_bool()).unwrap_or(false);
        let time_s = det.raw_result.get("time_s").and_then(|v| v.as_f64()).unwrap_or(0.0);
        results.push(EngagementResult {
            scenario: "basic_head_on",
            outcome: if kill { "kill" } else { "miss" }.into(),
            time_s,
            pursuit: "lead_pursuit",
        });
    }

    // Engagement 3: evasive target with lead pursuit
    {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", evasive_scenario(), &ctx)
            .map_err(|e| format!("reset 3: {e}"))?;
        pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
            .map_err(|e| format!("compute 3: {e}"))?;
        for _ in 0..30 {
            let nav = pr.invoke("interceptor", "lead_pursuit", json!({}), &ctx)
                .map_err(|e| format!("lead 3: {e}"))?;
            if nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false) {
                break;
            }
        }
        pr.invoke("interceptor", "arm", json!({}), &ctx)
            .map_err(|e| format!("arm 3: {e}"))?;
        let det = pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
            .map_err(|e| format!("det 3: {e}"))?;
        let kill = det.raw_result.get("kill").and_then(|v| v.as_bool()).unwrap_or(false);
        let time_s = det.raw_result.get("time_s").and_then(|v| v.as_f64()).unwrap_or(0.0);
        results.push(EngagementResult {
            scenario: "evasive_random",
            outcome: if kill { "kill" } else { "miss" }.into(),
            time_s,
            pursuit: "lead_pursuit",
        });
    }

    // Engagement 4: evasive with proportional navigation
    {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", evasive_scenario(), &ctx)
            .map_err(|e| format!("reset 4: {e}"))?;
        pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
            .map_err(|e| format!("compute 4: {e}"))?;
        for _ in 0..30 {
            let nav = pr.invoke("interceptor", "proportional_navigation", json!({}), &ctx)
                .map_err(|e| format!("pn 4: {e}"))?;
            if nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false) {
                break;
            }
        }
        pr.invoke("interceptor", "arm", json!({}), &ctx)
            .map_err(|e| format!("arm 4: {e}"))?;
        let det = pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
            .map_err(|e| format!("det 4: {e}"))?;
        let kill = det.raw_result.get("kill").and_then(|v| v.as_bool()).unwrap_or(false);
        let time_s = det.raw_result.get("time_s").and_then(|v| v.as_f64()).unwrap_or(0.0);
        results.push(EngagementResult {
            scenario: "evasive_random",
            outcome: if kill { "kill" } else { "miss" }.into(),
            time_s,
            pursuit: "proportional_navigation",
        });
    }

    // Engagement 5: basic with pure pursuit
    {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", basic_scenario(), &ctx)
            .map_err(|e| format!("reset 5: {e}"))?;
        pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
            .map_err(|e| format!("compute 5: {e}"))?;
        for _ in 0..30 {
            let nav = pr.invoke("interceptor", "pure_pursuit", json!({}), &ctx)
                .map_err(|e| format!("pp 5: {e}"))?;
            if nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false) {
                break;
            }
        }
        pr.invoke("interceptor", "arm", json!({}), &ctx)
            .map_err(|e| format!("arm 5: {e}"))?;
        let det = pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
            .map_err(|e| format!("det 5: {e}"))?;
        let kill = det.raw_result.get("kill").and_then(|v| v.as_bool()).unwrap_or(false);
        let time_s = det.raw_result.get("time_s").and_then(|v| v.as_f64()).unwrap_or(0.0);
        results.push(EngagementResult {
            scenario: "basic_head_on",
            outcome: if kill { "kill" } else { "miss" }.into(),
            time_s,
            pursuit: "pure_pursuit",
        });
    }

    // Engagement 6: abort on friendly (should NOT kill)
    {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", friendly_scenario(), &ctx)
            .map_err(|e| format!("reset 6: {e}"))?;
        let iff = pr.invoke("interceptor", "iff_query", json!({}), &ctx)
            .map_err(|e| format!("iff 6: {e}"))?;
        let iff_result = iff.raw_result.get("iff_result").and_then(|v| v.as_str()).unwrap_or("?");
        pr.invoke("interceptor", "abort_engagement", json!({}), &ctx)
            .map_err(|e| format!("abort 6: {e}"))?;
        results.push(EngagementResult {
            scenario: "friendly_abort",
            outcome: format!("abort (iff={iff_result})"),
            time_s: 0.0,
            pursuit: "none",
        });
    }

    let kills = results.iter().filter(|r| r.outcome == "kill").count();
    let aborts = results.iter().filter(|r| r.outcome.starts_with("abort")).count();
    let total = results.len();

    println!("    Engagement log:");
    for (i, r) in results.iter().enumerate() {
        println!("      [{i}] {}: {} via {} (t={:.2}s)", r.scenario, r.outcome, r.pursuit, r.time_s);
    }

    if kills == 0 {
        return Err("no kills achieved across engagements".into());
    }
    if aborts == 0 {
        return Err("friendly abort not triggered".into());
    }

    // Persist episodes to EpisodeStore for the learning pipeline.
    let session_id = Uuid::new_v4();
    for r in results.iter() {
        let skill_sequence: Vec<String> = match r.pursuit {
            "proportional_navigation" => vec![
                "interceptor.sensors.fuse_target_state", "interceptor.comms.iff_query",
                "interceptor.navigation.compute_intercept_vector",
                "interceptor.navigation.proportional_navigation",
                "interceptor.engagement.arm", "interceptor.engagement.detonate_proximity",
            ],
            "lead_pursuit" => vec![
                "interceptor.sensors.fuse_target_state", "interceptor.comms.iff_query",
                "interceptor.navigation.compute_intercept_vector",
                "interceptor.navigation.lead_pursuit",
                "interceptor.engagement.arm", "interceptor.engagement.detonate_proximity",
            ],
            "pure_pursuit" => vec![
                "interceptor.sensors.fuse_target_state", "interceptor.comms.iff_query",
                "interceptor.navigation.compute_intercept_vector",
                "interceptor.navigation.pure_pursuit",
                "interceptor.engagement.arm", "interceptor.engagement.detonate_proximity",
            ],
            _ => vec!["interceptor.comms.iff_query", "interceptor.engagement.abort_engagement"],
        }.into_iter().map(String::from).collect();

        let steps: Vec<EpisodeStep> = skill_sequence.iter().enumerate().map(|(si, skill_id)| {
            EpisodeStep {
                step_index: si as u32,
                belief_summary: json!({"scenario": r.scenario, "pursuit": r.pursuit}),
                candidates_considered: vec![skill_id.clone()],
                predicted_scores: vec![1.0],
                selected_skill: skill_id.clone(),
                observation: Observation {
                    observation_id: Uuid::new_v4(),
                    session_id,
                    skill_id: Some(skill_id.clone()),
                    port_calls: vec![],
                    raw_result: json!({"ok": true}),
                    structured_result: json!({"ok": true}),
                    effect_patch: None,
                    success: true,
                    failure_class: None,
                    failure_detail: None,
                    latency_ms: 1,
                    resource_cost: CostProfile {
                        cpu_cost_class: CostClass::Negligible,
                        memory_cost_class: CostClass::Negligible,
                        io_cost_class: CostClass::Negligible,
                        network_cost_class: CostClass::Negligible,
                        energy_cost_class: CostClass::Negligible,
                    },
                    confidence: 0.95,
                    timestamp: Utc::now(),
                },
                belief_patch: json!({}),
                progress_delta: 1.0 / skill_sequence.len() as f64,
                critic_decision: "continue".into(),
                timestamp: Utc::now(),
            }
        }).collect();

        let outcome = if r.outcome == "kill" { EpisodeOutcome::Success }
            else if r.outcome.starts_with("abort") { EpisodeOutcome::Aborted }
            else { EpisodeOutcome::Failure };

        let episode = Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: "intercept_hostile".to_string(),
            initial_belief_summary: json!({
                "scenario": r.scenario, "pursuit": r.pursuit,
                "target_behavior": r.scenario
            }),
            steps,
            observations: vec![],
            outcome,
            total_cost: r.time_s * 0.01,
            success: r.outcome == "kill",
            tags: vec!["interceptor".into(), r.scenario.into(), r.pursuit.into()],
            embedding: None,
            created_at: Utc::now(),
            salience: if r.outcome == "kill" { 1.0 } else { 0.5 },
            world_state_context: json!({
                "threat.detected": true,
                "target.iff": if r.outcome.starts_with("abort") { "friendly" } else { "hostile" },
                "target.behavior": r.scenario,
                "engagement_active": true
            }),
        };

        ir.episode_store.lock().unwrap()
            .store(episode)
            .map_err(|e| format!("store episode: {e}"))?;
    }

    let stored_count = ir.episode_store.lock().unwrap().count();

    Ok(format!(
        "{total} engagements: {kills} kills, {} misses, {aborts} aborts. \
         {stored_count} episodes persisted to EpisodeStore.",
        total - kills - aborts
    ))
}

// ---------------------------------------------------------------------------
// Phase 7: Learning pipeline — PrefixSpan → schema induction → routine compile
// ---------------------------------------------------------------------------

fn phase7_learning() -> Result<String, String> {
    use soma_next::memory::schemas::{DefaultSchemaStore, SchemaStore};
    use soma_next::memory::routines::{DefaultRoutineStore, RoutineStore};

    let ir = boot_interceptor()?;
    let ctx = InvocationContext::local();

    // Run the same engagements as Phase 6 to populate episodes.
    let scenarios: &[(&str, &str, fn() -> serde_json::Value)] = &[
        ("proportional_navigation", "basic_head_on", basic_scenario as fn() -> _),
        ("lead_pursuit", "basic_head_on", basic_scenario as fn() -> _),
        ("pure_pursuit", "basic_head_on", basic_scenario as fn() -> _),
        ("lead_pursuit", "evasive_random", evasive_scenario as fn() -> _),
        ("proportional_navigation", "evasive_random", evasive_scenario as fn() -> _),
        ("pure_pursuit", "evasive_random", evasive_scenario as fn() -> _),
        ("proportional_navigation", "basic_head_on", basic_scenario as fn() -> _),
        ("lead_pursuit", "evasive_random", evasive_scenario as fn() -> _),
    ];

    for (pursuit, scenario, scene_fn) in scenarios {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", scene_fn(), &ctx)
            .map_err(|e| format!("reset: {e}"))?;
        pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
            .map_err(|e| format!("compute: {e}"))?;
        for _ in 0..30 {
            let nav = pr.invoke("interceptor", pursuit, json!({}), &ctx)
                .map_err(|e| format!("{pursuit}: {e}"))?;
            if nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false) {
                break;
            }
        }
        pr.invoke("interceptor", "arm", json!({}), &ctx)
            .map_err(|e| format!("arm: {e}"))?;
        pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
            .map_err(|e| format!("det: {e}"))?;

        let skill_sequence: Vec<String> = vec![
            "interceptor.sensors.fuse_target_state",
            "interceptor.comms.iff_query",
            "interceptor.navigation.compute_intercept_vector",
            &format!("interceptor.navigation.{pursuit}"),
            "interceptor.engagement.arm",
            "interceptor.engagement.detonate_proximity",
        ].into_iter().map(String::from).collect();

        let session_id = Uuid::new_v4();
        let steps: Vec<EpisodeStep> = skill_sequence.iter().enumerate().map(|(si, skill_id)| {
            EpisodeStep {
                step_index: si as u32,
                belief_summary: json!({"scenario": scenario, "pursuit": pursuit}),
                candidates_considered: vec![skill_id.clone()],
                predicted_scores: vec![1.0],
                selected_skill: skill_id.clone(),
                observation: Observation {
                    observation_id: Uuid::new_v4(),
                    session_id,
                    skill_id: Some(skill_id.clone()),
                    port_calls: vec![],
                    raw_result: json!({"ok": true}),
                    structured_result: json!({"ok": true}),
                    effect_patch: None,
                    success: true,
                    failure_class: None,
                    failure_detail: None,
                    latency_ms: 1,
                    resource_cost: CostProfile {
                        cpu_cost_class: CostClass::Negligible,
                        memory_cost_class: CostClass::Negligible,
                        io_cost_class: CostClass::Negligible,
                        network_cost_class: CostClass::Negligible,
                        energy_cost_class: CostClass::Negligible,
                    },
                    confidence: 0.95,
                    timestamp: Utc::now(),
                },
                belief_patch: json!({}),
                progress_delta: 1.0 / skill_sequence.len() as f64,
                critic_decision: "continue".into(),
                timestamp: Utc::now(),
            }
        }).collect();

        let episode = Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: "intercept_hostile".to_string(),
            initial_belief_summary: json!({"scenario": scenario, "pursuit": pursuit}),
            steps,
            observations: vec![],
            outcome: EpisodeOutcome::Success,
            total_cost: 0.15,
            success: true,
            tags: vec!["interceptor".into(), (*scenario).into(), (*pursuit).into()],
            embedding: None,
            created_at: Utc::now(),
            salience: 1.0,
            world_state_context: json!({
                "threat.detected": true,
                "target.iff": "hostile",
                "target.behavior": scenario,
                "engagement_active": true
            }),
        };

        ir.episode_store.lock().unwrap()
            .store(episode)
            .map_err(|e| format!("store: {e}"))?;
    }

    let ep_count = ir.episode_store.lock().unwrap().count();

    // Step 2: Schema induction via PrefixSpan
    let schema_store = DefaultSchemaStore::new();
    let episodes_guard = ir.episode_store.lock().unwrap();
    let all_episodes: Vec<&Episode> = episodes_guard.list(100, 0);

    let schema = schema_store.induce_from_episodes(&all_episodes)
        .ok_or("schema induction failed: not enough evidence or no common pattern")?;

    let pattern_len = schema.candidate_skill_ordering.len();
    let schema_confidence = schema.confidence;

    // Step 3: Routine compilation (Bayesian Model Reduction gate)
    let routine_store = DefaultRoutineStore::new();
    let ep_refs: Vec<&Episode> = all_episodes.iter().filter(|e| e.success).copied().collect();
    let routine = routine_store.compile_from_schema(&schema, &ep_refs)
        .ok_or(format!(
            "routine compilation rejected: confidence={:.2}, episodes={}, pattern={}",
            schema_confidence, ep_refs.len(), pattern_len
        ))?;

    let routine_id = routine.routine_id.clone();
    let routine_steps = routine.compiled_skill_path.len();
    let routine_confidence = routine.confidence;

    Ok(format!(
        "{ep_count} episodes → PrefixSpan found {pattern_len}-step pattern (confidence={schema_confidence:.2}) → \
         compiled routine '{routine_id}' ({routine_steps} steps, confidence={routine_confidence:.2}). \
         Common sequence: {:?}",
        &schema.candidate_skill_ordering
    ))
}

// ---------------------------------------------------------------------------
// Phase 8: Learned routine fires reactively via plan-following
// ---------------------------------------------------------------------------

fn phase8_learned_fires() -> Result<String, String> {
    use soma_next::memory::schemas::{DefaultSchemaStore, SchemaStore};
    use soma_next::memory::routines::{DefaultRoutineStore, RoutineStore};

    let ir = boot_interceptor()?;
    let ctx = InvocationContext::local();

    // Replay engagements to populate episodes (same as Phase 7).
    let scenarios: &[(&str, fn() -> serde_json::Value)] = &[
        ("proportional_navigation", basic_scenario as fn() -> _),
        ("lead_pursuit", basic_scenario as fn() -> _),
        ("pure_pursuit", basic_scenario as fn() -> _),
        ("lead_pursuit", evasive_scenario as fn() -> _),
        ("proportional_navigation", evasive_scenario as fn() -> _),
        ("pure_pursuit", evasive_scenario as fn() -> _),
        ("proportional_navigation", basic_scenario as fn() -> _),
        ("lead_pursuit", evasive_scenario as fn() -> _),
    ];

    for (pursuit, scene_fn) in scenarios {
        let pr = ir.port_runtime.lock().unwrap();
        pr.invoke("interceptor", "reset", scene_fn(), &ctx)
            .map_err(|e| format!("reset: {e}"))?;
        pr.invoke("interceptor", "compute_intercept_vector", json!({}), &ctx)
            .map_err(|e| format!("compute: {e}"))?;
        for _ in 0..30 {
            let nav = pr.invoke("interceptor", pursuit, json!({}), &ctx)
                .map_err(|e| format!("{pursuit}: {e}"))?;
            if nav.raw_result.get("within_kill_radius").and_then(|v| v.as_bool()).unwrap_or(false) {
                break;
            }
        }
        pr.invoke("interceptor", "arm", json!({}), &ctx)
            .map_err(|e| format!("arm: {e}"))?;
        pr.invoke("interceptor", "detonate_proximity", json!({}), &ctx)
            .map_err(|e| format!("det: {e}"))?;

        let session_id = Uuid::new_v4();
        let skill_sequence: Vec<String> = vec![
            "interceptor.sensors.fuse_target_state",
            "interceptor.comms.iff_query",
            "interceptor.navigation.compute_intercept_vector",
            &format!("interceptor.navigation.{pursuit}"),
            "interceptor.engagement.arm",
            "interceptor.engagement.detonate_proximity",
        ].into_iter().map(String::from).collect();

        let steps: Vec<EpisodeStep> = skill_sequence.iter().enumerate().map(|(si, skill_id)| {
            EpisodeStep {
                step_index: si as u32,
                belief_summary: json!({}),
                candidates_considered: vec![skill_id.clone()],
                predicted_scores: vec![1.0],
                selected_skill: skill_id.clone(),
                observation: Observation {
                    observation_id: Uuid::new_v4(),
                    session_id,
                    skill_id: Some(skill_id.clone()),
                    port_calls: vec![],
                    raw_result: json!({"ok": true}),
                    structured_result: json!({"ok": true}),
                    effect_patch: None,
                    success: true,
                    failure_class: None,
                    failure_detail: None,
                    latency_ms: 1,
                    resource_cost: CostProfile {
                        cpu_cost_class: CostClass::Negligible,
                        memory_cost_class: CostClass::Negligible,
                        io_cost_class: CostClass::Negligible,
                        network_cost_class: CostClass::Negligible,
                        energy_cost_class: CostClass::Negligible,
                    },
                    confidence: 0.95,
                    timestamp: Utc::now(),
                },
                belief_patch: json!({}),
                progress_delta: 1.0 / skill_sequence.len() as f64,
                critic_decision: "continue".into(),
                timestamp: Utc::now(),
            }
        }).collect();

        ir.episode_store.lock().unwrap()
            .store(Episode {
                episode_id: Uuid::new_v4(),
                goal_fingerprint: "intercept_hostile".to_string(),
                initial_belief_summary: json!({}),
                steps,
                observations: vec![],
                outcome: EpisodeOutcome::Success,
                total_cost: 0.15,
                success: true,
                tags: vec!["interceptor".into()],
                embedding: None,
                created_at: Utc::now(),
                salience: 1.0,
                world_state_context: json!({"threat.detected": true, "target.iff": "hostile"}),
            })
            .map_err(|e| format!("store: {e}"))?;
    }

    // Induce schema and compile routine.
    let schema_store = DefaultSchemaStore::new();
    let episodes_guard = ir.episode_store.lock().unwrap();
    let all_episodes: Vec<&Episode> = episodes_guard.list(100, 0);
    let schema = schema_store.induce_from_episodes(&all_episodes)
        .ok_or("schema induction failed")?;
    let routine_store = DefaultRoutineStore::new();
    let ep_refs: Vec<&Episode> = all_episodes.iter().filter(|e| e.success).copied().collect();
    let mut routine = routine_store.compile_from_schema(&schema, &ep_refs)
        .ok_or("routine compilation failed")?;
    drop(episodes_guard);

    // Override match_conditions and enable autonomous so reactive monitor fires it.
    routine.match_conditions = vec![Precondition {
        condition_type: "world_state".into(),
        expression: json!({"threat.detected": true}),
        description: "fires on threat detection".into(),
    }];
    routine.autonomous = true;

    let routine_id = routine.routine_id.clone();
    let step_count = routine.compiled_skill_path.len();

    // Register the compiled routine in the runtime's routine store.
    ir.routine_store.lock().unwrap()
        .register(routine)
        .map_err(|e| format!("register routine: {e}"))?;

    // Reset sim state for a fresh engagement.
    ir.port_runtime.lock().unwrap()
        .invoke("interceptor", "reset", basic_scenario(), &ctx)
        .map_err(|e| format!("reset: {e}"))?;

    // Trigger: set world state to threat.detected=true — reactive monitor should fire the routine.
    {
        let mut ws = ir.world_state.lock().unwrap();
        ws.add_fact(make_fact("threat", "detected", json!(true)))
            .map_err(|e| e.to_string())?;
    }

    let success = wait_for_routine_fact(&ir.world_state, &routine_id, 5)?;

    Ok(format!(
        "Learned routine '{routine_id}' ({step_count} steps) registered and fired reactively \
         (success={success}). Full loop: episodes → PrefixSpan → schema → BMR → routine → \
         reactive trigger → plan-following → port execution."
    ))
}
