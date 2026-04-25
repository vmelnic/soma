use std::sync::Mutex;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "kitchen";

const COUNTER_CENTER: [f64; 2] = [300.0, 200.0];
const CABINET_POS: [f64; 2] = [80.0, 340.0];
const SHELF_POS: [f64; 2] = [80.0, 280.0];
const DRAWER_POS: [f64; 2] = [240.0, 370.0];
const WINDOW_POS: [f64; 2] = [350.0, 30.0];
const PROCESSOR_POS: [f64; 2] = [500.0, 340.0];
const BLOCK_POS: [f64; 2] = [540.0, 200.0];
const BOARD_TARGET: [f64; 2] = [300.0, 180.0];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ItemKind {
    SpiceJar,
    CuttingBoard,
    Knife,
}

impl ItemKind {
    fn name(self) -> &'static str {
        match self {
            ItemKind::SpiceJar => "spice_jar",
            ItemKind::CuttingBoard => "cutting_board",
            ItemKind::Knife => "knife",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "spice_jar" => Some(ItemKind::SpiceJar),
            "cutting_board" => Some(ItemKind::CuttingBoard),
            "knife" => Some(ItemKind::Knife),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct Item {
    id: String,
    kind: ItemKind,
    pos: [f64; 2],
    removed: bool,
}

struct Kitchen {
    arm: [f64; 2],
    carrying: Option<usize>,
    items: Vec<Item>,
    cabinet_open: bool,
    drawer_open: bool,
    window_open: bool,
    processor_on: bool,
    knife_in_block: bool,
    done: bool,
    steps: usize,
    tasks_done: Vec<String>,
    required_tasks: Vec<String>,
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

impl Kitchen {
    fn complete_task(&mut self, task: &str) {
        let t = task.to_string();
        if !self.tasks_done.contains(&t) {
            self.tasks_done.push(t);
        }
        if !self.done && self.required_tasks.iter().all(|t| self.tasks_done.contains(t)) {
            self.done = true;
        }
    }

    fn observation(&self) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self.items.iter().enumerate()
            .filter(|(_, item)| !item.removed)
            .map(|(i, item)| serde_json::json!({
                "id": item.id,
                "kind": item.kind.name(),
                "pos": item.pos,
                "index": i,
            }))
            .collect();

        let carrying = self.carrying.map(|idx| {
            serde_json::json!({
                "id": self.items[idx].id,
                "kind": self.items[idx].kind.name(),
            })
        });

        serde_json::json!({
            "arm_pos": self.arm,
            "carrying": carrying,
            "items": items,
            "cabinet_open": self.cabinet_open,
            "drawer_open": self.drawer_open,
            "window_open": self.window_open,
            "processor_on": self.processor_on,
            "knife_in_block": self.knife_in_block,
            "tasks_done": self.tasks_done,
            "required_tasks": self.required_tasks,
            "done": self.done,
            "steps": self.steps,
            "step_count": self.steps,
        })
    }

    fn from_config(input: &serde_json::Value) -> Self {
        if let Some(items_arr) = input.get("items").and_then(|v| v.as_array()) {
            let mut items = Vec::new();
            for (i, iv) in items_arr.iter().enumerate() {
                let id = iv.get("id").and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("item{i}"));
                let kind = iv.get("kind").and_then(|v| v.as_str())
                    .and_then(ItemKind::from_str)
                    .unwrap_or(ItemKind::SpiceJar);
                let pos = iv.get("pos").and_then(|v| v.as_array())
                    .map(|a| [
                        a.first().and_then(|v| v.as_f64()).unwrap_or(200.0),
                        a.get(1).and_then(|v| v.as_f64()).unwrap_or(200.0),
                    ])
                    .unwrap_or([200.0, 200.0]);
                items.push(Item { id, kind, pos, removed: false });
            }

            let required_tasks: Vec<String> = input.get("required_tasks")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_else(Self::all_tasks);

            return Kitchen {
                arm: COUNTER_CENTER,
                carrying: None,
                items,
                cabinet_open: input.get("cabinet_open").and_then(|v| v.as_bool()).unwrap_or(false),
                drawer_open: input.get("drawer_open").and_then(|v| v.as_bool()).unwrap_or(true),
                window_open: input.get("window_open").and_then(|v| v.as_bool()).unwrap_or(false),
                processor_on: false,
                knife_in_block: false,
                done: false,
                steps: 0,
                tasks_done: Vec::new(),
                required_tasks,
            };
        }

        let seed = input.get("seed").and_then(|v| v.as_u64()).unwrap_or(42);
        Self::default_scenario(seed)
    }

    fn default_scenario(seed: u64) -> Self {
        let mut rng = Rng(seed);
        Kitchen {
            arm: COUNTER_CENTER,
            carrying: None,
            items: vec![
                Item {
                    id: "jar1".into(), kind: ItemKind::SpiceJar,
                    pos: [120.0 + (rng.next() % 250) as f64, 80.0 + (rng.next() % 180) as f64],
                    removed: false,
                },
                Item {
                    id: "board1".into(), kind: ItemKind::CuttingBoard,
                    pos: [350.0 + (rng.next() % 120) as f64, 80.0 + (rng.next() % 100) as f64],
                    removed: false,
                },
                Item {
                    id: "knife1".into(), kind: ItemKind::Knife,
                    pos: [150.0 + (rng.next() % 200) as f64, 120.0 + (rng.next() % 150) as f64],
                    removed: false,
                },
            ],
            cabinet_open: false,
            drawer_open: true,
            window_open: false,
            processor_on: false,
            knife_in_block: false,
            done: false,
            steps: 0,
            tasks_done: Vec::new(),
            required_tasks: Self::all_tasks(),
        }
    }

    fn all_tasks() -> Vec<String> {
        vec![
            "reach".into(), "push".into(), "pick_place".into(),
            "door_open".into(), "drawer_close".into(), "drawer_open".into(),
            "button_press".into(), "peg_insert".into(),
            "window_open".into(), "window_close".into(),
        ]
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

// ---------------------------------------------------------------------------
// Port
// ---------------------------------------------------------------------------

pub struct KitchenPort {
    spec: PortSpec,
    state: Mutex<Option<Kitchen>>,
}

impl KitchenPort {
    pub fn new() -> Self {
        Self { spec: build_spec(), state: Mutex::new(None) }
    }
}

impl Default for KitchenPort {
    fn default() -> Self { Self::new() }
}

impl Port for KitchenPort {
    fn spec(&self) -> &PortSpec { &self.spec }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "reset" => self.do_reset(&input),
            "scan" => self.do_scan(&input),
            "push_board" => self.do_push_board(),
            "pick_jar" => self.do_pick_jar(),
            "pick_knife" => self.do_pick_knife(),
            "place_shelf" => self.do_place_shelf(),
            "place_counter" => self.do_place_counter(),
            "door_open" => self.do_door_open(),
            "door_close" => self.do_door_close(),
            "drawer_open" => self.do_drawer_open(),
            "drawer_close" => self.do_drawer_close(),
            "button_press" => self.do_button_press(),
            "peg_insert" => self.do_peg_insert(),
            "window_open" => self.do_window_open(),
            "window_close" => self.do_window_close(),
            other => return Err(PortError::Validation(format!("unknown capability: {other}"))),
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        match result {
            Ok(value) => Ok(PortCallRecord::success(PORT_ID, capability_id, value, latency_ms)),
            Err(e) => Ok(PortCallRecord::failure(PORT_ID, capability_id, e.failure_class(), &e.to_string(), latency_ms)),
        }
    }

    fn validate_input(&self, capability_id: &str, _input: &serde_json::Value) -> soma_port_sdk::Result<()> {
        match capability_id {
            "reset"|"scan"|"push_board"|"pick_jar"|"pick_knife"|"place_shelf"|"place_counter"|
            "door_open"|"door_close"|"drawer_open"|"drawer_close"|"button_press"|"peg_insert"|
            "window_open"|"window_close" => Ok(()),
            other => Err(PortError::Validation(format!("unknown capability: {other}"))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState { PortLifecycleState::Active }
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

impl KitchenPort {
    fn with_kitchen<F, T>(&self, f: F) -> soma_port_sdk::Result<T>
    where F: FnOnce(&mut Kitchen) -> soma_port_sdk::Result<T> {
        let mut lock = self.state.lock().unwrap();
        match lock.as_mut() {
            Some(k) => f(k),
            None => Err(PortError::ExternalError("no active kitchen — call reset first".into())),
        }
    }

    fn do_reset(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let kitchen = Kitchen::from_config(input);
        let obs = kitchen.observation();
        *self.state.lock().unwrap() = Some(kitchen);
        Ok(obs)
    }

    fn do_scan(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        { let lock = self.state.lock().unwrap(); if lock.is_none() { drop(lock); self.do_reset(input)?; } }
        self.with_kitchen(|k| {
            k.steps += 1;
            k.complete_task("reach");
            Ok(k.observation())
        })
    }

    fn do_push_board(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if let Some(idx) = k.items.iter().position(|i| i.kind == ItemKind::CuttingBoard && !i.removed) {
                if dist(k.items[idx].pos, BOARD_TARGET) < 10.0 {
                    k.complete_task("push");
                    return Ok(k.observation());
                }
                k.arm = k.items[idx].pos;
                k.items[idx].pos = BOARD_TARGET;
                k.arm = BOARD_TARGET;
                k.complete_task("reach");
                k.complete_task("push");
            }
            Ok(k.observation())
        })
    }

    fn do_pick_jar(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if k.carrying.is_some() { return Ok(k.observation()); }
            let mut best: Option<(usize, f64)> = None;
            for (i, item) in k.items.iter().enumerate() {
                if item.kind == ItemKind::SpiceJar && !item.removed {
                    let d = dist(k.arm, item.pos);
                    if best.is_none() || d < best.unwrap().1 { best = Some((i, d)); }
                }
            }
            if let Some((idx, _)) = best {
                k.arm = k.items[idx].pos;
                k.carrying = Some(idx);
                k.complete_task("reach");
            }
            Ok(k.observation())
        })
    }

    fn do_pick_knife(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if k.carrying.is_some() { return Ok(k.observation()); }
            if k.knife_in_block { return Ok(k.observation()); }
            if let Some(idx) = k.items.iter().position(|i| i.kind == ItemKind::Knife && !i.removed) {
                k.arm = k.items[idx].pos;
                k.carrying = Some(idx);
                k.complete_task("reach");
            }
            Ok(k.observation())
        })
    }

    fn do_place_shelf(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if let Some(idx) = k.carrying.take() {
                k.arm = SHELF_POS;
                k.items[idx].pos = SHELF_POS;
                k.items[idx].removed = true;
                k.complete_task("reach");
                k.complete_task("pick_place");
            }
            Ok(k.observation())
        })
    }

    fn do_place_counter(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if let Some(idx) = k.carrying.take() {
                k.items[idx].pos = k.arm;
                k.items[idx].removed = false;
                k.complete_task("pick_place");
            }
            Ok(k.observation())
        })
    }

    fn do_door_open(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if k.cabinet_open { return Ok(k.observation()); }
            k.arm = CABINET_POS;
            k.cabinet_open = true;
            k.complete_task("reach");
            k.complete_task("door_open");
            Ok(k.observation())
        })
    }

    fn do_door_close(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if !k.cabinet_open { return Ok(k.observation()); }
            k.arm = CABINET_POS;
            k.cabinet_open = false;
            k.complete_task("reach");
            Ok(k.observation())
        })
    }

    fn do_drawer_open(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if k.drawer_open { return Ok(k.observation()); }
            k.arm = DRAWER_POS;
            k.drawer_open = true;
            k.complete_task("reach");
            k.complete_task("drawer_open");
            Ok(k.observation())
        })
    }

    fn do_drawer_close(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if !k.drawer_open { return Ok(k.observation()); }
            k.arm = DRAWER_POS;
            k.drawer_open = false;
            k.complete_task("reach");
            k.complete_task("drawer_close");
            Ok(k.observation())
        })
    }

    fn do_button_press(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            k.arm = PROCESSOR_POS;
            k.processor_on = true;
            k.complete_task("reach");
            k.complete_task("button_press");
            Ok(k.observation())
        })
    }

    fn do_peg_insert(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if k.knife_in_block { return Ok(k.observation()); }
            if let Some(idx) = k.carrying {
                if k.items[idx].kind == ItemKind::Knife {
                    k.arm = BLOCK_POS;
                    k.items[idx].pos = BLOCK_POS;
                    k.items[idx].removed = true;
                    k.carrying = None;
                    k.knife_in_block = true;
                    k.complete_task("reach");
                    k.complete_task("peg_insert");
                }
            }
            Ok(k.observation())
        })
    }

    fn do_window_open(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if k.window_open { return Ok(k.observation()); }
            k.arm = WINDOW_POS;
            k.window_open = true;
            k.complete_task("reach");
            k.complete_task("window_open");
            Ok(k.observation())
        })
    }

    fn do_window_close(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_kitchen(|k| {
            if k.done { return Ok(k.observation()); }
            k.steps += 1;
            if !k.window_open { return Ok(k.observation()); }
            k.arm = WINDOW_POS;
            k.window_open = false;
            k.complete_task("reach");
            k.complete_task("window_close");
            Ok(k.observation())
        })
    }
}

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

fn cap(id: &str, purpose: &str, effect: SideEffectClass) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.into(), name: id.into(), purpose: purpose.into(),
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::object(serde_json::json!({"arm_pos":{"type":"array"},"done":{"type":"boolean"}})),
        effect_class: effect, rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic, idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
        cost_profile: CostProfile::default(), remote_exposable: false, auth_override: None,
    }
}

fn build_spec() -> PortSpec {
    let m = SideEffectClass::LocalStateMutation;
    PortSpec {
        port_id: PORT_ID.into(), name: "Kitchen".into(),
        version: semver::Version::new(0, 1, 0), kind: PortKind::Custom,
        description: "2D kitchen countertop simulation covering all 10 Meta-World ML10 manipulation tasks".into(),
        namespace: "kitchen".into(), trust_level: TrustLevel::BuiltIn,
        capabilities: vec![
            cap("reset", "Create kitchen from scenario config", m),
            cap("scan", "Observe kitchen state", SideEffectClass::ReadOnly),
            cap("push_board", "Push cutting board to counter center", m),
            cap("pick_jar", "Pick up nearest spice jar", m),
            cap("pick_knife", "Pick up knife", m),
            cap("place_shelf", "Place carried item on cabinet shelf", m),
            cap("place_counter", "Place carried item on counter", m),
            cap("door_open", "Open cabinet door", m),
            cap("door_close", "Close cabinet door", m),
            cap("drawer_open", "Open drawer", m),
            cap("drawer_close", "Close drawer", m),
            cap("button_press", "Press food processor button", m),
            cap("peg_insert", "Insert carried knife into block", m),
            cap("window_open", "Slide window open", m),
            cap("window_close", "Slide window closed", m),
        ],
        input_schema: SchemaRef::any(), output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: m,
        latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["arm_pos".into(), "done".into(), "carrying".into(), "tasks_done".into()],
        validation_rules: vec![], remote_exposure: false,
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(KitchenPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_organize_sequence() {
        let port = KitchenPort::new();
        let r = port.invoke("reset", serde_json::json!({"preset": "organize"})).unwrap();
        assert!(r.success);
        assert!(!r.structured_result["done"].as_bool().unwrap());

        let sequence = [
            "scan", "push_board", "pick_jar", "door_open", "place_shelf",
            "door_close", "drawer_close", "drawer_open", "pick_knife",
            "peg_insert", "button_press", "window_open", "window_close",
        ];
        for cap in sequence {
            let r = port.invoke(cap, serde_json::json!({})).unwrap();
            assert!(r.success, "{cap} failed");
        }

        let r = port.invoke("scan", serde_json::json!({})).unwrap();
        assert!(r.structured_result["done"].as_bool().unwrap(), "should be done after all tasks");
    }

    #[test]
    fn noop_semantics() {
        let port = KitchenPort::new();
        port.invoke("reset", serde_json::json!({"preset": "organize"})).unwrap();

        let r = port.invoke("door_open", serde_json::json!({})).unwrap();
        assert!(r.success);
        let r = port.invoke("door_open", serde_json::json!({})).unwrap();
        assert!(r.success, "door_open should no-op when already open");

        let r = port.invoke("drawer_close", serde_json::json!({})).unwrap();
        assert!(r.success);
        let r = port.invoke("drawer_close", serde_json::json!({})).unwrap();
        assert!(r.success, "drawer_close should no-op when already closed");
    }

    #[test]
    fn pick_place_cycle() {
        let port = KitchenPort::new();
        port.invoke("reset", serde_json::json!({"preset": "organize"})).unwrap();

        let r = port.invoke("pick_jar", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["carrying"].is_object());

        let r = port.invoke("place_counter", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["carrying"].is_null());
    }

    #[test]
    fn peg_insert_requires_knife() {
        let port = KitchenPort::new();
        port.invoke("reset", serde_json::json!({"preset": "organize"})).unwrap();

        let r = port.invoke("peg_insert", serde_json::json!({})).unwrap();
        assert!(r.success, "peg_insert should no-op without knife");
        assert!(!r.structured_result["knife_in_block"].as_bool().unwrap());

        port.invoke("pick_knife", serde_json::json!({})).unwrap();
        let r = port.invoke("peg_insert", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["knife_in_block"].as_bool().unwrap());
    }

    #[test]
    fn custom_layout() {
        let port = KitchenPort::new();
        let r = port.invoke("reset", serde_json::json!({
            "items": [
                {"id": "j1", "kind": "spice_jar", "pos": [200, 150]},
                {"id": "k1", "kind": "knife", "pos": [300, 150]}
            ],
            "required_tasks": ["pick_place", "peg_insert"]
        })).unwrap();
        assert!(r.success);

        port.invoke("pick_jar", serde_json::json!({})).unwrap();
        port.invoke("place_shelf", serde_json::json!({})).unwrap();
        port.invoke("pick_knife", serde_json::json!({})).unwrap();
        let r = port.invoke("peg_insert", serde_json::json!({})).unwrap();
        assert!(r.structured_result["done"].as_bool().unwrap());
    }
}
