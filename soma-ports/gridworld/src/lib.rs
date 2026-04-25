use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "gridworld";
const DEFAULT_VIEW_RADIUS: usize = 3;

// ---------------------------------------------------------------------------
// Cell types and colors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Empty,
    Wall,
    Key,
    DoorLocked,
    DoorOpen,
    Goal,
    Lava,
    Ball,
    Box_,
}

impl Cell {
    fn from_char(ch: &str) -> Self {
        match ch {
            "W" => Cell::Wall,
            "K" => Cell::Key,
            "D" => Cell::DoorLocked,
            "O" => Cell::DoorOpen,
            "G" => Cell::Goal,
            "L" => Cell::Lava,
            "B" => Cell::Ball,
            "X" => Cell::Box_,
            _ => Cell::Empty,
        }
    }

    fn to_char(self) -> &'static str {
        match self {
            Cell::Empty => ".",
            Cell::Wall => "W",
            Cell::Key => "K",
            Cell::DoorLocked => "D",
            Cell::DoorOpen => "O",
            Cell::Goal => "G",
            Cell::Lava => "L",
            Cell::Ball => "B",
            Cell::Box_ => "X",
        }
    }

    fn is_pickable(self) -> bool {
        matches!(self, Cell::Key | Cell::Ball | Cell::Box_)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    fn name(self) -> &'static str {
        match self {
            Dir::North => "north",
            Dir::East => "east",
            Dir::South => "south",
            Dir::West => "west",
        }
    }
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

struct Grid {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
    ax: usize,
    ay: usize,
    adir: Dir,
    carrying: Option<Cell>,
    done: bool,
    reward: f64,
    steps: usize,
    explored: Vec<bool>,
    view_radius: usize,
}

impl Grid {
    fn cell(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.w + x]
    }
    fn set_cell(&mut self, x: usize, y: usize, c: Cell) {
        self.cells[y * self.w + x] = c;
    }

    fn has_fog(&self) -> bool { !self.explored.is_empty() }

    fn is_explored(&self, x: usize, y: usize) -> bool {
        !self.has_fog() || self.explored[y * self.w + x]
    }

    fn with_fog(mut self, view_radius: usize) -> Self {
        self.view_radius = view_radius;
        self.explored = vec![false; self.w * self.h];
        self.reveal_around(self.ax, self.ay);
        self
    }

    fn reveal_around(&mut self, cx: usize, cy: usize) {
        if !self.has_fog() { return; }
        let mut visited = vec![false; self.w * self.h];
        let mut queue = VecDeque::new();
        let si = cy * self.w + cx;
        visited[si] = true;
        self.explored[si] = true;
        queue.push_back((cx, cy, 0u32));
        while let Some((x, y, dist)) = queue.pop_front() {
            if dist >= self.view_radius as u32 { continue; }
            for (ddx, ddy) in [(-1i32, 0), (1, 0), (0, -1i32), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)] {
                let nx = x as i32 + ddx;
                let ny = y as i32 + ddy;
                if nx < 0 || ny < 0 || (nx as usize) >= self.w || (ny as usize) >= self.h { continue; }
                let (ux, uy) = (nx as usize, ny as usize);
                let ni = uy * self.w + ux;
                if visited[ni] { continue; }
                visited[ni] = true;
                self.explored[ni] = true;
                let c = self.cell(ux, uy);
                if !matches!(c, Cell::Wall | Cell::DoorLocked | Cell::Ball | Cell::Box_) {
                    queue.push_back((ux, uy, dist + 1));
                }
            }
        }
    }

    fn nearest_frontier(&self) -> Option<(usize, usize)> {
        if !self.has_fog() { return None; }
        let mut visited = vec![false; self.w * self.h];
        let mut queue = VecDeque::new();
        let si = self.ay * self.w + self.ax;
        visited[si] = true;
        queue.push_back(si);
        while let Some(idx) = queue.pop_front() {
            let (cx, cy) = (idx % self.w, idx / self.w);
            for (ddx, ddy) in [(-1i32, 0), (1, 0), (0, -1i32), (0, 1)] {
                let nx = cx as i32 + ddx;
                let ny = cy as i32 + ddy;
                if nx >= 0 && ny >= 0 && (nx as usize) < self.w && (ny as usize) < self.h {
                    if !self.explored[(ny as usize) * self.w + (nx as usize)] {
                        return Some((cx, cy));
                    }
                }
            }
            for (ddx, ddy) in [(-1i32, 0), (1, 0), (0, -1i32), (0, 1)] {
                let nx = cx as i32 + ddx;
                let ny = cy as i32 + ddy;
                if nx < 0 || ny < 0 || nx >= self.w as i32 || ny >= self.h as i32 { continue; }
                let (ux, uy) = (nx as usize, ny as usize);
                let ni = uy * self.w + ux;
                if !visited[ni] && self.passable(ux, uy) {
                    visited[ni] = true;
                    queue.push_back(ni);
                }
            }
        }
        None
    }

    fn find_all(&self, target: Cell) -> Vec<(usize, usize)> {
        let mut result = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                if self.is_explored(x, y) && self.cell(x, y) == target {
                    result.push((x, y));
                }
            }
        }
        result
    }

    fn find_first(&self, target: Cell) -> Option<(usize, usize)> {
        for y in 0..self.h {
            for x in 0..self.w {
                if self.cell(x, y) == target {
                    return Some((x, y));
                }
            }
        }
        None
    }

    fn find_adjacent_passable(&self, tx: usize, ty: usize) -> Option<(usize, usize)> {
        for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1i32), (0, 1)] {
            let nx = tx as i32 + dx;
            let ny = ty as i32 + dy;
            if nx >= 0 && ny >= 0 && (nx as usize) < self.w && (ny as usize) < self.h {
                let (nx, ny) = (nx as usize, ny as usize);
                if self.passable(nx, ny) {
                    return Some((nx, ny));
                }
            }
        }
        None
    }

    // ── Generators ──────────────────────────────────────────────────────

    fn new_empty(w: usize, h: usize) -> Self {
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        cells[(h - 2) * w + (w - 2)] = Cell::Goal;
        Grid { w, h, cells, ax: 1, ay: 1, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_doorkey(size: usize, seed: u64) -> Self {
        let mut rng = Rng(seed);
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);

        let wall_x = w / 2;
        for y in 1..h - 1 { cells[y * w + wall_x] = Cell::Wall; }

        let door_y = 1 + (rng.next() as usize % (h - 2));
        cells[door_y * w + wall_x] = Cell::DoorLocked;

        let (kx, ky) = rng.find_empty(&cells, w, h, 1, wall_x, 1, h - 1);
        cells[ky * w + kx] = Cell::Key;

        let (gx, gy) = rng.find_empty(&cells, w, h, wall_x + 1, w - 1, 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, wall_x, 1, h - 1);

        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_distshift(size: usize, seed: u64, strip_row: usize, gap_col: usize) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);

        let row = strip_row.clamp(1, h - 2);
        for x in 1..w - 1 { cells[row * w + x] = Cell::Lava; }
        let gap = gap_col.clamp(1, w - 2);
        cells[row * w + gap] = Cell::Empty;

        let mut rng = Rng(seed);
        let (gx, gy) = rng.find_empty(&cells, w, h, 1, w - 1, row + 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, w - 1, 1, row);
        Grid { w, h, cells, ax, ay, adir: Dir::South, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_crossing(size: usize, seed: u64, num_crossings: usize, use_lava: bool) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);
        let obstacle = if use_lava { Cell::Lava } else { Cell::Wall };

        for i in 0..num_crossings {
            let is_horizontal = i % 2 == 0;
            if is_horizontal {
                let y = 2 + (rng.next() as usize % (h - 4));
                for x in 1..w - 1 { cells[y * w + x] = obstacle; }
                let gap = 1 + (rng.next() as usize % (w - 2));
                cells[y * w + gap] = Cell::Empty;
            } else {
                let x = 2 + (rng.next() as usize % (w - 4));
                for y in 1..h - 1 { cells[y * w + x] = obstacle; }
                let gap = 1 + (rng.next() as usize % (h - 2));
                cells[gap * w + x] = Cell::Empty;
            }
        }

        cells[(h - 2) * w + (w - 2)] = Cell::Goal;
        cells[1 * w + 1] = Cell::Empty;
        Grid { w, h, cells, ax: 1, ay: 1, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_fourrooms(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        let mx = w / 2;
        let my = h / 2;
        for y in 1..h - 1 { cells[y * w + mx] = Cell::Wall; }
        for x in 1..w - 1 { cells[my * w + x] = Cell::Wall; }

        // Gaps in each wall segment
        let g1 = 1 + (rng.next() as usize % (my - 1));
        cells[g1 * w + mx] = Cell::Empty;
        let g2 = my + 1 + (rng.next() as usize % (h - my - 2));
        cells[g2 * w + mx] = Cell::Empty;
        let g3 = 1 + (rng.next() as usize % (mx - 1));
        cells[my * w + g3] = Cell::Empty;
        let g4 = mx + 1 + (rng.next() as usize % (w - mx - 2));
        cells[my * w + g4] = Cell::Empty;

        let (gx, gy) = rng.find_empty(&cells, w, h, mx + 1, w - 1, my + 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        Grid { w, h, cells, ax: 1, ay: 1, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_multiroom(size: usize, seed: u64, num_rooms: usize) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        let room_count = num_rooms.min(w / 3);
        let room_width = (w - 1) / room_count;

        for i in 1..room_count {
            let wall_x = i * room_width;
            if wall_x >= w - 1 { break; }
            for y in 1..h - 1 { cells[y * w + wall_x] = Cell::Wall; }
            let door_y = 1 + (rng.next() as usize % (h - 2));
            cells[door_y * w + wall_x] = Cell::DoorOpen;
        }

        let (gx, gy) = rng.find_empty(&cells, w, h, (room_count - 1) * room_width + 1, w - 1, 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        Grid { w, h, cells, ax: 1, ay: 1, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_lavagap(size: usize, seed: u64) -> Self {
        Self::new_distshift(size, seed, size / 2, 1 + (Rng(seed).next() as usize % (size - 2)))
    }

    fn new_keycorridor(size: usize, seed: u64, num_doors: usize) -> Self {
        let w = 4 + num_doors * 3;
        let h = size.max(5);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        for i in 0..num_doors {
            let wall_x = 3 + i * 3;
            if wall_x >= w - 1 { break; }
            for y in 1..h - 1 { cells[y * w + wall_x] = Cell::Wall; }
            let door_y = 1 + (rng.next() as usize % (h - 2));
            cells[door_y * w + wall_x] = Cell::DoorLocked;
            let ky = 1 + (rng.next() as usize % (h - 2));
            let kx = wall_x - 1;
            if cells[ky * w + kx] == Cell::Empty {
                cells[ky * w + kx] = Cell::Key;
            }
        }

        let (gx, gy) = rng.find_empty(&cells, w, h, w - 3, w - 1, 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        Grid { w, h, cells, ax: 1, ay: 1, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_fetch(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        let num_balls = 3;
        for _ in 0..num_balls {
            let (bx, by) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
            cells[by * w + bx] = Cell::Ball;
        }

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_dynamic_obstacles(size: usize, seed: u64, num_obstacles: usize) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        cells[(h - 2) * w + (w - 2)] = Cell::Goal;

        for _ in 0..num_obstacles {
            let (bx, by) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
            cells[by * w + bx] = Cell::Ball;
        }

        Grid { w, h, cells, ax: 1, ay: 1, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_locked_room(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        let wall_x = w / 3;
        let wall_x2 = 2 * w / 3;
        for y in 1..h - 1 {
            cells[y * w + wall_x] = Cell::Wall;
            cells[y * w + wall_x2] = Cell::Wall;
        }

        let d1y = 1 + (rng.next() as usize % (h - 2));
        cells[d1y * w + wall_x] = Cell::DoorOpen;

        let d2y = 1 + (rng.next() as usize % (h - 2));
        cells[d2y * w + wall_x2] = Cell::DoorLocked;

        let (kx, ky) = rng.find_empty(&cells, w, h, 1, wall_x, 1, h - 1);
        cells[ky * w + kx] = Cell::Key;

        let (gx, gy) = rng.find_empty(&cells, w, h, wall_x2 + 1, w - 1, 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        let (ax, ay) = rng.find_empty(&cells, w, h, wall_x + 1, wall_x2, 1, h - 1);
        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_unlock_pickup(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        let wall_x = w / 2;
        for y in 1..h - 1 { cells[y * w + wall_x] = Cell::Wall; }
        let door_y = 1 + (rng.next() as usize % (h - 2));
        cells[door_y * w + wall_x] = Cell::DoorLocked;

        let (kx, ky) = rng.find_empty(&cells, w, h, 1, wall_x, 1, h - 1);
        cells[ky * w + kx] = Cell::Key;

        let (bx, by) = rng.find_empty(&cells, w, h, wall_x + 1, w - 1, 1, h - 1);
        cells[by * w + bx] = Cell::Box_;

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, wall_x, 1, h - 1);
        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_put_near(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        let (bx, by) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
        cells[by * w + bx] = Cell::Ball;

        let (xx, xy) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
        cells[xy * w + xx] = Cell::Box_;

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_playground(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        // A few of each object
        for cell_type in [Cell::Key, Cell::Ball, Cell::Box_, Cell::Goal] {
            let count = if cell_type == Cell::Goal { 1 } else { 2 };
            for _ in 0..count {
                let (ox, oy) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
                cells[oy * w + ox] = cell_type;
            }
        }

        // Add a door pair
        let wall_x = w / 2;
        cells[3 * w + wall_x] = Cell::Wall;
        cells[4 * w + wall_x] = Cell::DoorLocked;
        cells[5 * w + wall_x] = Cell::Wall;

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_red_blue_door(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        let wall_x = w / 2;
        for y in 1..h - 1 { cells[y * w + wall_x] = Cell::Wall; }

        let d1y = 1 + (rng.next() as usize % (h / 2 - 1)).max(1);
        cells[d1y * w + wall_x] = Cell::DoorLocked;
        let d2y = h / 2 + 1 + (rng.next() as usize % (h / 2 - 2)).max(1);
        if d2y < h - 1 { cells[d2y * w + wall_x] = Cell::DoorOpen; }

        let (gx, gy) = rng.find_empty(&cells, w, h, wall_x + 1, w - 1, 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        let (kx, ky) = rng.find_empty(&cells, w, h, 1, wall_x, 1, h - 1);
        cells[ky * w + kx] = Cell::Key;

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, wall_x, 1, h - 1);
        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_memory(size: usize, seed: u64) -> Self {
        let (w, h) = (size, size);
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        // Two objects to choose from
        let (b1x, b1y) = rng.find_empty(&cells, w, h, 1, w / 2, 1, h - 1);
        cells[b1y * w + b1x] = Cell::Ball;

        let (b2x, b2y) = rng.find_empty(&cells, w, h, w / 2, w - 1, 1, h - 1);
        cells[b2y * w + b2x] = Cell::Key;

        let (ax, ay) = rng.find_empty(&cells, w, h, 1, w - 1, 1, h - 1);
        Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    fn new_obstructed_maze(size: usize, seed: u64) -> Self {
        let (w, h) = (size.max(9), size.max(9));
        let mut cells = vec![Cell::Empty; w * h];
        Self::add_walls(&mut cells, w, h);
        let mut rng = Rng(seed);

        // Create maze-like corridors
        let third = w / 3;
        let two_third = 2 * w / 3;
        for y in 1..h - 1 { cells[y * w + third] = Cell::Wall; }
        for y in 1..h - 1 { cells[y * w + two_third] = Cell::Wall; }

        let g1 = 1 + (rng.next() as usize % (h - 2));
        cells[g1 * w + third] = Cell::DoorLocked;
        let g2 = 1 + (rng.next() as usize % (h - 2));
        cells[g2 * w + two_third] = Cell::DoorLocked;

        let (k1x, k1y) = rng.find_empty(&cells, w, h, 1, third, 1, h - 1);
        cells[k1y * w + k1x] = Cell::Key;

        let (k2x, k2y) = rng.find_empty(&cells, w, h, third + 1, two_third, 1, h - 1);
        cells[k2y * w + k2x] = Cell::Key;

        // Add obstructing balls
        let (bx, by) = rng.find_empty(&cells, w, h, 1, third, 1, h - 1);
        cells[by * w + bx] = Cell::Ball;

        let (gx, gy) = rng.find_empty(&cells, w, h, two_third + 1, w - 1, 1, h - 1);
        cells[gy * w + gx] = Cell::Goal;

        Grid { w, h, cells, ax: 1, ay: 1, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn add_walls(cells: &mut [Cell], w: usize, h: usize) {
        for x in 0..w { cells[x] = Cell::Wall; cells[(h - 1) * w + x] = Cell::Wall; }
        for y in 0..h { cells[y * w] = Cell::Wall; cells[y * w + (w - 1)] = Cell::Wall; }
    }

    fn from_layout(cell_data: &[Vec<&str>], agent: (usize, usize)) -> Result<Self, String> {
        let h = cell_data.len();
        if h < 3 { return Err("grid too small".into()); }
        let w = cell_data[0].len();
        if w < 3 { return Err("grid too small".into()); }

        let mut cells = vec![Cell::Empty; w * h];
        for (y, row) in cell_data.iter().enumerate() {
            if row.len() != w { return Err("ragged grid".into()); }
            for (x, ch) in row.iter().enumerate() {
                cells[y * w + x] = Cell::from_char(ch);
            }
        }

        let (ax, ay) = if agent.0 < w && agent.1 < h { agent } else { return Err("agent out of bounds".into()); };
        cells[ay * w + ax] = Cell::Empty;

        Ok(Grid { w, h, cells, ax, ay, adir: Dir::East, carrying: None, done: false, reward: 0.0, steps: 0, explored: Vec::new(), view_radius: 0 })
    }

    // ── Rendering ───────────────────────────────────────────────────────

    fn render(&self) -> String {
        let mut lines = Vec::with_capacity(self.h);
        for y in 0..self.h {
            let mut row = String::with_capacity(self.w * 2);
            for x in 0..self.w {
                if x == self.ax && y == self.ay {
                    row.push(match self.adir { Dir::North=>'△', Dir::East=>'▷', Dir::South=>'▽', Dir::West=>'◁' });
                } else if !self.is_explored(x, y) {
                    row.push('?');
                } else {
                    row.push(match self.cell(x, y) {
                        Cell::Empty=>'·', Cell::Wall=>'█', Cell::Key=>'K', Cell::DoorLocked=>'D',
                        Cell::DoorOpen=>'_', Cell::Goal=>'G', Cell::Lava=>'~', Cell::Ball=>'o', Cell::Box_=>'□',
                    });
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
                    let arrow = match self.adir { Dir::North=>'▲', Dir::East=>'▶', Dir::South=>'▼', Dir::West=>'◀' };
                    row.push_str(&format!("\x1b[91;48;5;254m{arrow} \x1b[0m"));
                } else if !self.is_explored(x, y) {
                    row.push_str("\x1b[48;5;234m  \x1b[0m");
                } else {
                    row.push_str(match self.cell(x, y) {
                        Cell::Empty => "\x1b[48;5;254m  \x1b[0m",
                        Cell::Wall => "\x1b[48;5;239m  \x1b[0m",
                        Cell::Key => "\x1b[93;48;5;254m🔑\x1b[0m",
                        Cell::DoorLocked => "\x1b[48;5;52m🚪\x1b[0m",
                        Cell::DoorOpen => "\x1b[48;5;22m  \x1b[0m",
                        Cell::Goal => "\x1b[48;5;28m🏁\x1b[0m",
                        Cell::Lava => "\x1b[48;5;202m🔥\x1b[0m",
                        Cell::Ball => "\x1b[48;5;254m🔵\x1b[0m",
                        Cell::Box_ => "\x1b[48;5;254m📦\x1b[0m",
                    });
                }
            }
            lines.push(row);
        }
        lines.join("\n")
    }

    fn cell_char(&self, x: usize, y: usize) -> &'static str {
        if x == self.ax && y == self.ay { return "A"; }
        if !self.is_explored(x, y) { return "?"; }
        self.cell(x, y).to_char()
    }

    fn observation(&self) -> serde_json::Value {
        let cells: Vec<Vec<&str>> = (0..self.h)
            .map(|y| (0..self.w).map(|x| self.cell_char(x, y)).collect())
            .collect();

        let keys = self.find_all(Cell::Key);
        let doors_locked = self.find_all(Cell::DoorLocked);
        let doors_open = self.find_all(Cell::DoorOpen);
        let goals = self.find_all(Cell::Goal);
        let balls = self.find_all(Cell::Ball);
        let boxes = self.find_all(Cell::Box_);

        let explored_pct = if self.has_fog() {
            let explored = self.explored.iter().filter(|&&e| e).count();
            (explored as f64 / (self.w * self.h) as f64 * 100.0).round()
        } else { 100.0 };

        let mut obs = serde_json::json!({
            "agent_pos": [self.ax, self.ay],
            "agent_dir": self.adir.name(),
            "carrying": self.carrying.map(|c| c.to_char()),
            "carrying_key": self.carrying == Some(Cell::Key),
            "keys": keys, "doors_locked": doors_locked, "doors_open": doors_open,
            "goals": goals, "balls": balls, "boxes": boxes,
            "done": self.done, "reward": self.reward,
            "step_count": self.steps, "grid_size": [self.w, self.h],
            "explored_pct": explored_pct, "view_radius": self.view_radius,
            "cells": cells, "render": self.render(), "render_ansi": self.render_ansi(),
        });
        if self.has_fog() {
            obs["explored_mask"] = serde_json::json!(self.explored);
        }
        obs
    }

    // ── Navigation ──────────────────────────────────────────────────────

    fn passable(&self, x: usize, y: usize) -> bool {
        self.is_explored(x, y) && !matches!(self.cell(x, y), Cell::Wall | Cell::DoorLocked | Cell::Lava | Cell::Ball | Cell::Box_)
    }

    fn faced_cell(&self) -> Option<(usize, usize)> {
        let (dx, dy): (i32, i32) = match self.adir {
            Dir::North => (0, -1),
            Dir::East => (1, 0),
            Dir::South => (0, 1),
            Dir::West => (-1, 0),
        };
        let nx = self.ax as i32 + dx;
        let ny = self.ay as i32 + dy;
        if nx >= 0 && ny >= 0 && (nx as usize) < self.w && (ny as usize) < self.h {
            Some((nx as usize, ny as usize))
        } else {
            None
        }
    }

    fn face_toward(&mut self, tx: usize, ty: usize) {
        let dx = tx as i32 - self.ax as i32;
        let dy = ty as i32 - self.ay as i32;
        if dx.abs() >= dy.abs() {
            self.adir = if dx > 0 { Dir::East } else { Dir::West };
        } else {
            self.adir = if dy > 0 { Dir::South } else { Dir::North };
        }
    }

    fn passable_or_target(&self, x: usize, y: usize, tx: usize, ty: usize) -> bool {
        if x == tx && y == ty { return true; }
        self.passable(x, y)
    }

    fn bfs_path(&self, sx: usize, sy: usize, tx: usize, ty: usize) -> Option<Vec<(usize, usize)>> {
        self.bfs_path_with(sx, sy, tx, ty, false)
    }

    fn bfs_path_with(&self, sx: usize, sy: usize, tx: usize, ty: usize, allow_target: bool) -> Option<Vec<(usize, usize)>> {
        if sx == tx && sy == ty { return Some(vec![(tx, ty)]); }
        let mut visited = vec![false; self.w * self.h];
        let mut parent: Vec<Option<usize>> = vec![None; self.w * self.h];
        let mut queue = VecDeque::new();
        let si = sy * self.w + sx;
        visited[si] = true;
        queue.push_back(si);

        while let Some(idx) = queue.pop_front() {
            let (cx, cy) = (idx % self.w, idx / self.w);
            if cx == tx && cy == ty {
                let mut path = vec![];
                let mut cur = idx;
                loop { path.push((cur % self.w, cur / self.w)); match parent[cur] { Some(p) => cur = p, None => break } }
                path.reverse();
                return Some(path);
            }
            for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1i32), (0, 1)] {
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if nx < 0 || ny < 0 || nx >= self.w as i32 || ny >= self.h as i32 { continue; }
                let (nx, ny) = (nx as usize, ny as usize);
                let ni = ny * self.w + nx;
                let ok = if allow_target { self.passable_or_target(nx, ny, tx, ty) } else { self.passable(nx, ny) };
                if !visited[ni] && ok {
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
            for &(px, py) in &path {
                self.reveal_around(px, py);
            }
            if path.len() > 1 {
                let dest = path[path.len() - 1];
                self.ax = dest.0;
                self.ay = dest.1;
                self.steps += path.len() - 1;
            }
            true
        } else { false }
    }

    fn navigate_adjacent(&mut self, tx: usize, ty: usize) -> bool {
        if let Some(adj) = self.find_adjacent_passable(tx, ty) {
            self.navigate_to(adj.0, adj.1)
        } else { false }
    }

    fn check_done_on_goal(&mut self) {
        if self.cell(self.ax, self.ay) == Cell::Goal {
            self.done = true;
            self.reward = 1.0 - 0.9 * (self.steps as f64 / (self.w * self.h * 4) as f64).min(1.0);
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
    fn find_empty(&mut self, cells: &[Cell], w: usize, _h: usize, x0: usize, x1: usize, y0: usize, y1: usize) -> (usize, usize) {
        let xr = (x1 - x0).max(1);
        let yr = (y1 - y0).max(1);
        loop {
            let x = x0 + (self.next() as usize % xr);
            let y = y0 + (self.next() as usize % yr);
            if cells[y * w + x] == Cell::Empty { return (x, y); }
        }
    }
}

// ---------------------------------------------------------------------------
// Port
// ---------------------------------------------------------------------------

pub struct GridWorldPort {
    spec: PortSpec,
    state: Mutex<Option<Grid>>,
    seed_counter: Mutex<u64>,
}

impl GridWorldPort {
    pub fn new() -> Self {
        Self { spec: build_spec(), state: Mutex::new(None), seed_counter: Mutex::new(42) }
    }

    fn next_seed(&self) -> u64 {
        let mut counter = self.seed_counter.lock().unwrap();
        *counter = counter.wrapping_add(1);
        *counter ^ 0xDEAD_BEEF_CAFE_BABE
    }
}

impl Default for GridWorldPort {
    fn default() -> Self { Self::new() }
}

impl Port for GridWorldPort {
    fn spec(&self) -> &PortSpec { &self.spec }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "reset" => self.do_reset(&input),
            "scan" => self.do_scan(&input),
            "go_to" => self.do_go_to(&input),
            "go_to_key" => self.do_go_to_cell(Cell::Key, false),
            "go_to_door" => self.do_go_to_cell(Cell::DoorLocked, true),
            "go_to_goal" => self.do_go_to_goal(),
            "go_to_ball" => self.do_go_to_cell(Cell::Ball, true),
            "go_to_box" => self.do_go_to_cell(Cell::Box_, true),
            "pickup" => self.do_pickup(),
            "drop" => self.do_drop(),
            "toggle" => self.do_toggle(),
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
            "reset"|"scan"|"go_to"|"go_to_key"|"go_to_door"|"go_to_goal"|"go_to_ball"|"go_to_box"|"pickup"|"drop"|"toggle" => Ok(()),
            other => Err(PortError::Validation(format!("unknown capability: {other}"))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState { PortLifecycleState::Active }
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

impl GridWorldPort {
    fn with_grid<F, T>(&self, f: F) -> soma_port_sdk::Result<T>
    where F: FnOnce(&mut Grid) -> soma_port_sdk::Result<T> {
        let mut lock = self.state.lock().unwrap();
        match lock.as_mut() {
            Some(grid) => f(grid),
            None => Err(PortError::ExternalError("no active grid — call reset or scan first".into())),
        }
    }

    fn do_reset(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let grid = if let Some(cells_val) = input.get("cells") {
            let empty_row = Vec::new();
            let rows: Vec<Vec<&str>> = cells_val.as_array()
                .ok_or_else(|| PortError::Validation("cells must be array".into()))?
                .iter()
                .map(|row| row.as_array().unwrap_or(&empty_row).iter().map(|c| c.as_str().unwrap_or(".")).collect())
                .collect();
            let agent = input.get("agent").and_then(|v| v.as_array())
                .map(|a| (a.first().and_then(|v| v.as_u64()).unwrap_or(1) as usize,
                          a.get(1).and_then(|v| v.as_u64()).unwrap_or(1) as usize))
                .unwrap_or((1, 1));
            Grid::from_layout(&rows, agent).map_err(|e| PortError::Validation(e))?
        } else {
            let size = input.get("size").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
            if !(3..=25).contains(&size) { return Err(PortError::Validation("size must be 3..25".into())); }
            let seed = input.get("seed").and_then(|v| v.as_u64()).unwrap_or_else(|| self.next_seed());
            let env = input.get("env").and_then(|v| v.as_str()).unwrap_or("doorkey");

            match env {
                "empty" => Grid::new_empty(size, size),
                "doorkey" => Grid::new_doorkey(size, seed),
                "distshift" => {
                    let strip = input.get("strip_row").and_then(|v| v.as_u64()).unwrap_or(size as u64 / 2) as usize;
                    let gap = input.get("gap_col").and_then(|v| v.as_u64()).unwrap_or(size as u64 / 2) as usize;
                    Grid::new_distshift(size, seed, strip, gap)
                }
                "crossing" => {
                    let n = input.get("num_crossings").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                    let lava = input.get("use_lava").and_then(|v| v.as_bool()).unwrap_or(true);
                    Grid::new_crossing(size, seed, n, lava)
                }
                "fourrooms" => Grid::new_fourrooms(size, seed),
                "multiroom" => {
                    let n = input.get("num_rooms").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
                    Grid::new_multiroom(size, seed, n)
                }
                "lavagap" => Grid::new_lavagap(size, seed),
                "keycorridor" => {
                    let n = input.get("num_doors").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
                    Grid::new_keycorridor(size, seed, n)
                }
                "fetch" => Grid::new_fetch(size, seed),
                "dynamic_obstacles" => {
                    let n = input.get("num_obstacles").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
                    Grid::new_dynamic_obstacles(size, seed, n)
                }
                "locked_room" => Grid::new_locked_room(size, seed),
                "unlock" => Grid::new_doorkey(size, seed), // same structure, different goal
                "unlock_pickup" => Grid::new_unlock_pickup(size, seed),
                "blocked_unlock_pickup" => Grid::new_unlock_pickup(size, seed), // similar
                "put_near" => Grid::new_put_near(size, seed),
                "playground" => Grid::new_playground(size, seed),
                "red_blue_door" => Grid::new_red_blue_door(size, seed),
                "memory" => Grid::new_memory(size, seed),
                "obstructed_maze" | "obstructed_maze_full" | "obstructed_maze_dlhb" => Grid::new_obstructed_maze(size, seed),
                _ => Grid::new_doorkey(size, seed),
            }
        };
        let vr = input.get("view_radius").and_then(|v| v.as_u64()).unwrap_or(DEFAULT_VIEW_RADIUS as u64) as usize;
        let mut grid = grid.with_fog(vr);
        if let Some(mask) = input.get("explored_mask").and_then(|v| v.as_array()) {
            for (i, v) in mask.iter().enumerate() {
                if i < grid.explored.len() && v.as_bool().unwrap_or(false) {
                    grid.explored[i] = true;
                }
            }
        }
        let obs = grid.observation();
        *self.state.lock().unwrap() = Some(grid);
        Ok(obs)
    }

    fn do_scan(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        { let lock = self.state.lock().unwrap(); if lock.is_none() { drop(lock); self.do_reset(input)?; } }
        self.with_grid(|grid| { grid.steps += 1; Ok(grid.observation()) })
    }

    fn do_go_to(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let target = input.get("target").and_then(|v| v.as_str()).unwrap_or("goal");
        match target {
            "key" => self.do_go_to_cell(Cell::Key, false),
            "door" => self.do_go_to_cell(Cell::DoorLocked, true),
            "goal" => self.do_go_to_goal(),
            "ball" => self.do_go_to_cell(Cell::Ball, true),
            "box" => self.do_go_to_cell(Cell::Box_, true),
            _ => {
                if let Some(pos) = input.get("target").and_then(|v| v.as_array()) {
                    let x = pos.first().and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    let y = pos.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    self.with_grid(|grid| {
                        if grid.done { return Ok(grid.observation()); }
                        if grid.navigate_to(x, y) { grid.check_done_on_goal(); Ok(grid.observation()) }
                        else { grid.steps += 1; Err(PortError::ExternalError(format!("no path to [{x},{y}]"))) }
                    })
                } else {
                    Err(PortError::Validation(format!("unknown target: {target}")))
                }
            }
        }
    }

    fn do_go_to_cell(&self, target: Cell, adjacent: bool) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done { return Ok(grid.observation()); }
            let no_locked_doors = grid.find_first(Cell::DoorLocked).is_none();
            if target == Cell::Key && (grid.carrying == Some(Cell::Key) || no_locked_doors) {
                grid.steps += 1;
                return Ok(grid.observation());
            }
            if target == Cell::DoorLocked && no_locked_doors {
                grid.steps += 1;
                return Ok(grid.observation());
            }
            let positions = grid.find_all(target);
            if !positions.is_empty() {
                for (tx, ty) in &positions {
                    let ok = if adjacent { grid.navigate_adjacent(*tx, *ty) } else { grid.navigate_to(*tx, *ty) };
                    if ok {
                        if adjacent { grid.face_toward(*tx, *ty); }
                        grid.check_done_on_goal();
                        return Ok(grid.observation());
                    }
                }
            }
            if let Some((fx, fy)) = grid.nearest_frontier() {
                if !(fx == grid.ax && fy == grid.ay) {
                    grid.navigate_to(fx, fy);
                }
            }
            grid.steps += 1;
            Err(PortError::ExternalError(format!("no {:?} visible", target)))
        })
    }

    fn do_go_to_goal(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done { return Ok(grid.observation()); }
            let goals = grid.find_all(Cell::Goal);
            if !goals.is_empty() {
                for (gx, gy) in &goals {
                    if grid.navigate_to(*gx, *gy) {
                        grid.done = true;
                        grid.reward = 1.0 - 0.9 * (grid.steps as f64 / (grid.w * grid.h * 4) as f64).min(1.0);
                        return Ok(grid.observation());
                    }
                }
            }
            if let Some((fx, fy)) = grid.nearest_frontier() {
                if !(fx == grid.ax && fy == grid.ay) {
                    grid.navigate_to(fx, fy);
                }
            }
            grid.steps += 1;
            Err(PortError::ExternalError("no goal visible".into()))
        })
    }

    fn do_pickup(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done { return Ok(grid.observation()); }
            grid.steps += 1;
            if grid.carrying.is_some() {
                return Ok(grid.observation());
            }
            let here = grid.cell(grid.ax, grid.ay);
            if here.is_pickable() {
                grid.carrying = Some(here);
                grid.set_cell(grid.ax, grid.ay, Cell::Empty);
                grid.reveal_around(grid.ax, grid.ay);
                return Ok(grid.observation());
            }
            if let Some((fx, fy)) = grid.faced_cell() {
                let faced = grid.cell(fx, fy);
                if faced.is_pickable() {
                    grid.carrying = Some(faced);
                    grid.set_cell(fx, fy, Cell::Empty);
                    grid.reveal_around(grid.ax, grid.ay);
                    return Ok(grid.observation());
                }
            }
            if grid.find_first(Cell::DoorLocked).is_none() {
                return Ok(grid.observation());
            }
            Err(PortError::ExternalError("nothing to pick up here".into()))
        })
    }

    fn do_drop(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done { return Ok(grid.observation()); }
            grid.steps += 1;
            match grid.carrying.take() {
                Some(obj) => {
                    if grid.cell(grid.ax, grid.ay) == Cell::Empty {
                        grid.set_cell(grid.ax, grid.ay, obj);
                        return Ok(grid.observation());
                    }
                    if let Some((fx, fy)) = grid.faced_cell() {
                        if grid.cell(fx, fy) == Cell::Empty {
                            grid.set_cell(fx, fy, obj);
                            return Ok(grid.observation());
                        }
                    }
                    grid.carrying = Some(obj);
                    Err(PortError::ExternalError("no empty cell to drop into".into()))
                }
                None => Err(PortError::ExternalError("not carrying anything".into())),
            }
        })
    }

    fn do_toggle(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.with_grid(|grid| {
            if grid.done { return Ok(grid.observation()); }
            grid.steps += 1;

            let doors = grid.find_all(Cell::DoorLocked);
            if doors.is_empty() {
                return Ok(grid.observation());
            }
            let mut toggled = false;
            for (dx, dy) in &doors {
                let dist = (grid.ax as i32 - *dx as i32).unsigned_abs() as usize
                    + (grid.ay as i32 - *dy as i32).unsigned_abs() as usize;
                if dist <= 1 {
                    if grid.carrying == Some(Cell::Key) {
                        grid.set_cell(*dx, *dy, Cell::DoorOpen);
                        grid.carrying = None;
                        toggled = true;
                        break;
                    } else {
                        return Err(PortError::ExternalError("no key to unlock door".into()));
                    }
                }
            }
            if toggled { grid.reveal_around(grid.ax, grid.ay); Ok(grid.observation()) }
            else { Err(PortError::ExternalError("not adjacent to any locked door".into())) }
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
        output_schema: SchemaRef::object(serde_json::json!({"agent_pos":{"type":"array"},"done":{"type":"boolean"},"reward":{"type":"number"}})),
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
        port_id: PORT_ID.into(), name: "GridWorld".into(),
        version: semver::Version::new(0, 2, 0), kind: PortKind::Custom,
        description: "Grid world environments for MiniGrid benchmark comparison".into(),
        namespace: "gridworld".into(), trust_level: TrustLevel::BuiltIn,
        capabilities: vec![
            cap("reset", "Create grid from env type or custom layout", m),
            cap("scan", "Return current grid observation", SideEffectClass::ReadOnly),
            cap("go_to", "Navigate agent to target (key/door/goal/ball/box/[x,y])", m),
            cap("go_to_key", "Navigate agent to nearest key", m),
            cap("go_to_door", "Navigate agent adjacent to nearest locked door", m),
            cap("go_to_goal", "Navigate agent to goal (marks done)", m),
            cap("go_to_ball", "Navigate agent to nearest ball", m),
            cap("go_to_box", "Navigate agent to nearest box", m),
            cap("pickup", "Pick up object at current position (key/ball/box)", m),
            cap("drop", "Drop carried object at current position", m),
            cap("toggle", "Toggle adjacent door (consumes key)", m),
        ],
        input_schema: SchemaRef::any(), output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: m,
        latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["agent_pos".into(), "done".into(), "reward".into(), "carrying".into()],
        validation_rules: vec![], remote_exposure: false,
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
            let key = grid.find_first(Cell::Key).unwrap();
            assert!(grid.bfs_path(grid.ax, grid.ay, key.0, key.1).is_some(), "seed {seed}: key unreachable");
            let door = grid.find_first(Cell::DoorLocked).unwrap();
            let adj = grid.find_adjacent_passable(door.0, door.1).unwrap();
            assert!(grid.bfs_path(grid.ax, grid.ay, adj.0, adj.1).is_some(), "seed {seed}: door unreachable");
            grid.set_cell(door.0, door.1, Cell::DoorOpen);
            let goal = grid.find_first(Cell::Goal).unwrap();
            assert!(grid.bfs_path(door.0, door.1, goal.0, goal.1).is_some(), "seed {seed}: goal unreachable after door open");
        }
    }

    #[test]
    fn full_solve_sequence() {
        let port = GridWorldPort::new();
        let r = port.invoke("reset", serde_json::json!({"size": 8, "seed": 42, "view_radius": 100})).unwrap();
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

    #[test]
    fn distshift_solvable() {
        for seed in 1..=20 {
            let grid = Grid::new_distshift(8, seed, 4, 3);
            let goal = grid.find_first(Cell::Goal).unwrap();
            assert!(grid.bfs_path(grid.ax, grid.ay, goal.0, goal.1).is_some(), "seed {seed}: goal unreachable");
        }
    }

    #[test]
    fn env_generators() {
        let port = GridWorldPort::new();
        for env in ["empty", "doorkey", "distshift", "crossing", "fourrooms", "multiroom",
                     "lavagap", "fetch", "dynamic_obstacles", "locked_room", "unlock_pickup",
                     "put_near", "playground", "red_blue_door", "memory", "obstructed_maze"] {
            let r = port.invoke("reset", serde_json::json!({"size": 8, "seed": 42, "env": env})).unwrap();
            assert!(r.success, "env {env} failed to reset");
        }
    }

    #[test]
    fn ball_pickup_drop() {
        let port = GridWorldPort::new();
        port.invoke("reset", serde_json::json!({"size": 8, "seed": 1, "env": "fetch", "view_radius": 100})).unwrap();
        let r = port.invoke("go_to_ball", serde_json::json!({})).unwrap();
        assert!(r.success);
        let r = port.invoke("pickup", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert_eq!(r.structured_result["carrying"].as_str().unwrap(), "B");
        let r = port.invoke("drop", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["carrying"].is_null());
    }

    #[test]
    fn fog_of_war() {
        let port = GridWorldPort::new();
        let r = port.invoke("reset", serde_json::json!({"size": 12, "seed": 42, "view_radius": 2})).unwrap();
        assert!(r.success);
        let pct = r.structured_result["explored_pct"].as_f64().unwrap();
        assert!(pct < 50.0, "view_radius=2 on 12×12 should reveal <50%, got {pct}%");
        let cells = r.structured_result["cells"].as_array().unwrap();
        let has_fog = cells.iter().any(|row| row.as_array().unwrap().iter().any(|c| c.as_str().unwrap() == "?"));
        assert!(has_fog, "should have fog cells");
    }

    #[test]
    fn fog_exploration() {
        let port = GridWorldPort::new();
        port.invoke("reset", serde_json::json!({"size": 8, "seed": 42, "view_radius": 2})).unwrap();
        let r1 = port.invoke("scan", serde_json::json!({})).unwrap();
        let pct1 = r1.structured_result["explored_pct"].as_f64().unwrap();
        // go_to_key will either find visible key or explore frontier
        let _ = port.invoke("go_to_key", serde_json::json!({}));
        let r2 = port.invoke("scan", serde_json::json!({})).unwrap();
        let pct2 = r2.structured_result["explored_pct"].as_f64().unwrap();
        assert!(pct2 >= pct1, "exploration should reveal more: {pct1}% -> {pct2}%");
    }

    #[test]
    fn fog_blocked_by_wall() {
        let port = GridWorldPort::new();
        let r = port.invoke("reset", serde_json::json!({
            "cells": [["W","W","W","W","W","W","W"],["W",".",".","W",".",".","W"],["W",".",".","W",".","K","W"],["W",".",".","W",".",".","W"],["W","W","W","W","W","W","W"]],
            "agent": [1, 1], "view_radius": 5
        })).unwrap();
        assert!(r.success);
        let keys = r.structured_result["keys"].as_array().unwrap();
        assert!(keys.is_empty(), "key behind sealed wall should not be visible, got {:?}", keys);
    }

    #[test]
    fn custom_layout() {
        let port = GridWorldPort::new();
        let r = port.invoke("reset", serde_json::json!({
            "cells": [["W","W","W","W","W"],["W",".",".","G","W"],["W",".",".","L","W"],["W",".",".",".","W"],["W","W","W","W","W"]],
            "agent": [1, 3], "view_radius": 100
        })).unwrap();
        assert!(r.success);
        let r = port.invoke("go_to_goal", serde_json::json!({})).unwrap();
        assert!(r.success);
        assert!(r.structured_result["done"].as_bool().unwrap());
    }

    #[test]
    fn noop_skills_when_no_doors() {
        let port = GridWorldPort::new();
        port.invoke("reset", serde_json::json!({
            "cells": [["W","W","W","W","W","W","W","W"],["W",".",".",".",".",".",".","W"],["W",".",".",".",".",".",".","W"],["W",".",".",".",".",".","G","W"],["W","W","W","W","W","W","W","W"]],
            "agent": [1, 1], "view_radius": 100
        })).unwrap();
        let r = port.invoke("go_to_key", serde_json::json!({})).unwrap();
        assert!(r.success, "go_to_key should noop when no doors exist");
        let r = port.invoke("pickup", serde_json::json!({})).unwrap();
        assert!(r.success, "pickup should noop when no doors exist");
        let r = port.invoke("go_to_door", serde_json::json!({})).unwrap();
        assert!(r.success, "go_to_door should noop when no doors exist");
        let r = port.invoke("toggle", serde_json::json!({})).unwrap();
        assert!(r.success, "toggle should noop when no doors exist");
        let r = port.invoke("go_to_goal", serde_json::json!({})).unwrap();
        assert!(r.success, "should reach goal");
        assert!(r.structured_result["done"].as_bool().unwrap());
    }
}
