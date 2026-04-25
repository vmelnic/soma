use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "gridworld";

// ---------------------------------------------------------------------------
// Grid world
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cell {
    Empty,
    Wall,
    Key,
    DoorLocked,
    DoorOpen,
    Goal,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    fn dx(self) -> i32 {
        match self {
            Dir::East => 1,
            Dir::West => -1,
            _ => 0,
        }
    }
    fn dy(self) -> i32 {
        match self {
            Dir::North => -1,
            Dir::South => 1,
            _ => 0,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Dir::North => "north",
            Dir::East => "east",
            Dir::South => "south",
            Dir::West => "west",
        }
    }
}

struct Grid {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
    ax: usize,
    ay: usize,
    adir: Dir,
    has_key: bool,
    key_pos: (usize, usize),
    door_pos: (usize, usize),
    goal_pos: (usize, usize),
    done: bool,
    reward: f64,
    steps: usize,
}

impl Grid {
    fn cell(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.w + x]
    }
    fn set_cell(&mut self, x: usize, y: usize, c: Cell) {
        self.cells[y * self.w + x] = c;
    }

    fn new_doorkey(size: usize, seed: u64) -> Self {
        let mut rng = Rng(seed);
        let w = size;
        let h = size;
        let mut cells = vec![Cell::Empty; w * h];

        // Outer walls.
        for x in 0..w {
            cells[x] = Cell::Wall;
            cells[(h - 1) * w + x] = Cell::Wall;
        }
        for y in 0..h {
            cells[y * w] = Cell::Wall;
            cells[y * w + (w - 1)] = Cell::Wall;
        }

        // Vertical wall dividing left and right rooms.
        let wall_x = w / 2;
        for y in 1..h - 1 {
            cells[y * w + wall_x] = Cell::Wall;
        }

        // Door in the wall.
        let door_y = 1 + (rng.next() as usize % (h - 2));
        cells[door_y * w + wall_x] = Cell::DoorLocked;

        // Key in the left room.
        let (kx, ky) = loop {
            let x = 1 + (rng.next() as usize % (wall_x - 1));
            let y = 1 + (rng.next() as usize % (h - 2));
            if cells[y * w + x] == Cell::Empty {
                break (x, y);
            }
        };
        cells[ky * w + kx] = Cell::Key;

        // Goal in the right room.
        let (gx, gy) = loop {
            let x = wall_x + 1 + (rng.next() as usize % (w - wall_x - 2));
            let y = 1 + (rng.next() as usize % (h - 2));
            if cells[y * w + x] == Cell::Empty {
                break (x, y);
            }
        };
        cells[gy * w + gx] = Cell::Goal;

        // Agent in the left room, not on key.
        let (ax, ay) = loop {
            let x = 1 + (rng.next() as usize % (wall_x - 1));
            let y = 1 + (rng.next() as usize % (h - 2));
            if cells[y * w + x] == Cell::Empty {
                break (x, y);
            }
        };

        Grid {
            w,
            h,
            cells,
            ax,
            ay,
            adir: Dir::East,
            has_key: false,
            key_pos: (kx, ky),
            door_pos: (wall_x, door_y),
            goal_pos: (gx, gy),
            done: false,
            reward: 0.0,
            steps: 0,
        }
    }

    fn render(&self) -> String {
        let mut lines = Vec::with_capacity(self.h);
        for y in 0..self.h {
            let mut row = String::with_capacity(self.w * 2);
            for x in 0..self.w {
                if x == self.ax && y == self.ay {
                    let arrow = match self.adir {
                        Dir::North => '△',
                        Dir::East => '▷',
                        Dir::South => '▽',
                        Dir::West => '◁',
                    };
                    row.push(arrow);
                } else {
                    match self.cell(x, y) {
                        Cell::Empty => row.push('·'),
                        Cell::Wall => row.push('█'),
                        Cell::Key => row.push('K'),
                        Cell::DoorLocked => row.push('D'),
                        Cell::DoorOpen => row.push('_'),
                        Cell::Goal => row.push('G'),
                    }
                }
            }
            lines.push(row);
        }
        lines.join("\n")
    }

    fn render_ansi(&self) -> String {
        let mut lines = Vec::with_capacity(self.h);
        for y in 0..self.h {
            let mut row = String::new();
            for x in 0..self.w {
                if x == self.ax && y == self.ay {
                    let arrow = match self.adir {
                        Dir::North => '▲',
                        Dir::East => '▶',
                        Dir::South => '▼',
                        Dir::West => '◀',
                    };
                    // Red agent on dark floor
                    row.push_str(&format!("\x1b[91;48;5;254m{arrow} \x1b[0m"));
                } else {
                    match self.cell(x, y) {
                        Cell::Empty => row.push_str("\x1b[48;5;254m  \x1b[0m"),
                        Cell::Wall => row.push_str("\x1b[48;5;239m  \x1b[0m"),
                        Cell::Key => row.push_str("\x1b[93;48;5;254m🔑\x1b[0m"),
                        Cell::DoorLocked => row.push_str("\x1b[48;5;52m🚪\x1b[0m"),
                        Cell::DoorOpen => row.push_str("\x1b[48;5;22m  \x1b[0m"),
                        Cell::Goal => row.push_str("\x1b[48;5;28m🏁\x1b[0m"),
                    }
                }
            }
            lines.push(row);
        }
        lines.join("\n")
    }

    fn cell_char(&self, x: usize, y: usize) -> &'static str {
        if x == self.ax && y == self.ay {
            return "A";
        }
        match self.cell(x, y) {
            Cell::Empty => ".",
            Cell::Wall => "W",
            Cell::Key => "K",
            Cell::DoorLocked => "D",
            Cell::DoorOpen => "O",
            Cell::Goal => "G",
        }
    }

    fn observation(&self) -> serde_json::Value {
        let cells: Vec<Vec<&str>> = (0..self.h)
            .map(|y| (0..self.w).map(|x| self.cell_char(x, y)).collect())
            .collect();

        serde_json::json!({
            "agent_pos": [self.ax, self.ay],
            "agent_dir": self.adir.name(),
            "carrying_key": self.has_key,
            "key_pos": if !self.has_key { serde_json::json!([self.key_pos.0, self.key_pos.1]) } else { serde_json::json!(null) },
            "door_pos": [self.door_pos.0, self.door_pos.1],
            "door_locked": self.cell(self.door_pos.0, self.door_pos.1) == Cell::DoorLocked,
            "goal_pos": [self.goal_pos.0, self.goal_pos.1],
            "done": self.done,
            "reward": self.reward,
            "step_count": self.steps,
            "grid_size": self.w,
            "cells": cells,
            "render": self.render(),
            "render_ansi": self.render_ansi(),
        })
    }

    fn passable(&self, x: usize, y: usize) -> bool {
        match self.cell(x, y) {
            Cell::Wall | Cell::DoorLocked => false,
            _ => true,
        }
    }

    fn bfs_path(&self, sx: usize, sy: usize, tx: usize, ty: usize) -> Option<Vec<(usize, usize)>> {
        if sx == tx && sy == ty {
            return Some(vec![(tx, ty)]);
        }
        let mut visited = vec![false; self.w * self.h];
        let mut parent: Vec<Option<usize>> = vec![None; self.w * self.h];
        let mut queue = VecDeque::new();
        let si = sy * self.w + sx;
        visited[si] = true;
        queue.push_back(si);

        while let Some(idx) = queue.pop_front() {
            let cx = idx % self.w;
            let cy = idx / self.w;
            if cx == tx && cy == ty {
                let mut path = vec![];
                let mut cur = idx;
                loop {
                    path.push((cur % self.w, cur / self.w));
                    match parent[cur] {
                        Some(p) => cur = p,
                        None => break,
                    }
                }
                path.reverse();
                return Some(path);
            }
            for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1i32), (0, 1)] {
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if nx < 0 || ny < 0 || nx >= self.w as i32 || ny >= self.h as i32 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let ni = ny * self.w + nx;
                if !visited[ni] && self.passable(nx, ny) {
                    visited[ni] = true;
                    parent[ni] = Some(idx);
                    queue.push_back(ni);
                }
            }
        }
        None
    }

    fn navigate_to(&mut self, tx: usize, ty: usize) -> bool {
        if let Some(path) = self.bfs_path(self.ax, self.ay, tx, ty) {
            if path.len() > 1 {
                let dest = path[path.len() - 1];
                self.ax = dest.0;
                self.ay = dest.1;
                self.steps += path.len() - 1;
            }
            true
        } else {
            false
        }
    }
}

// Minimal xorshift64 RNG.
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
// Port struct
// ---------------------------------------------------------------------------

pub struct GridWorldPort {
    spec: PortSpec,
    state: Mutex<Option<Grid>>,
    seed_counter: Mutex<u64>,
}

impl GridWorldPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            state: Mutex::new(None),
            seed_counter: Mutex::new(42),
        }
    }

    fn next_seed(&self) -> u64 {
        let mut counter = self.seed_counter.lock().unwrap();
        *counter = counter.wrapping_add(1);
        *counter ^ 0xDEAD_BEEF_CAFE_BABE
    }
}

impl Default for GridWorldPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait
// ---------------------------------------------------------------------------

impl Port for GridWorldPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "reset" => self.do_reset(&input),
            "scan" => self.do_scan(),
            "go_to_key" => self.do_go_to_key(),
            "pickup" => self.do_pickup(),
            "go_to_door" => self.do_go_to_door(),
            "toggle" => self.do_toggle(),
            "go_to_goal" => self.do_go_to_goal(),
            other => return Err(PortError::Validation(format!("unknown capability: {other}"))),
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => Ok(PortCallRecord::success(PORT_ID, capability_id, value, latency_ms)),
            Err(e) => Ok(PortCallRecord::failure(PORT_ID, capability_id, e.failure_class(), &e.to_string(), latency_ms)),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        _input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        match capability_id {
            "reset" | "scan" | "go_to_key" | "pickup" | "go_to_door" | "toggle" | "go_to_goal" => Ok(()),
            other => Err(PortError::Validation(format!("unknown capability: {other}"))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl GridWorldPort {
    fn with_grid<F, T>(&self, f: F) -> soma_port_sdk::Result<T>
    where
        F: FnOnce(&mut Grid) -> soma_port_sdk::Result<T>,
    {
        let mut lock = self.state.lock().unwrap();
        match lock.as_mut() {
            Some(grid) => f(grid),
            None => Err(PortError::ExternalError("no active grid — call reset first".into())),
        }
    }

    fn do_reset(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let size = input.get("size").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
        if size < 5 || size > 20 {
            return Err(PortError::Validation("size must be 5..20".into()));
        }
        let seed = input.get("seed").and_then(|v| v.as_u64()).unwrap_or_else(|| self.next_seed());
        let grid = Grid::new_doorkey(size, seed);
        let obs = grid.observation();
        *self.state.lock().unwrap() = Some(grid);
        Ok(obs)
    }

    fn do_scan(&self) -> soma_port_sdk::Result<serde_json::Value> {
        {
            let lock = self.state.lock().unwrap();
            if lock.is_none() {
                drop(lock);
                self.do_reset(&serde_json::json!({}))?;
            }
        }
        self.with_grid(|grid| {
            grid.steps += 1;
            Ok(grid.observation())
        })
    }

    fn do_go_to_key(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done {
                return Ok(grid.observation());
            }
            if grid.has_key {
                return Err(PortError::ExternalError("already carrying key".into()));
            }
            let (kx, ky) = grid.key_pos;
            if grid.navigate_to(kx, ky) {
                Ok(grid.observation())
            } else {
                grid.steps += 1;
                Err(PortError::ExternalError("no path to key".into()))
            }
        })
    }

    fn do_pickup(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done {
                return Ok(grid.observation());
            }
            grid.steps += 1;
            let (kx, ky) = grid.key_pos;
            if grid.ax == kx && grid.ay == ky && grid.cell(kx, ky) == Cell::Key {
                grid.has_key = true;
                grid.set_cell(kx, ky, Cell::Empty);
                Ok(grid.observation())
            } else {
                Err(PortError::ExternalError("not on key cell".into()))
            }
        })
    }

    fn do_go_to_door(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done {
                return Ok(grid.observation());
            }
            let (dx, dy) = grid.door_pos;
            // Navigate to cell adjacent to the door (left side).
            let adj_x = if dx > 0 { dx - 1 } else { dx + 1 };
            if grid.navigate_to(adj_x, dy) {
                Ok(grid.observation())
            } else {
                grid.steps += 1;
                Err(PortError::ExternalError("no path to door".into()))
            }
        })
    }

    fn do_toggle(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done {
                return Ok(grid.observation());
            }
            grid.steps += 1;
            let (dx, dy) = grid.door_pos;
            let dist = (grid.ax as i32 - dx as i32).unsigned_abs() as usize
                + (grid.ay as i32 - dy as i32).unsigned_abs() as usize;
            if dist > 1 {
                return Err(PortError::ExternalError("not adjacent to door".into()));
            }
            if !grid.has_key {
                return Err(PortError::ExternalError("no key to unlock door".into()));
            }
            if grid.cell(dx, dy) == Cell::DoorLocked {
                grid.set_cell(dx, dy, Cell::DoorOpen);
                Ok(grid.observation())
            } else {
                Err(PortError::ExternalError("door already open".into()))
            }
        })
    }

    fn do_go_to_goal(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done {
                return Ok(grid.observation());
            }
            let (gx, gy) = grid.goal_pos;
            if grid.navigate_to(gx, gy) {
                grid.done = true;
                grid.reward = 1.0 - 0.9 * (grid.steps as f64 / (grid.w * grid.h * 4) as f64).min(1.0);
                Ok(grid.observation())
            } else {
                grid.steps += 1;
                Err(PortError::ExternalError("no path to goal — door may be locked".into()))
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

fn cap(id: &str, purpose: &str, effect: SideEffectClass) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.into(),
        name: id.into(),
        purpose: purpose.into(),
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::object(serde_json::json!({
            "agent_pos": {"type": "array"},
            "done": {"type": "boolean"},
            "reward": {"type": "number"},
        })),
        effect_class: effect,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 5,
            max_latency_ms: 10,
        },
        cost_profile: CostProfile::default(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "GridWorld".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "DoorKey grid world for MiniGrid benchmark comparison".into(),
        namespace: "gridworld".into(),
        trust_level: TrustLevel::BuiltIn,
        capabilities: vec![
            cap("reset", "Create new DoorKey grid, return initial observation", SideEffectClass::LocalStateMutation),
            cap("scan", "Return current grid observation", SideEffectClass::ReadOnly),
            cap("go_to_key", "Navigate agent to key position", SideEffectClass::LocalStateMutation),
            cap("pickup", "Pick up key at current position", SideEffectClass::LocalStateMutation),
            cap("go_to_door", "Navigate agent to door position", SideEffectClass::LocalStateMutation),
            cap("toggle", "Toggle door (unlock with key)", SideEffectClass::LocalStateMutation),
            cap("go_to_goal", "Navigate agent to goal position", SideEffectClass::LocalStateMutation),
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: SideEffectClass::LocalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 5,
            max_latency_ms: 10,
        },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["agent_pos".into(), "done".into(), "reward".into(), "carrying_key".into()],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(GridWorldPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doorkey_solvable() {
        for seed in 1..=100 {
            let mut grid = Grid::new_doorkey(8, seed);
            // Key must be reachable from agent.
            assert!(grid.bfs_path(grid.ax, grid.ay, grid.key_pos.0, grid.key_pos.1).is_some(),
                "seed {seed}: key unreachable");
            // Door adjacent must be reachable.
            let (dx, dy) = grid.door_pos;
            assert!(grid.bfs_path(grid.ax, grid.ay, dx - 1, dy).is_some(),
                "seed {seed}: door unreachable");
            // After unlocking door, goal must be reachable.
            grid.set_cell(dx, dy, Cell::DoorOpen);
            assert!(grid.bfs_path(dx, dy, grid.goal_pos.0, grid.goal_pos.1).is_some(),
                "seed {seed}: goal unreachable after door open");
        }
    }

    #[test]
    fn full_solve_sequence() {
        let port = GridWorldPort::new();
        let reset_input = serde_json::json!({"size": 8, "seed": 42});
        let r = port.invoke("reset", reset_input).unwrap();
        assert!(r.success);

        let r = port.invoke("scan", serde_json::json!({})).unwrap();
        assert!(r.success);

        let r = port.invoke("go_to_key", serde_json::json!({})).unwrap();
        assert!(r.success);

        let r = port.invoke("pickup", serde_json::json!({})).unwrap();
        assert!(r.success);

        let r = port.invoke("go_to_door", serde_json::json!({})).unwrap();
        assert!(r.success);

        let r = port.invoke("toggle", serde_json::json!({})).unwrap();
        assert!(r.success);

        let r = port.invoke("go_to_goal", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["done"].as_bool().unwrap());
        assert!(r.structured_result["reward"].as_f64().unwrap() > 0.0);
    }
}
