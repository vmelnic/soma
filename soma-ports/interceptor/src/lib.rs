use std::sync::Mutex;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "interceptor";
const DT: f64 = 0.01; // 10ms simulation step

// --- Vec3 ---

#[derive(Clone, Copy, Debug)]
struct Vec3 {
    x: f64,
    y: f64,
    z: f64,
}

impl Vec3 {
    fn new(x: f64, y: f64, z: f64) -> Self { Self { x, y, z } }
    fn zero() -> Self { Self { x: 0.0, y: 0.0, z: 0.0 } }

    fn add(self, other: Self) -> Self {
        Self { x: self.x + other.x, y: self.y + other.y, z: self.z + other.z }
    }
    fn sub(self, other: Self) -> Self {
        Self { x: self.x - other.x, y: self.y - other.y, z: self.z - other.z }
    }
    fn scale(self, s: f64) -> Self {
        Self { x: self.x * s, y: self.y * s, z: self.z * s }
    }
    fn mag(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
    fn normalized(self) -> Self {
        let m = self.mag();
        if m < 1e-10 { Vec3::zero() } else { self.scale(1.0 / m) }
    }
    fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }
    fn distance(self, other: Self) -> f64 {
        self.sub(other).mag()
    }
}

// --- Drone State ---

#[derive(Clone, Debug)]
struct DroneState {
    pos: Vec3,
    vel: Vec3,
    heading_deg: f64,
    speed_ms: f64,
    max_speed_ms: f64,
    altitude_m: f64,
}

impl DroneState {
    fn advance(&mut self, dt: f64) {
        let heading_rad = self.heading_deg.to_radians();
        self.vel = Vec3::new(
            heading_rad.sin() * self.speed_ms,
            heading_rad.cos() * self.speed_ms,
            0.0,
        );
        self.pos = self.pos.add(self.vel.scale(dt));
        self.pos.z = self.altitude_m;
    }
}

// --- Target ---

#[derive(Clone, Debug)]
struct Target {
    id: String,
    kind: String,
    iff: IffStatus,
    state: DroneState,
    behavior: TargetBehavior,
    maneuver_timer: f64,
    maneuver_interval: (f64, f64),
    maneuver_magnitude_deg: f64,
    alive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum IffStatus {
    Unknown,
    Hostile,
    Friendly,
}

#[derive(Clone, Debug)]
enum TargetBehavior {
    StraightLine,
    EvasiveRandom,
}

impl Target {
    fn step(&mut self, dt: f64, rng: &mut Rng) {
        if !self.alive { return; }
        match self.behavior {
            TargetBehavior::StraightLine => {}
            TargetBehavior::EvasiveRandom => {
                self.maneuver_timer -= dt;
                if self.maneuver_timer <= 0.0 {
                    let range = self.maneuver_interval.1 - self.maneuver_interval.0;
                    self.maneuver_timer = self.maneuver_interval.0 + rng.next_f64() * range;
                    let delta = (rng.next_f64() - 0.5) * 2.0 * self.maneuver_magnitude_deg;
                    self.state.heading_deg += delta;
                    self.state.altitude_m += (rng.next_f64() - 0.5) * 20.0;
                    self.state.altitude_m = self.state.altitude_m.max(20.0);
                }
            }
        }
        self.state.advance(dt);
    }
}

// --- Simulation ---

#[derive(Clone, Debug)]
struct Simulation {
    interceptor: DroneState,
    targets: Vec<Target>,
    armed: bool,
    engaged: bool,
    engagement_target: Option<usize>,
    sensor_range_m: f64,
    warhead_radius_m: f64,
    geofence_center: Vec3,
    geofence_radius_m: f64,
    min_altitude_m: f64,
    time_s: f64,
    max_time_s: f64,
    outcome: SimOutcome,
    kills: Vec<String>,
    episodes: Vec<serde_json::Value>,
    rng: Rng,
}

#[derive(Clone, Debug, PartialEq)]
enum SimOutcome {
    InProgress,
    Kill,
    Miss,
    Abort,
    Timeout,
}

impl Simulation {
    fn from_scenario(input: &serde_json::Value) -> Self {
        let int_cfg = input.get("interceptor").unwrap_or(input);
        let interceptor = DroneState {
            pos: parse_pos(int_cfg.get("start_position")),
            vel: Vec3::zero(),
            heading_deg: int_cfg.get("start_heading_deg").and_then(|v| v.as_f64()).unwrap_or(0.0),
            speed_ms: int_cfg.get("speed_ms").and_then(|v| v.as_f64()).unwrap_or(30.0),
            max_speed_ms: int_cfg.get("max_speed_ms").and_then(|v| v.as_f64()).unwrap_or(50.0),
            altitude_m: int_cfg.get("start_position")
                .and_then(|p| p.get("alt_m")).and_then(|v| v.as_f64()).unwrap_or(100.0),
        };

        let targets: Vec<Target> = input.get("targets")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(|t| parse_target(t)).collect())
            .unwrap_or_default();

        let geofence_center = input.get("geofence")
            .and_then(|g| g.get("center"))
            .map(|c| parse_pos(Some(c)))
            .unwrap_or(interceptor.pos);
        let geofence_radius = input.get("geofence")
            .and_then(|g| g.get("radius_m")).and_then(|v| v.as_f64()).unwrap_or(5000.0);

        let max_time = input.get("success_criteria")
            .and_then(|s| s.get("max_time_secs")).and_then(|v| v.as_f64()).unwrap_or(120.0);

        let sensor_range = int_cfg.get("sensor_range_m").and_then(|v| v.as_f64()).unwrap_or(1500.0);
        let warhead_radius = int_cfg.get("warhead_radius_m").and_then(|v| v.as_f64()).unwrap_or(5.0);

        let seed = input.get("seed").and_then(|v| v.as_u64()).unwrap_or(42);

        Simulation {
            interceptor,
            targets,
            armed: false,
            engaged: false,
            engagement_target: None,
            sensor_range_m: sensor_range,
            warhead_radius_m: warhead_radius,
            geofence_center,
            geofence_radius_m: geofence_radius,
            min_altitude_m: 10.0,
            time_s: 0.0,
            max_time_s: max_time,
            outcome: SimOutcome::InProgress,
            kills: Vec::new(),
            episodes: Vec::new(),
            rng: Rng(seed),
        }
    }

    fn step_physics(&mut self) {
        self.interceptor.advance(DT);
        for target in &mut self.targets {
            target.step(DT, &mut self.rng.clone());
        }
        self.rng.advance();
        self.time_s += DT;

        if self.time_s >= self.max_time_s && self.outcome == SimOutcome::InProgress {
            self.outcome = SimOutcome::Timeout;
        }
    }

    fn observation(&self) -> serde_json::Value {
        let detected_targets: Vec<serde_json::Value> = self.targets.iter()
            .filter(|t| t.alive && self.interceptor.pos.distance(t.state.pos) <= self.sensor_range_m)
            .map(|t| serde_json::json!({
                "id": t.id,
                "kind": t.kind,
                "iff": match t.iff { IffStatus::Hostile => "hostile", IffStatus::Friendly => "friendly", IffStatus::Unknown => "unknown" },
                "pos": [t.state.pos.x, t.state.pos.y, t.state.pos.z],
                "vel": [t.state.vel.x, t.state.vel.y, t.state.vel.z],
                "distance_m": self.interceptor.pos.distance(t.state.pos),
                "bearing_deg": bearing(self.interceptor.pos, t.state.pos),
            }))
            .collect();

        serde_json::json!({
            "time_s": self.time_s,
            "interceptor": {
                "pos": [self.interceptor.pos.x, self.interceptor.pos.y, self.interceptor.pos.z],
                "vel": [self.interceptor.vel.x, self.interceptor.vel.y, self.interceptor.vel.z],
                "heading_deg": self.interceptor.heading_deg,
                "speed_ms": self.interceptor.speed_ms,
                "altitude_m": self.interceptor.altitude_m,
                "armed": self.armed,
                "engaged": self.engaged,
            },
            "targets_detected": detected_targets,
            "target_count": self.targets.iter().filter(|t| t.alive).count(),
            "kills": self.kills,
            "outcome": match self.outcome {
                SimOutcome::InProgress => "in_progress",
                SimOutcome::Kill => "kill",
                SimOutcome::Miss => "miss",
                SimOutcome::Abort => "abort",
                SimOutcome::Timeout => "timeout",
            },
            "geofence_margin_m": self.geofence_radius_m - self.interceptor.pos.distance(self.geofence_center),
            "in_geofence": self.interceptor.pos.distance(self.geofence_center) <= self.geofence_radius_m,
        })
    }

    fn closest_hostile(&self) -> Option<usize> {
        self.targets.iter().enumerate()
            .filter(|(_, t)| t.alive && t.iff == IffStatus::Hostile)
            .min_by(|(_, a), (_, b)| {
                let da = self.interceptor.pos.distance(a.state.pos);
                let db = self.interceptor.pos.distance(b.state.pos);
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, _)| i)
    }

    fn closest_any(&self) -> Option<usize> {
        self.targets.iter().enumerate()
            .filter(|(_, t)| t.alive && self.interceptor.pos.distance(t.state.pos) <= self.sensor_range_m)
            .min_by(|(_, a), (_, b)| {
                let da = self.interceptor.pos.distance(a.state.pos);
                let db = self.interceptor.pos.distance(b.state.pos);
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, _)| i)
    }
}

fn parse_pos(val: Option<&serde_json::Value>) -> Vec3 {
    match val {
        Some(v) => {
            if let Some(arr) = v.as_array() {
                Vec3::new(
                    arr.first().and_then(|x| x.as_f64()).unwrap_or(0.0),
                    arr.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0),
                    arr.get(2).and_then(|x| x.as_f64()).unwrap_or(100.0),
                )
            } else {
                let lat = v.get("lat").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let lon = v.get("lon").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let alt = v.get("alt_m").and_then(|x| x.as_f64()).unwrap_or(100.0);
                // Convert lat/lon to local meters (approximate, 1 deg ≈ 111km)
                Vec3::new(lon * 111_000.0, lat * 111_000.0, alt)
            }
        }
        None => Vec3::new(0.0, 0.0, 100.0),
    }
}

fn parse_target(val: &serde_json::Value) -> Target {
    let pos = parse_pos(val.get("start_position"));
    let heading = val.get("start_heading_deg").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let speed = val.get("speed_ms").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let behavior = match val.get("behavior").and_then(|v| v.as_str()).unwrap_or("straight_line") {
        "evasive_random" => TargetBehavior::EvasiveRandom,
        _ => TargetBehavior::StraightLine,
    };
    let iff = match val.get("iff").and_then(|v| v.as_str()).unwrap_or("unknown") {
        "hostile" => IffStatus::Hostile,
        "friendly" => IffStatus::Friendly,
        _ => IffStatus::Unknown,
    };
    let interval = val.get("maneuver_interval_secs")
        .and_then(|v| v.as_array())
        .map(|a| (
            a.first().and_then(|x| x.as_f64()).unwrap_or(3.0),
            a.get(1).and_then(|x| x.as_f64()).unwrap_or(5.0),
        ))
        .unwrap_or((3.0, 5.0));
    let magnitude = val.get("maneuver_magnitude_deg").and_then(|v| v.as_f64()).unwrap_or(30.0);

    Target {
        id: val.get("target_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
        kind: val.get("type").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
        iff,
        state: DroneState {
            pos,
            vel: Vec3::zero(),
            heading_deg: heading,
            speed_ms: speed,
            max_speed_ms: speed,
            altitude_m: pos.z,
        },
        behavior,
        maneuver_timer: interval.0,
        maneuver_interval: interval,
        maneuver_magnitude_deg: magnitude,
        alive: true,
    }
}

fn bearing(from: Vec3, to: Vec3) -> f64 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    dx.atan2(dy).to_degrees()
}

// --- RNG ---

#[derive(Clone, Debug)]
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() % 10000) as f64 / 10000.0
    }
    fn advance(&mut self) {
        self.next_u64();
    }
}

// --- Port ---

pub struct InterceptorPort {
    spec: PortSpec,
    sim: Mutex<Option<Simulation>>,
}

impl InterceptorPort {
    pub fn new() -> Self {
        Self { spec: build_spec(), sim: Mutex::new(None) }
    }
}

impl Default for InterceptorPort {
    fn default() -> Self { Self::new() }
}

impl Port for InterceptorPort {
    fn spec(&self) -> &PortSpec { &self.spec }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            // Sensors
            "rf_scan" => self.do_rf_scan(),
            "visual_detect" => self.do_visual_detect(),
            "ir_detect" => self.do_ir_detect(),
            "acoustic_detect" => self.do_acoustic_detect(),
            "imu_read" => self.do_imu_read(),
            "gps_read" => self.do_gps_read(),
            "fuse_target_state" => self.do_fuse_target_state(),
            // Navigation
            "set_heading" => self.do_set_heading(&input),
            "set_throttle" => self.do_set_throttle(&input),
            "set_altitude" => self.do_set_altitude(&input),
            "set_waypoint" => self.do_set_waypoint(&input),
            "compute_intercept_vector" => self.do_compute_intercept_vector(),
            "proportional_navigation" => self.do_proportional_navigation(),
            "lead_pursuit" => self.do_lead_pursuit(),
            "pure_pursuit" => self.do_pure_pursuit(),
            // Engagement
            "arm" => self.do_arm(),
            "disarm" => self.do_disarm(),
            "detonate_proximity" => self.do_detonate_proximity(),
            "abort_engagement" => self.do_abort_engagement(),
            "report_kill" => self.do_report_kill(),
            "report_miss" => self.do_report_miss(),
            // Comms
            "beacon_status" => self.do_beacon_status(&input),
            "receive_tasking" => self.do_receive_tasking(),
            "share_target" => self.do_share_target(),
            "upload_episode" => self.do_upload_episode(),
            "download_routines" => self.do_download_routines(),
            "iff_query" => self.do_iff_query(),
            // Simulation control
            "reset" => self.do_reset(&input),
            "step" => self.do_step(&input),
            other => return Err(PortError::Validation(format!("unknown capability: {other}"))),
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        match result {
            Ok(value) => Ok(PortCallRecord::success(PORT_ID, capability_id, value, latency_ms)),
            Err(e) => Ok(PortCallRecord::failure(PORT_ID, capability_id, e.failure_class(), &e.to_string(), latency_ms)),
        }
    }

    fn validate_input(&self, capability_id: &str, _input: &serde_json::Value) -> soma_port_sdk::Result<()> {
        let known = [
            "rf_scan", "visual_detect", "ir_detect", "acoustic_detect", "imu_read", "gps_read",
            "fuse_target_state", "set_heading", "set_throttle", "set_altitude", "set_waypoint",
            "compute_intercept_vector", "proportional_navigation", "lead_pursuit", "pure_pursuit",
            "arm", "disarm", "detonate_proximity", "abort_engagement", "report_kill", "report_miss",
            "beacon_status", "receive_tasking", "share_target", "upload_episode", "download_routines",
            "iff_query", "reset", "step",
        ];
        if known.contains(&capability_id) { Ok(()) }
        else { Err(PortError::Validation(format!("unknown capability: {capability_id}"))) }
    }

    fn lifecycle_state(&self) -> PortLifecycleState { PortLifecycleState::Active }
}

// --- Capability Implementations ---

impl InterceptorPort {
    fn with_sim<F, T>(&self, f: F) -> soma_port_sdk::Result<T>
    where F: FnOnce(&mut Simulation) -> soma_port_sdk::Result<T> {
        let mut lock = self.sim.lock().unwrap();
        match lock.as_mut() {
            Some(sim) => f(sim),
            None => Err(PortError::ExternalError("no active simulation — call reset first".into())),
        }
    }

    // --- Simulation Control ---

    fn do_reset(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let sim = Simulation::from_scenario(input);
        let obs = sim.observation();
        *self.sim.lock().unwrap() = Some(sim);
        Ok(obs)
    }

    fn do_step(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let steps = input.get("steps").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        self.with_sim(|sim| {
            for _ in 0..steps.min(1000) {
                sim.step_physics();
            }
            Ok(sim.observation())
        })
    }

    // --- Sensors ---

    fn do_rf_scan(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let detections: Vec<serde_json::Value> = sim.targets.iter()
                .filter(|t| t.alive)
                .filter(|t| sim.interceptor.pos.distance(t.state.pos) <= sim.sensor_range_m)
                .map(|t| serde_json::json!({
                    "id": t.id,
                    "signal_strength_dbm": -40.0 - sim.interceptor.pos.distance(t.state.pos) * 0.05,
                    "frequency_ghz": 2.4,
                    "bearing_deg": bearing(sim.interceptor.pos, t.state.pos),
                    "distance_m": sim.interceptor.pos.distance(t.state.pos),
                }))
                .collect();
            Ok(serde_json::json!({ "detections": detections, "count": detections.len() }))
        })
    }

    fn do_visual_detect(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let detections: Vec<serde_json::Value> = sim.targets.iter()
                .filter(|t| t.alive)
                .filter(|t| sim.interceptor.pos.distance(t.state.pos) <= sim.sensor_range_m * 0.7)
                .map(|t| serde_json::json!({
                    "id": t.id,
                    "type": t.kind,
                    "pixel_size": 500.0 / sim.interceptor.pos.distance(t.state.pos).max(1.0),
                    "bearing_deg": bearing(sim.interceptor.pos, t.state.pos),
                    "elevation_deg": elevation(sim.interceptor.pos, t.state.pos),
                }))
                .collect();
            Ok(serde_json::json!({ "detections": detections, "count": detections.len() }))
        })
    }

    fn do_ir_detect(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let detections: Vec<serde_json::Value> = sim.targets.iter()
                .filter(|t| t.alive)
                .filter(|t| sim.interceptor.pos.distance(t.state.pos) <= sim.sensor_range_m * 0.5)
                .map(|t| serde_json::json!({
                    "id": t.id,
                    "thermal_signature": 0.8,
                    "bearing_deg": bearing(sim.interceptor.pos, t.state.pos),
                }))
                .collect();
            Ok(serde_json::json!({ "detections": detections, "count": detections.len() }))
        })
    }

    fn do_acoustic_detect(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let detections: Vec<serde_json::Value> = sim.targets.iter()
                .filter(|t| t.alive)
                .filter(|t| sim.interceptor.pos.distance(t.state.pos) <= 500.0)
                .map(|t| serde_json::json!({
                    "id": t.id,
                    "frequency_hz": 8000.0,
                    "amplitude_db": 60.0 - sim.interceptor.pos.distance(t.state.pos) * 0.08,
                    "bearing_deg": bearing(sim.interceptor.pos, t.state.pos),
                }))
                .collect();
            Ok(serde_json::json!({ "detections": detections, "count": detections.len() }))
        })
    }

    fn do_imu_read(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            Ok(serde_json::json!({
                "heading_deg": sim.interceptor.heading_deg,
                "speed_ms": sim.interceptor.speed_ms,
                "altitude_m": sim.interceptor.altitude_m,
                "acceleration_g": [0.0, 0.0, 1.0],
            }))
        })
    }

    fn do_gps_read(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            Ok(serde_json::json!({
                "pos": [sim.interceptor.pos.x, sim.interceptor.pos.y, sim.interceptor.pos.z],
                "vel": [sim.interceptor.vel.x, sim.interceptor.vel.y, sim.interceptor.vel.z],
                "fix_quality": 3,
                "satellites": 12,
            }))
        })
    }

    fn do_fuse_target_state(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let targets: Vec<serde_json::Value> = sim.targets.iter()
                .filter(|t| t.alive && sim.interceptor.pos.distance(t.state.pos) <= sim.sensor_range_m)
                .map(|t| {
                    let dist = sim.interceptor.pos.distance(t.state.pos);
                    let closing = sim.interceptor.vel.sub(t.state.vel).dot(
                        t.state.pos.sub(sim.interceptor.pos).normalized()
                    );
                    serde_json::json!({
                        "id": t.id,
                        "kind": t.kind,
                        "iff": match t.iff { IffStatus::Hostile => "hostile", IffStatus::Friendly => "friendly", _ => "unknown" },
                        "pos": [t.state.pos.x, t.state.pos.y, t.state.pos.z],
                        "vel": [t.state.vel.x, t.state.vel.y, t.state.vel.z],
                        "distance_m": dist,
                        "closing_rate_ms": closing,
                        "time_to_intercept_s": if closing > 0.0 { dist / closing } else { f64::INFINITY },
                        "bearing_deg": bearing(sim.interceptor.pos, t.state.pos),
                        "confidence": 0.95,
                    })
                })
                .collect();
            Ok(serde_json::json!({ "targets": targets, "fused": true, "time_s": sim.time_s }))
        })
    }

    // --- Navigation ---

    fn do_set_heading(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            if let Some(heading) = input.get("heading").and_then(|v| v.as_f64()) {
                sim.interceptor.heading_deg = heading;
            }
            sim.step_physics();
            Ok(sim.observation())
        })
    }

    fn do_set_throttle(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let pct = input.get("throttle").and_then(|v| v.as_f64()).unwrap_or(100.0);
            sim.interceptor.speed_ms = sim.interceptor.max_speed_ms * (pct / 100.0);
            sim.step_physics();
            Ok(sim.observation())
        })
    }

    fn do_set_altitude(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            if let Some(alt) = input.get("altitude_m").and_then(|v| v.as_f64()) {
                sim.interceptor.altitude_m = alt.max(sim.min_altitude_m);
            } else if let Some(delta) = input.get("delta").and_then(|v| v.as_f64()) {
                sim.interceptor.altitude_m = (sim.interceptor.altitude_m + delta).max(sim.min_altitude_m);
            }
            sim.step_physics();
            Ok(sim.observation())
        })
    }

    fn do_set_waypoint(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            // In sim: just point toward waypoint
            if let Some(wp) = input.get("waypoint").and_then(|v| v.as_str()) {
                match wp {
                    "loiter_point" | "rtb" => {
                        let target = sim.geofence_center;
                        sim.interceptor.heading_deg = bearing(sim.interceptor.pos, target);
                        sim.engaged = false;
                        sim.engagement_target = None;
                    }
                    _ => {}
                }
            }
            sim.step_physics();
            Ok(sim.observation())
        })
    }

    fn do_compute_intercept_vector(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let target_idx = sim.engagement_target.or_else(|| sim.closest_hostile());
            match target_idx {
                Some(idx) => {
                    let target = &sim.targets[idx];
                    let rel = target.state.pos.sub(sim.interceptor.pos);
                    let dist = rel.mag();
                    let time_to_intercept = dist / (sim.interceptor.speed_ms + target.state.speed_ms);
                    let predicted_pos = target.state.pos.add(target.state.vel.scale(time_to_intercept));
                    let intercept_heading = bearing(sim.interceptor.pos, predicted_pos);

                    sim.engagement_target = Some(idx);
                    sim.engaged = true;

                    Ok(serde_json::json!({
                        "target_id": target.id,
                        "intercept_heading_deg": intercept_heading,
                        "distance_m": dist,
                        "time_to_intercept_s": time_to_intercept,
                        "predicted_pos": [predicted_pos.x, predicted_pos.y, predicted_pos.z],
                        "computed": true,
                    }))
                }
                None => Ok(serde_json::json!({ "computed": false, "reason": "no_target" })),
            }
        })
    }

    fn do_proportional_navigation(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let target_idx = sim.engagement_target.or_else(|| sim.closest_hostile());
            match target_idx {
                Some(idx) if sim.targets[idx].alive => {
                    let n = 3.0;
                    let target = &sim.targets[idx];
                    let los = target.state.pos.sub(sim.interceptor.pos);
                    let closing_vel = sim.interceptor.speed_ms + target.state.speed_ms * 0.5;

                    sim.interceptor.heading_deg = los.x.atan2(los.y).to_degrees();
                    sim.interceptor.altitude_m = target.state.altitude_m;
                    sim.interceptor.speed_ms = sim.interceptor.max_speed_ms;
                    sim.engaged = true;

                    let steps = 100;
                    let mut min_dist = f64::INFINITY;
                    for _ in 0..steps {
                        sim.step_physics();
                        let tgt = &sim.targets[idx];
                        let los_new = tgt.state.pos.sub(sim.interceptor.pos);
                        sim.interceptor.heading_deg = los_new.x.atan2(los_new.y).to_degrees();
                        sim.interceptor.altitude_m = tgt.state.altitude_m;
                        let d = sim.interceptor.pos.distance(tgt.state.pos);
                        min_dist = min_dist.min(d);
                        if d <= sim.warhead_radius_m { break; }
                    }

                    let dist = sim.interceptor.pos.distance(sim.targets[idx].state.pos);
                    Ok(serde_json::json!({
                        "pursuit_mode": "proportional_navigation",
                        "nav_constant": n,
                        "distance_m": dist,
                        "min_distance_m": min_dist,
                        "closing_rate_ms": closing_vel,
                        "time_s": sim.time_s,
                        "within_kill_radius": dist <= sim.warhead_radius_m,
                    }))
                }
                _ => Ok(serde_json::json!({ "pursuit_mode": "proportional_navigation", "error": "no_target" })),
            }
        })
    }

    fn do_lead_pursuit(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let target_idx = sim.engagement_target.or_else(|| sim.closest_hostile());
            match target_idx {
                Some(idx) if sim.targets[idx].alive => {
                    let target = &sim.targets[idx];
                    let dist = sim.interceptor.pos.distance(target.state.pos);
                    let time_to_intercept = dist / sim.interceptor.max_speed_ms;
                    let predicted = target.state.pos.add(target.state.vel.scale(time_to_intercept));

                    sim.interceptor.heading_deg = bearing(sim.interceptor.pos, predicted);
                    sim.interceptor.altitude_m = predicted.z;
                    sim.interceptor.speed_ms = sim.interceptor.max_speed_ms;
                    sim.engaged = true;

                    let steps = 100;
                    let mut min_dist = f64::INFINITY;
                    for _ in 0..steps {
                        sim.step_physics();
                        let t = &sim.targets[idx];
                        let d_now = sim.interceptor.pos.distance(t.state.pos);
                        let tti = d_now / sim.interceptor.max_speed_ms;
                        let pred = t.state.pos.add(t.state.vel.scale(tti));
                        sim.interceptor.heading_deg = bearing(sim.interceptor.pos, pred);
                        sim.interceptor.altitude_m = pred.z;
                        min_dist = min_dist.min(d_now);
                        if d_now <= sim.warhead_radius_m { break; }
                    }

                    let dist_now = sim.interceptor.pos.distance(sim.targets[idx].state.pos);
                    Ok(serde_json::json!({
                        "pursuit_mode": "lead_pursuit",
                        "distance_m": dist_now,
                        "min_distance_m": min_dist,
                        "time_s": sim.time_s,
                        "within_kill_radius": dist_now <= sim.warhead_radius_m,
                    }))
                }
                _ => Ok(serde_json::json!({ "pursuit_mode": "lead_pursuit", "error": "no_target" })),
            }
        })
    }

    fn do_pure_pursuit(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let target_idx = sim.engagement_target.or_else(|| sim.closest_hostile());
            match target_idx {
                Some(idx) if sim.targets[idx].alive => {
                    sim.interceptor.speed_ms = sim.interceptor.max_speed_ms;
                    sim.engaged = true;

                    let steps = 100;
                    let mut min_dist = f64::INFINITY;
                    for _ in 0..steps {
                        let tgt = &sim.targets[idx];
                        sim.interceptor.heading_deg = bearing(sim.interceptor.pos, tgt.state.pos);
                        sim.interceptor.altitude_m = tgt.state.altitude_m;
                        sim.step_physics();
                        let d = sim.interceptor.pos.distance(sim.targets[idx].state.pos);
                        min_dist = min_dist.min(d);
                        if d <= sim.warhead_radius_m { break; }
                    }

                    let dist_now = sim.interceptor.pos.distance(sim.targets[idx].state.pos);
                    Ok(serde_json::json!({
                        "pursuit_mode": "pure_pursuit",
                        "distance_m": dist_now,
                        "min_distance_m": min_dist,
                        "time_s": sim.time_s,
                        "within_kill_radius": dist_now <= sim.warhead_radius_m,
                    }))
                }
                _ => Ok(serde_json::json!({ "pursuit_mode": "pure_pursuit", "error": "no_target" })),
            }
        })
    }

    // --- Engagement ---

    fn do_arm(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            sim.armed = true;
            Ok(serde_json::json!({ "armed": true, "time_s": sim.time_s }))
        })
    }

    fn do_disarm(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            sim.armed = false;
            Ok(serde_json::json!({ "armed": false, "time_s": sim.time_s }))
        })
    }

    fn do_detonate_proximity(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            if !sim.armed {
                return Ok(serde_json::json!({ "detonated": false, "reason": "not_armed" }));
            }

            let target_idx = sim.engagement_target.or_else(|| sim.closest_hostile());
            match target_idx {
                Some(idx) => {
                    let dist = sim.interceptor.pos.distance(sim.targets[idx].state.pos);
                    if dist <= sim.warhead_radius_m {
                        sim.targets[idx].alive = false;
                        sim.kills.push(sim.targets[idx].id.clone());
                        sim.outcome = SimOutcome::Kill;
                        sim.armed = false;
                        Ok(serde_json::json!({
                            "detonated": true,
                            "kill": true,
                            "target_id": sim.targets[idx].id,
                            "distance_m": dist,
                            "time_s": sim.time_s,
                        }))
                    } else {
                        sim.outcome = SimOutcome::Miss;
                        sim.armed = false;
                        Ok(serde_json::json!({
                            "detonated": true,
                            "kill": false,
                            "miss_distance_m": dist,
                            "target_id": sim.targets[idx].id,
                            "time_s": sim.time_s,
                        }))
                    }
                }
                None => {
                    sim.armed = false;
                    Ok(serde_json::json!({ "detonated": true, "kill": false, "reason": "no_target" }))
                }
            }
        })
    }

    fn do_abort_engagement(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            sim.engaged = false;
            sim.armed = false;
            sim.engagement_target = None;
            sim.outcome = SimOutcome::Abort;
            Ok(serde_json::json!({ "aborted": true, "time_s": sim.time_s }))
        })
    }

    fn do_report_kill(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            Ok(serde_json::json!({ "kills": sim.kills, "time_s": sim.time_s }))
        })
    }

    fn do_report_miss(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            Ok(serde_json::json!({
                "outcome": "miss",
                "time_s": sim.time_s,
                "engagement_target": sim.engagement_target.map(|i| sim.targets[i].id.clone()),
            }))
        })
    }

    // --- Comms ---

    fn do_beacon_status(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let state = input.get("state").and_then(|v| v.as_str()).unwrap_or("loiter");
            Ok(serde_json::json!({
                "beacon_state": state,
                "pos": [sim.interceptor.pos.x, sim.interceptor.pos.y, sim.interceptor.pos.z],
                "armed": sim.armed,
                "engaged": sim.engaged,
                "kills": sim.kills.len(),
                "time_s": sim.time_s,
            }))
        })
    }

    fn do_receive_tasking(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            Ok(serde_json::json!({
                "tasking": "patrol",
                "area": { "center": [sim.geofence_center.x, sim.geofence_center.y], "radius_m": sim.geofence_radius_m },
                "time_s": sim.time_s,
            }))
        })
    }

    fn do_share_target(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let targets: Vec<serde_json::Value> = sim.targets.iter()
                .filter(|t| t.alive && sim.interceptor.pos.distance(t.state.pos) <= sim.sensor_range_m)
                .map(|t| serde_json::json!({
                    "id": t.id,
                    "pos": [t.state.pos.x, t.state.pos.y, t.state.pos.z],
                    "iff": match t.iff { IffStatus::Hostile => "hostile", IffStatus::Friendly => "friendly", _ => "unknown" },
                }))
                .collect();
            Ok(serde_json::json!({ "shared_targets": targets, "count": targets.len() }))
        })
    }

    fn do_upload_episode(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            Ok(serde_json::json!({
                "uploaded": true,
                "episode_count": sim.episodes.len(),
                "kills": sim.kills.len(),
                "outcome": match sim.outcome {
                    SimOutcome::Kill => "kill",
                    SimOutcome::Miss => "miss",
                    SimOutcome::Abort => "abort",
                    SimOutcome::Timeout => "timeout",
                    SimOutcome::InProgress => "in_progress",
                },
            }))
        })
    }

    fn do_download_routines(&self) -> soma_port_sdk::Result<serde_json::Value> {
        Ok(serde_json::json!({ "routines_available": 0, "message": "no new routines from ground station" }))
    }

    fn do_iff_query(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_sim(|sim| {
            let target_idx = sim.engagement_target.or_else(|| sim.closest_any());
            match target_idx {
                Some(idx) => {
                    let target = &sim.targets[idx];
                    Ok(serde_json::json!({
                        "target_id": target.id,
                        "iff_result": match target.iff {
                            IffStatus::Hostile => "hostile",
                            IffStatus::Friendly => "friendly",
                            IffStatus::Unknown => "unknown",
                        },
                        "confidence": 0.95,
                        "method": "rf_fingerprint",
                    }))
                }
                None => Ok(serde_json::json!({ "iff_result": "no_target", "confidence": 0.0 })),
            }
        })
    }
}

fn elevation(from: Vec3, to: Vec3) -> f64 {
    let dz = to.z - from.z;
    let horiz = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
    dz.atan2(horiz).to_degrees()
}

// --- Spec ---

fn cap(id: &str, purpose: &str, effect: SideEffectClass) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.into(), name: id.into(), purpose: purpose.into(),
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        effect_class: effect, rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic, idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
        cost_profile: CostProfile::default(), remote_exposable: false, auth_override: None,
    }
}

fn build_spec() -> PortSpec {
    let m = SideEffectClass::LocalStateMutation;
    let r = SideEffectClass::ReadOnly;
    PortSpec {
        port_id: PORT_ID.into(), name: "Interceptor".into(),
        version: semver::Version::new(0, 1, 0), kind: PortKind::Custom,
        description: "Drone interceptor simulation: 6DOF flight, multi-sensor fusion, engagement geometry".into(),
        namespace: "interceptor".into(), trust_level: TrustLevel::BuiltIn,
        capabilities: vec![
            // Sensors
            cap("rf_scan", "Scan for RF emissions from drone controllers", r),
            cap("visual_detect", "Visual target detection via camera", r),
            cap("ir_detect", "Infrared/thermal target detection", r),
            cap("acoustic_detect", "Acoustic propeller signature detection", r),
            cap("imu_read", "Read IMU (heading, speed, acceleration)", r),
            cap("gps_read", "Read GPS position and velocity", r),
            cap("fuse_target_state", "Multi-sensor fusion for target state estimation", r),
            // Navigation
            cap("set_heading", "Set interceptor heading in degrees", m),
            cap("set_throttle", "Set throttle percentage (0-100)", m),
            cap("set_altitude", "Set target altitude or altitude delta", m),
            cap("set_waypoint", "Navigate to named waypoint", m),
            cap("compute_intercept_vector", "Compute optimal intercept geometry", r),
            cap("proportional_navigation", "Execute proportional navigation pursuit", m),
            cap("lead_pursuit", "Execute lead pursuit with target prediction", m),
            cap("pure_pursuit", "Execute pure pursuit (point at target)", m),
            // Engagement
            cap("arm", "Arm warhead for detonation", m),
            cap("disarm", "Safe warhead", m),
            cap("detonate_proximity", "Trigger proximity detonation", m),
            cap("abort_engagement", "Abort current engagement, return to loiter", m),
            cap("report_kill", "Report successful engagement", r),
            cap("report_miss", "Report missed engagement", r),
            // Comms
            cap("beacon_status", "Broadcast status beacon to ground/swarm", r),
            cap("receive_tasking", "Receive tasking orders from ground", r),
            cap("share_target", "Share target detection with swarm peers", r),
            cap("upload_episode", "Upload engagement episode to ground station", m),
            cap("download_routines", "Download compiled routines from ground station", r),
            cap("iff_query", "Query IFF (Identify Friend or Foe) for target", r),
            // Sim control
            cap("reset", "Initialize simulation from scenario config", m),
            cap("step", "Advance simulation by N physics steps", m),
        ],
        input_schema: SchemaRef::any(), output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: m,
        latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec![],
        validation_rules: vec![], remote_exposure: false,
    }
}

// --- C ABI ---

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(InterceptorPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_intercept_scenario() {
        let port = InterceptorPort::new();

        // Reset with basic scenario
        let r = port.invoke("reset", serde_json::json!({
            "interceptor": {
                "start_position": [0.0, 0.0, 100.0],
                "start_heading_deg": 0.0,
                "speed_ms": 40.0,
                "max_speed_ms": 50.0,
                "sensor_range_m": 1500.0,
                "warhead_radius_m": 5.0,
            },
            "targets": [{
                "target_id": "hostile_1",
                "type": "fixed_wing",
                "iff": "hostile",
                "start_position": [0.0, 800.0, 100.0],
                "start_heading_deg": 180.0,
                "speed_ms": 20.0,
                "behavior": "straight_line",
            }],
            "geofence": { "center": [0.0, 400.0, 0.0], "radius_m": 3000.0 },
            "success_criteria": { "max_time_secs": 60.0 },
        })).unwrap();
        assert!(r.success);

        // Fuse target state
        let r = port.invoke("fuse_target_state", serde_json::json!({})).unwrap();
        assert!(r.success);
        let targets = r.structured_result["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0]["iff"], "hostile");

        // IFF query
        let r = port.invoke("iff_query", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert_eq!(r.structured_result["iff_result"], "hostile");

        // Compute intercept vector
        let r = port.invoke("compute_intercept_vector", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["computed"].as_bool().unwrap());

        // Execute proportional navigation
        let r = port.invoke("proportional_navigation", serde_json::json!({})).unwrap();
        assert!(r.success);

        // Arm
        let r = port.invoke("arm", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["armed"].as_bool().unwrap());

        // Detonate
        let r = port.invoke("detonate_proximity", serde_json::json!({})).unwrap();
        assert!(r.success);
        // May or may not kill depending on pursuit convergence
    }

    #[test]
    fn abort_on_friendly() {
        let port = InterceptorPort::new();

        port.invoke("reset", serde_json::json!({
            "interceptor": { "start_position": [0.0, 0.0, 100.0], "speed_ms": 30.0, "max_speed_ms": 50.0, "sensor_range_m": 1500.0, "warhead_radius_m": 5.0 },
            "targets": [{ "target_id": "friend_1", "iff": "friendly", "start_position": [0.0, 500.0, 100.0], "speed_ms": 10.0, "behavior": "straight_line" }],
        })).unwrap();

        let r = port.invoke("iff_query", serde_json::json!({})).unwrap();
        assert_eq!(r.structured_result["iff_result"], "friendly");

        // Disarm and abort
        let r = port.invoke("disarm", serde_json::json!({})).unwrap();
        assert!(!r.structured_result["armed"].as_bool().unwrap());

        let r = port.invoke("abort_engagement", serde_json::json!({})).unwrap();
        assert!(r.structured_result["aborted"].as_bool().unwrap());
    }

    #[test]
    fn sensor_range_limits() {
        let port = InterceptorPort::new();

        port.invoke("reset", serde_json::json!({
            "interceptor": { "start_position": [0.0, 0.0, 100.0], "speed_ms": 30.0, "max_speed_ms": 50.0, "sensor_range_m": 500.0, "warhead_radius_m": 5.0 },
            "targets": [{ "target_id": "far_target", "iff": "hostile", "start_position": [0.0, 2000.0, 100.0], "speed_ms": 20.0, "behavior": "straight_line" }],
        })).unwrap();

        // Target out of range
        let r = port.invoke("rf_scan", serde_json::json!({})).unwrap();
        assert_eq!(r.structured_result["count"], 0);

        let r = port.invoke("fuse_target_state", serde_json::json!({})).unwrap();
        assert!(r.structured_result["targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn cannot_detonate_unarmed() {
        let port = InterceptorPort::new();

        port.invoke("reset", serde_json::json!({
            "interceptor": { "start_position": [0.0, 0.0, 100.0], "speed_ms": 30.0, "max_speed_ms": 50.0, "sensor_range_m": 1500.0, "warhead_radius_m": 5.0 },
            "targets": [{ "target_id": "h1", "iff": "hostile", "start_position": [0.0, 3.0, 100.0], "speed_ms": 0.0, "behavior": "straight_line" }],
        })).unwrap();

        let r = port.invoke("detonate_proximity", serde_json::json!({})).unwrap();
        assert!(!r.structured_result["detonated"].as_bool().unwrap_or(true));
        assert_eq!(r.structured_result["reason"], "not_armed");
    }
}
