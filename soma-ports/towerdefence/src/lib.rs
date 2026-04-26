use std::collections::VecDeque;
use std::sync::Mutex;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "towerdefence";

// ---------------------------------------------------------------------------
// Cell types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Grass,
    Path,
    Start,
    End,
    Tower(TowerType, u8), // type, level
}

impl Cell {
    fn is_buildable(self) -> bool {
        matches!(self, Cell::Grass)
    }
}

// ---------------------------------------------------------------------------
// Tower types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TowerType {
    Archer,
    Cannon,
    Mage,
}

impl TowerType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "archer" => Some(TowerType::Archer),
            "cannon" => Some(TowerType::Cannon),
            "mage" => Some(TowerType::Mage),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            TowerType::Archer => "archer",
            TowerType::Cannon => "cannon",
            TowerType::Mage => "mage",
        }
    }

    fn base_cost(self) -> u32 {
        match self {
            TowerType::Archer => 50,
            TowerType::Cannon => 75,
            TowerType::Mage => 100,
        }
    }

    fn base_damage(self) -> u32 {
        match self {
            TowerType::Archer => 10,
            TowerType::Cannon => 25,
            TowerType::Mage => 40,
        }
    }

    fn base_range(self) -> usize {
        match self {
            TowerType::Archer => 3,
            TowerType::Cannon => 2,
            TowerType::Mage => 4,
        }
    }

    fn fire_cooldown(self) -> u32 {
        match self {
            TowerType::Archer => 2,  // fast
            TowerType::Cannon => 5, // slow
            TowerType::Mage => 4,   // medium
        }
    }

    fn splash_radius(self) -> usize {
        match self {
            TowerType::Cannon => 1,
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Enemy types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EnemyType {
    Basic,
    Fast,
    Tank,
    Boss,
}

impl EnemyType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "basic" => Some(EnemyType::Basic),
            "fast" => Some(EnemyType::Fast),
            "tank" => Some(EnemyType::Tank),
            "boss" => Some(EnemyType::Boss),
            _ => None,
        }
    }

    fn hp(self) -> u32 {
        match self {
            EnemyType::Basic => 30,
            EnemyType::Fast => 15,
            EnemyType::Tank => 100,
            EnemyType::Boss => 300,
        }
    }

    fn speed(self) -> u32 {
        match self {
            EnemyType::Basic => 1,
            EnemyType::Fast => 2,
            EnemyType::Tank => 1,
            EnemyType::Boss => 1,
        }
    }

    fn reward(self) -> u32 {
        match self {
            EnemyType::Basic => 5,
            EnemyType::Fast => 3,
            EnemyType::Tank => 15,
            EnemyType::Boss => 50,
        }
    }

    fn name(self) -> &'static str {
        match self {
            EnemyType::Basic => "basic",
            EnemyType::Fast => "fast",
            EnemyType::Tank => "tank",
            EnemyType::Boss => "boss",
        }
    }
}

// ---------------------------------------------------------------------------
// Tower instance
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Tower {
    x: usize,
    y: usize,
    ty: TowerType,
    level: u8,
    cooldown: u32,
}

impl Tower {
    fn damage(&self) -> u32 {
        let base = self.ty.base_damage();
        base * (1 + self.level as u32)
    }

    fn range(&self) -> usize {
        self.ty.base_range() + (self.level as usize)
    }

    fn fire_rate(&self) -> u32 {
        let base = self.ty.fire_cooldown();
        if base > self.level as u32 { base - self.level as u32 } else { 1 }
    }

    fn upgrade_cost(&self) -> u32 {
        self.ty.base_cost() * (self.level as u32 + 1)
    }

    fn sell_value(&self) -> u32 {
        let total_spent = self.ty.base_cost();
        let upgrades: u32 = (1..=self.level).map(|l| self.ty.base_cost() * l as u32).sum();
        (total_spent + upgrades) / 2
    }
}

// ---------------------------------------------------------------------------
// Enemy instance
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Enemy {
    ty: EnemyType,
    x: f64,
    y: f64,
    hp: u32,
    max_hp: u32,
    path_idx: usize,
    sub_step: u32, // fractional progress between path nodes
    alive: bool,
    reached_end: bool,
}

// ---------------------------------------------------------------------------
// Wave definition
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Wave {
    enemies: Vec<EnemyType>,
    spawn_interval: u32,
}

// ---------------------------------------------------------------------------
// Game state
// ---------------------------------------------------------------------------

struct Game {
    w: usize,
    h: usize,
    grid: Vec<Vec<Cell>>,
    path: Vec<(usize, usize)>,
    towers: Vec<Tower>,
    enemies: Vec<Enemy>,
    gold: u32,
    lives: u32,
    wave_index: usize,
    waves: Vec<Wave>,
    wave_active: bool,
    spawn_queue: VecDeque<EnemyType>,
    spawn_cooldown: u32,
    tick_count: u64,
    done: bool,
    won: bool,
}

impl Game {
    fn new(grid: Vec<Vec<Cell>>, path: Vec<(usize, usize)>, waves: Vec<Wave>, gold: u32, lives: u32) -> Self {
        let h = grid.len();
        let w = grid.first().map(|r| r.len()).unwrap_or(0);
        Game {
            w, h, grid, path, towers: Vec::new(), enemies: Vec::new(),
            gold, lives, wave_index: 0, waves, wave_active: false,
            spawn_queue: VecDeque::new(), spawn_cooldown: 0,
            tick_count: 0, done: false, won: false,
        }
    }

    fn place_tower(&mut self, x: usize, y: usize, ty: TowerType) -> Result<(), String> {
        if x >= self.w || y >= self.h {
            return Err("out of bounds".into());
        }
        if !self.grid[y][x].is_buildable() {
            return Err("cannot build here".into());
        }
        let cost = ty.base_cost();
        if self.gold < cost {
            return Err("not enough gold".into());
        }
        self.gold -= cost;
        self.grid[y][x] = Cell::Tower(ty, 1);
        self.towers.push(Tower { x, y, ty, level: 1, cooldown: 0 });
        Ok(())
    }

    fn upgrade_tower(&mut self, x: usize, y: usize) -> Result<(), String> {
        if let Some(tower) = self.towers.iter_mut().find(|t| t.x == x && t.y == y) {
            if tower.level >= 3 {
                return Err("max level reached".into());
            }
            let cost = tower.upgrade_cost();
            if self.gold < cost {
                return Err("not enough gold".into());
            }
            self.gold -= cost;
            tower.level += 1;
            self.grid[y][x] = Cell::Tower(tower.ty, tower.level);
            Ok(())
        } else {
            Err("no tower at this location".into())
        }
    }

    fn sell_tower(&mut self, x: usize, y: usize) -> Result<(), String> {
        if let Some(idx) = self.towers.iter().position(|t| t.x == x && t.y == y) {
            let tower = self.towers.remove(idx);
            self.gold += tower.sell_value();
            self.grid[y][x] = Cell::Grass;
            Ok(())
        } else {
            Err("no tower at this location".into())
        }
    }

    fn start_wave(&mut self) -> Result<(), String> {
        if self.wave_active {
            return Err("wave already active".into());
        }
        if self.wave_index >= self.waves.len() {
            return Err("all waves completed".into());
        }
        let wave = self.waves[self.wave_index].clone();
        self.spawn_queue = wave.enemies.into_iter().collect();
        self.spawn_cooldown = 0;
        self.wave_active = true;
        self.wave_index += 1;
        Ok(())
    }

    fn tick(&mut self, ticks: u32) {
        for _ in 0..ticks {
            if self.done { break; }
            self.tick_once();
        }
    }

    fn tick_once(&mut self) {
        self.tick_count += 1;

        // Spawn enemies
        if self.wave_active && !self.spawn_queue.is_empty() {
            if self.spawn_cooldown == 0 {
                if let Some(ty) = self.spawn_queue.pop_front() {
                    if let Some(&(sx, sy)) = self.path.first() {
                        self.enemies.push(Enemy {
                            ty,
                            x: sx as f64,
                            y: sy as f64,
                            hp: ty.hp(),
                            max_hp: ty.hp(),
                            path_idx: 0,
                            sub_step: 0,
                            alive: true,
                            reached_end: false,
                        });
                    }
                    let current_wave_idx = self.wave_index.saturating_sub(1);
                    self.spawn_cooldown = self.waves.get(current_wave_idx).map(|w| w.spawn_interval).unwrap_or(5);
                }
            } else {
                self.spawn_cooldown -= 1;
            }
        }

        // Move enemies
        for enemy in &mut self.enemies {
            if !enemy.alive || enemy.reached_end { continue; }
            let speed = enemy.ty.speed();
            enemy.sub_step += speed;
            while enemy.sub_step >= 10 && enemy.path_idx + 1 < self.path.len() {
                enemy.sub_step -= 10;
                enemy.path_idx += 1;
                let (tx, ty) = self.path[enemy.path_idx];
                enemy.x = tx as f64;
                enemy.y = ty as f64;
            }
            if enemy.path_idx + 1 >= self.path.len() {
                enemy.reached_end = true;
                enemy.alive = false;
                self.lives = self.lives.saturating_sub(1);
            }
        }

        // Tower attacks
        for tower in &mut self.towers {
            if tower.cooldown > 0 {
                tower.cooldown -= 1;
                continue;
            }
            let range = tower.range();
            let tx = tower.x as f64;
            let ty = tower.y as f64;

            // Find target in range
            let target_idx = self.enemies.iter_mut().enumerate()
                .filter(|(_, e)| e.alive && !e.reached_end)
                .map(|(i, e)| {
                    let dx = e.x - tx;
                    let dy = e.y - ty;
                    let dist_sq = dx * dx + dy * dy;
                    (i, dist_sq)
                })
                .filter(|(_, d)| *d <= (range * range) as f64)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(i, _)| i);

            if let Some(idx) = target_idx {
                tower.cooldown = tower.fire_rate();
                let damage = tower.damage();
                let splash = tower.ty.splash_radius();
                let ex = self.enemies[idx].x;
                let ey = self.enemies[idx].y;

                if splash > 0 {
                    for e in &mut self.enemies {
                        if !e.alive || e.reached_end { continue; }
                        let dx = e.x - ex;
                        let dy = e.y - ey;
                        if dx * dx + dy * dy <= (splash * splash) as f64 {
                            e.hp = e.hp.saturating_sub(damage);
                            if e.hp == 0 {
                                e.alive = false;
                                self.gold += e.ty.reward();
                            }
                        }
                    }
                } else {
                    let e = &mut self.enemies[idx];
                    e.hp = e.hp.saturating_sub(damage);
                    if e.hp == 0 {
                        e.alive = false;
                        self.gold += e.ty.reward();
                    }
                }
            }
        }

        // Wave complete when spawn queue empty and no living enemies
        if self.wave_active && self.spawn_queue.is_empty()
            && self.enemies.iter().all(|e| !e.alive || e.reached_end) {
            self.wave_active = false;
        }

        // Check win/lose
        if self.lives == 0 {
            self.done = true;
            self.won = false;
        } else if self.wave_index >= self.waves.len()
            && !self.wave_active
            && self.spawn_queue.is_empty()
            && self.enemies.iter().all(|e| !e.alive || e.reached_end) {
            self.done = true;
            self.won = true;
        }

        // Clean up dead/reached enemies periodically
        self.enemies.retain(|e| e.alive && !e.reached_end);
    }

    fn observation(&self) -> serde_json::Value {
        let cells: Vec<Vec<String>> = self.grid.iter().map(|row| {
            row.iter().map(|c| match c {
                Cell::Grass => ".".to_string(),
                Cell::Path => "P".to_string(),
                Cell::Start => "S".to_string(),
                Cell::End => "X".to_string(),
                Cell::Tower(ty, lv) => format!("tower_{}_{}", ty.name(), lv),
            }).collect()
        }).collect();

        let towers_json: Vec<serde_json::Value> = self.towers.iter().map(|t| {
            serde_json::json!({
                "x": t.x, "y": t.y,
                "kind": t.ty.name(),
                "level": t.level,
                "damage": t.damage(),
                "range": t.range(),
                "cooldown": t.cooldown,
            })
        }).collect();

        let enemies_json: Vec<serde_json::Value> = self.enemies.iter().map(|e| {
            serde_json::json!({
                "kind": e.ty.name(),
                "x": e.x, "y": e.y,
                "hp": e.hp, "max_hp": e.max_hp,
                "alive": e.alive, "reached_end": e.reached_end,
            })
        }).collect();

        serde_json::json!({
            "grid": cells,
            "size": [self.w, self.h],
            "gold": self.gold,
            "lives": self.lives,
            "wave_index": self.wave_index,
            "current_wave": self.wave_index,
            "wave_active": self.wave_active,
            "enemies_remaining": self.spawn_queue.len(),
            "towers": towers_json,
            "enemies": enemies_json,
            "projectiles": [],
            "tick_count": self.tick_count,
            "done": self.done,
            "won": self.won,
        })
    }
}

// ---------------------------------------------------------------------------
// Port
// ---------------------------------------------------------------------------

pub struct TowerDefencePort {
    spec: PortSpec,
    state: Mutex<Option<Game>>,
}

impl TowerDefencePort {
    pub fn new() -> Self {
        Self { spec: build_spec(), state: Mutex::new(None) }
    }
}

impl Default for TowerDefencePort {
    fn default() -> Self { Self::new() }
}

impl Port for TowerDefencePort {
    fn spec(&self) -> &PortSpec { &self.spec }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> soma_port_sdk::Result<PortCallRecord> {
        let start = std::time::Instant::now();
        let result = match capability_id {
            "reset" => self.do_reset(&input),
            "get_state" => self.do_get_state(),
            "place_tower" => self.do_place_tower(&input),
            "upgrade_tower" => self.do_upgrade_tower(&input),
            "sell_tower" => self.do_sell_tower(&input),
            "start_wave" => self.do_start_wave(),
            "tick" => self.do_tick(&input),
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
            "reset"|"get_state"|"place_tower"|"upgrade_tower"|"sell_tower"|"start_wave"|"tick" => Ok(()),
            other => Err(PortError::Validation(format!("unknown capability: {other}"))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState { PortLifecycleState::Active }
}

impl TowerDefencePort {
    fn with_game<F, T>(&self, f: F) -> soma_port_sdk::Result<T>
    where F: FnOnce(&mut Game) -> soma_port_sdk::Result<T> {
        let mut lock = self.state.lock().unwrap();
        match lock.as_mut() {
            Some(game) => f(game),
            None => Err(PortError::ExternalError("no active game — call reset first".into())),
        }
    }

    fn do_reset(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let layout = input.get("layout").and_then(|v| v.as_array())
            .ok_or_else(|| PortError::Validation("layout must be array of strings".into()))?;
        let mut grid: Vec<Vec<Cell>> = Vec::new();
        let mut path: Vec<(usize, usize)> = Vec::new();
        let mut start: Option<(usize, usize)> = None;
        let mut end: Option<(usize, usize)> = None;

        for (y, row_val) in layout.iter().enumerate() {
            let row_str = row_val.as_str().unwrap_or("");
            let mut row: Vec<Cell> = Vec::new();
            for (x, ch) in row_str.chars().enumerate() {
                let cell = match ch {
                    '.' | 'B' => Cell::Grass,
                    '#' | 'P' => Cell::Path,
                    'S' => { start = Some((x, y)); Cell::Start }
                    'E' | 'X' => { end = Some((x, y)); Cell::End }
                    _ => Cell::Grass,
                };
                row.push(cell);
            }
            grid.push(row);
        }

        // Build path from explicit waypoints or derive from grid
        if let Some(waypoints) = input.get("path").and_then(|v| v.as_array()) {
            path = waypoints.iter().filter_map(|v| {
                let arr = v.as_array()?;
                let x = arr.first()?.as_u64()? as usize;
                let y = arr.get(1)?.as_u64()? as usize;
                Some((x, y))
            }).collect();
        } else {
            // Derive path: find start, then follow adjacent path cells to end
            if let Some(s) = start {
                path = Self::trace_path(&grid, s, end);
            }
        }

        let waves: Vec<Wave> = input.get("waves").and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|w| {
                let enemies: Vec<EnemyType> = w.get("enemies")?.as_array()?.iter().filter_map(|e| {
                    EnemyType::from_str(e.as_str()?)
                }).collect();
                let interval = w.get("spawn_interval")?.as_u64()? as u32;
                Some(Wave { enemies, spawn_interval: interval })
            }).collect()).unwrap_or_else(|| vec![
                Wave { enemies: vec![EnemyType::Basic; 5], spawn_interval: 5 },
                Wave { enemies: { let mut v = vec![EnemyType::Basic; 8]; v.extend(vec![EnemyType::Fast; 3]); v }, spawn_interval: 4 },
                Wave { enemies: { let mut v = vec![EnemyType::Tank; 3]; v.extend(vec![EnemyType::Basic; 5]); v }, spawn_interval: 5 },
            ]);

        let gold = input.get("initial_gold").or_else(|| input.get("gold")).and_then(|v| v.as_u64()).unwrap_or(100) as u32;
        let lives = input.get("lives").and_then(|v| v.as_u64()).unwrap_or(20) as u32;

        let game = Game::new(grid, path, waves, gold, lives);
        let obs = game.observation();
        *self.state.lock().unwrap() = Some(game);
        Ok(obs)
    }

    fn trace_path(grid: &[Vec<Cell>], start: (usize, usize), end: Option<(usize, usize)>) -> Vec<(usize, usize)> {
        let h = grid.len();
        let w = grid.first().map(|r| r.len()).unwrap_or(0);
        let mut visited = vec![vec![false; w]; h];
        let mut path = Vec::new();
        let mut current = start;
        path.push(current);
        visited[current.1][current.0] = true;

        loop {
            if Some(current) == end { break; }
            let mut found = false;
            for (dx, dy) in [(0i32, 1), (1, 0), (0, -1), (-1, 0)] {
                let nx = current.0 as i32 + dx;
                let ny = current.1 as i32 + dy;
                if nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 {
                    let (ux, uy) = (nx as usize, ny as usize);
                    if !visited[uy][ux] && matches!(grid[uy][ux], Cell::Path | Cell::End) {
                        visited[uy][ux] = true;
                        current = (ux, uy);
                        path.push(current);
                        found = true;
                        break;
                    }
                }
            }
            if !found { break; }
        }
        path
    }

    fn do_get_state(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_game(|game| Ok(game.observation()))
    }

    fn do_place_tower(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let x = input.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let y = input.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let kind = input.get("kind").and_then(|v| v.as_str())
            .or_else(|| input.get("tower_type").and_then(|v| v.as_str()))
            .unwrap_or("archer");
        let ty = TowerType::from_str(kind)
            .ok_or_else(|| PortError::Validation("kind must be archer, cannon, or mage".into()))?;
        self.with_game(|game| {
            game.place_tower(x, y, ty).map_err(|e| PortError::ExternalError(e))?;
            Ok(game.observation())
        })
    }

    fn do_upgrade_tower(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let x = input.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let y = input.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        self.with_game(|game| {
            game.upgrade_tower(x, y).map_err(|e| PortError::ExternalError(e))?;
            Ok(game.observation())
        })
    }

    fn do_sell_tower(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let x = input.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let y = input.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        self.with_game(|game| {
            game.sell_tower(x, y).map_err(|e| PortError::ExternalError(e))?;
            Ok(game.observation())
        })
    }

    fn do_start_wave(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_game(|game| {
            game.start_wave().map_err(|e| PortError::ExternalError(e))?;
            Ok(game.observation())
        })
    }

    fn do_tick(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let ticks = input.get("ticks").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        self.with_game(|game| {
            game.tick(ticks);
            Ok(game.observation())
        })
    }
}

fn build_spec() -> PortSpec {
    let m = SideEffectClass::LocalStateMutation;
    PortSpec {
        port_id: PORT_ID.into(), name: "TowerDefence".into(),
        version: semver::Version::new(0, 1, 0), kind: PortKind::Custom,
        description: "Grid-based tower defence simulation environment".into(),
        namespace: "towerdefence".into(), trust_level: TrustLevel::BuiltIn,
        capabilities: vec![
            cap("reset", "Create game from layout and wave config", m),
            cap("get_state", "Return current game state", SideEffectClass::ReadOnly),
            cap("place_tower", "Place a tower at grid coordinates", m),
            cap("upgrade_tower", "Upgrade an existing tower", m),
            cap("sell_tower", "Sell a tower for partial refund", m),
            cap("start_wave", "Begin the next wave", m),
            cap("tick", "Advance simulation by N ticks", m),
        ],
        input_schema: SchemaRef::any(), output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: m,
        latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["gold".into(), "lives".into(), "done".into(), "won".into()],
        validation_rules: vec![], remote_exposure: false,
    }
}

fn cap(id: &str, purpose: &str, effect: SideEffectClass) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.into(), name: id.into(), purpose: purpose.into(),
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::object(serde_json::json!({"gold":{"type":"integer"},"lives":{"type":"integer"},"done":{"type":"boolean"}})),
        effect_class: effect, rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic, idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
        cost_profile: CostProfile::default(), remote_exposable: false, auth_override: None,
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(TowerDefencePort::new()))
}
