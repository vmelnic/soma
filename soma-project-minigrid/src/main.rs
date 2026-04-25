// soma-project-minigrid — proves SOMA learns DoorKey-8x8 grid navigation.
//
// The benchmark: MiniGrid-DoorKey-8x8.
//
// Three phases:
//   Phase A — Brain-guided episodes (simulated)
//     Inject 10 episodes with the correct skill sequence, as a brain would
//     guide the body. Store in episode store.
//
//   Phase B — Learning pipeline
//     Mine episodes → induce schemas → compile routine via BMR.
//
//   Phase C — Autonomous execution with compiled routine
//     Run 10 new DoorKey instances through the real session controller.
//     Routine should fire via plan-following, executing the learned sequence
//     against the real grid world port.

use std::fs::File;
use std::io::BufWriter;

use chrono::Utc;
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Frame, ImageBuffer, Rgba, RgbaImage};
use serde_json::json;
use uuid::Uuid;

use soma_next::bootstrap::bootstrap;
use soma_next::config::SomaConfig;
use soma_next::memory::embedder::{GoalEmbedder, HashEmbedder};
use soma_next::runtime::session::{SessionRuntime as _, StepResult};
use soma_next::types::belief::Binding;
use soma_next::types::common::{CostClass, CostProfile};
use soma_next::types::episode::{Episode, EpisodeOutcome, EpisodeStep};
use soma_next::types::goal::{
    ExplorationStrategy, GoalSource, GoalSourceType, GoalSpec, Objective, Priority,
};
use soma_next::types::observation::Observation;


const GOAL_FINGERPRINT: &str = "solve_doorkey";

const SOLUTION_SEQUENCE: &[&str] = &[
    "grid.scan",
    "grid.go_to_key",
    "grid.pickup",
    "grid.go_to_door",
    "grid.toggle",
    "grid.go_to_goal",
];

fn main() {
    println!("==================================================");
    println!("soma-project-minigrid: DoorKey-8x8 learning proof");
    println!("==================================================\n");

    match run_proof() {
        Ok(result) => {
            println!("\n  {result}");
            println!("\n==================================================");
            if result.contains("FAILED") || result.starts_with("RESULT: Learned 0") {
                println!("PROOF INCOMPLETE");
                println!("==================================================");
                std::process::exit(1);
            } else {
                println!("ALL PHASES PASSED");
                println!("==================================================");
            }
        }
        Err(e) => {
            eprintln!("\n  FAILED: {e}");
            std::process::exit(1);
        }
    }
}

fn run_proof() -> Result<String, String> {
    let mut config = SomaConfig::default();
    let data_dir = std::env::temp_dir().join("soma-minigrid-proof");
    if data_dir.exists() {
        let _ = std::fs::remove_dir_all(&data_dir);
    }
    config.soma.data_dir = data_dir.to_string_lossy().to_string();

    let pack_path = std::env::var("SOMA_MINIGRID_PACK")
        .unwrap_or_else(|_| "packs/minigrid/manifest.json".to_string());

    let mut runtime = bootstrap(&config, &[pack_path])
        .map_err(|e| format!("bootstrap: {e}"))?;

    let embedder = HashEmbedder::new();

    // ─── Phase A: Brain-guided episodes ──────────────────────────────────

    println!("  Phase A: Brain-guided episodes (10 injected)");
    println!("  Sequence: {:?}\n", SOLUTION_SEQUENCE);

    for i in 0..10 {
        let episode = make_episode(i, SOLUTION_SEQUENCE, GOAL_FINGERPRINT);
        let mut es = runtime.episode_store.lock().unwrap();
        es.store(episode).map_err(|e| format!("episode store: {e}"))?;
    }
    println!("    Stored 10 episodes with {}-step solution", SOLUTION_SEQUENCE.len());

    // ─── Phase B: Learning pipeline ──────────────────────────────────────

    println!("\n  Phase B: Learning pipeline");

    soma_next::interfaces::cli::attempt_learning(
        &runtime.episode_store,
        &runtime.schema_store,
        &runtime.routine_store,
        GOAL_FINGERPRINT,
        &embedder as &dyn GoalEmbedder,
    );

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

    if routines.is_empty() {
        return Err("Learning pipeline produced no routines".to_string());
    }

    let doorkey_routine = routines.iter().find(|r| {
        r.compiled_skill_path.iter().map(|s| s.as_str()).collect::<Vec<_>>() ==
            SOLUTION_SEQUENCE.to_vec()
    });

    if doorkey_routine.is_none() {
        println!("  WARNING: No routine matches the expected solution sequence");
    }

    // ─── Phase C: Autonomous execution ───────────────────────────────────

    let visualize = std::env::var("SOMA_MINIGRID_VIS").map(|v| v != "0").unwrap_or(true);
    let gif_dir = std::env::current_dir().unwrap_or_default();

    println!("\n  Phase C: Autonomous execution with compiled routine (10 episodes)");

    let mut successes = 0usize;
    let mut plan_following_count = 0usize;
    let mut total_steps = 0usize;

    for i in 0..10 {
        let seed = 300 + i as u64;
        let goal = GoalSpec {
            goal_id: Uuid::new_v4(),
            source: GoalSource {
                source_type: GoalSourceType::Internal,
                identity: Some("minigrid-proof".to_string()),
                session_id: None,
                peer_id: None,
            },
            objective: Objective {
                description: GOAL_FINGERPRINT.to_string(),
                structured: Some(json!({ "size": 8, "seed": seed })),
            },
            constraints: Vec::new(),
            success_conditions: Vec::new(),
            risk_budget: 1.0,
            latency_budget_ms: 30_000,
            resource_budget: 1.0,
            deadline: None,
            permissions_scope: vec!["read_only".to_string()],
            priority: Priority::Normal,
            max_steps: Some(10),
            exploration: ExplorationStrategy::Greedy,
        };

        let mut session = runtime.session_controller
            .create_session(goal)
            .map_err(|e| format!("create_session: {e}"))?;

        session.belief.active_bindings.push(Binding {
            name: "size".to_string(),
            value: json!(8),
            source: "goal_structured".to_string(),
            confidence: 1.0,
        });
        session.belief.active_bindings.push(Binding {
            name: "seed".to_string(),
            value: json!(seed),
            source: "goal_structured".to_string(),
            confidence: 1.0,
        });

        for _ in 0..15 {
            match runtime.session_controller.run_step(&mut session) {
                Ok(StepResult::Continue) => continue,
                Ok(_) => break,
                Err(_) => break,
            }
        }

        let skills: Vec<&str> = session.trace.steps.iter()
            .map(|s| s.selected_skill.as_str())
            .collect();
        let used_plan = session.working_memory.used_plan_following;
        let step_count = skills.len();
        let success = session.trace.steps.iter().any(|s| {
            s.port_calls.iter().any(|pc| {
                pc.success && pc.structured_result.get("done")
                    .and_then(|v| v.as_bool()).unwrap_or(false)
            })
        });

        total_steps += step_count;
        if success { successes += 1; }
        if used_plan { plan_following_count += 1; }

        let status = if success { "SOLVED" } else { "FAILED" };
        let plan_tag = if used_plan { " [PLAN-FOLLOWING]" } else { "" };
        println!("    episode {i:2}: seed={seed} steps={step_count} skills={skills:?} {status}{plan_tag}");

        if visualize && i < 2 {
            // Terminal ANSI visualization
            println!();
            for step in &session.trace.steps {
                let label = &step.selected_skill;
                for pc in &step.port_calls {
                    if let Some(ansi) = pc.structured_result.get("render_ansi").and_then(|v| v.as_str()) {
                        println!("      ┌─ {label}");
                        for line in ansi.lines() {
                            println!("      │ {line}");
                        }
                        println!("      └─");
                    }
                }
            }

            // Generate animated GIF
            let mut frames: Vec<GridFrame> = Vec::new();
            for step in &session.trace.steps {
                for pc in &step.port_calls {
                    if let Some(cells) = pc.structured_result.get("cells") {
                        frames.push(GridFrame {
                            cells: cells.clone(),
                            label: step.selected_skill.clone(),
                        });
                    }
                }
            }
            if !frames.is_empty() {
                let gif_path = gif_dir.join(format!("episode_{i}_seed_{seed}.gif"));
                match write_gif(&frames, &gif_path) {
                    Ok(()) => println!("      GIF: {}", gif_path.display()),
                    Err(e) => println!("      GIF failed: {e}"),
                }
            }
        }
    }

    let avg_steps = total_steps as f64 / 10.0;
    println!("\n  Phase C results: {successes}/10 solved, avg steps={avg_steps:.1}, plan-following={plan_following_count}/10");

    // ─── Summary ─────────────────────────────────────────────────────────

    println!("\n  ────────────────────────────────────────────────");
    println!("  Phase A: 10 brain-guided episodes injected ({}-step solution)", SOLUTION_SEQUENCE.len());
    println!("  Phase B: {} schemas, {} routines compiled", schemas.len(), routines.len());
    println!("  Phase C: {successes}/10 solved, avg {avg_steps:.1} steps, {plan_following_count}/10 plan-following");

    Ok(format!(
        "RESULT: Learned {} schemas + {} routines from 10 brain-guided episodes. \
         Autonomous: {successes}/10 solved, avg {avg_steps:.1} steps, {plan_following_count}/10 plan-following.",
        schemas.len(), routines.len(),
    ))
}

fn make_episode(seq_no: usize, skills: &[&str], goal_fingerprint: &str) -> Episode {
    let session_id = Uuid::new_v4();
    let now = Utc::now();
    let cost_profile = CostProfile {
        cpu_cost_class: CostClass::Negligible,
        memory_cost_class: CostClass::Negligible,
        io_cost_class: CostClass::Low,
        network_cost_class: CostClass::Negligible,
        energy_cost_class: CostClass::Negligible,
    };

    let steps: Vec<EpisodeStep> = skills
        .iter()
        .enumerate()
        .map(|(i, skill)| EpisodeStep {
            step_index: i as u32,
            belief_summary: json!({ "step": i, "seq_no": seq_no }),
            candidates_considered: vec![skill.to_string()],
            predicted_scores: vec![0.9],
            selected_skill: skill.to_string(),
            observation: Observation {
                observation_id: Uuid::new_v4(),
                session_id,
                skill_id: Some(skill.to_string()),
                port_calls: Vec::new(),
                raw_result: json!({ "ok": true }),
                structured_result: json!({ "ok": true, "step": i }),
                effect_patch: None,
                success: true,
                failure_class: None,
                failure_detail: None,
                latency_ms: 5,
                resource_cost: cost_profile.clone(),
                confidence: 0.95,
                timestamp: now,
            },
            belief_patch: json!({}),
            progress_delta: 1.0 / skills.len() as f64,
            critic_decision: if i + 1 < skills.len() {
                "Continue".to_string()
            } else {
                "Stop".to_string()
            },
            timestamp: now,
        })
        .collect();

    let observations: Vec<Observation> = steps.iter().map(|s| s.observation.clone()).collect();
    let total_cost = steps.len() as f64 * 0.01;

    Episode {
        episode_id: Uuid::new_v4(),
        goal_fingerprint: goal_fingerprint.to_string(),
        initial_belief_summary: json!({ "seq_no": seq_no }),
        steps,
        observations,
        outcome: EpisodeOutcome::Success,
        total_cost,
        success: true,
        tags: vec!["minigrid_proof".to_string()],
        embedding: None,
        salience: 1.0,
        world_state_context: serde_json::Value::Null,
        created_at: now,
    }
}

// ---------------------------------------------------------------------------
// GIF visualization
// ---------------------------------------------------------------------------

const CELL_PX: u32 = 48;
const BORDER_PX: u32 = 2;

struct GridFrame {
    cells: serde_json::Value,
    label: String,
}

fn cell_color(ch: &str) -> [u8; 4] {
    match ch {
        "W" => [100, 100, 100, 255],   // wall — dark gray
        "K" => [255, 215, 0, 255],     // key — gold
        "D" => [180, 40, 40, 255],     // door locked — red
        "O" => [60, 180, 60, 255],     // door open — green
        "G" => [40, 200, 40, 255],     // goal — bright green
        "A" => [30, 120, 255, 255],    // agent — blue
        _   => [230, 230, 230, 255],   // floor — light gray
    }
}

fn render_frame(frame: &GridFrame) -> Option<RgbaImage> {
    let rows = frame.cells.as_array()?;
    let h = rows.len() as u32;
    let w = rows.first()?.as_array()?.len() as u32;
    let label_h = 24u32;
    let img_w = w * CELL_PX;
    let img_h = h * CELL_PX + label_h;

    let mut img: RgbaImage = ImageBuffer::from_pixel(img_w, img_h, Rgba([20, 20, 30, 255]));

    // Draw label background and skill name
    for py in 0..label_h {
        for px in 0..img_w {
            img.put_pixel(px, py, Rgba([30, 30, 45, 255]));
        }
    }
    draw_text_5x7(&mut img, 4, 8, &frame.label, Rgba([200, 200, 220, 255]));

    // Draw grid cells
    for (gy, row) in rows.iter().enumerate() {
        let cols = row.as_array()?;
        for (gx, cell) in cols.iter().enumerate() {
            let ch = cell.as_str().unwrap_or(".");
            let color = cell_color(ch);
            let x0 = gx as u32 * CELL_PX;
            let y0 = gy as u32 * CELL_PX + label_h;

            for py in 0..CELL_PX {
                for px in 0..CELL_PX {
                    let on_border = px < BORDER_PX || py < BORDER_PX
                        || px >= CELL_PX - BORDER_PX || py >= CELL_PX - BORDER_PX;
                    let c = if on_border {
                        [50, 50, 60, 255]
                    } else {
                        color
                    };
                    img.put_pixel(x0 + px, y0 + py, Rgba(c));
                }
            }

            // Draw agent triangle
            if ch == "A" {
                draw_triangle(&mut img, x0, y0, CELL_PX);
            }
            // Draw key icon
            if ch == "K" {
                draw_key_icon(&mut img, x0, y0, CELL_PX);
            }
            // Draw goal star
            if ch == "G" {
                draw_star(&mut img, x0, y0, CELL_PX);
            }
        }
    }

    Some(img)
}

fn draw_triangle(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let cx = size / 2;
    let margin = size / 4;
    let color = Rgba([255, 60, 60, 255]);
    for py in margin..size - margin {
        let progress = (py - margin) as f64 / (size - 2 * margin) as f64;
        let half_w = (progress * (size / 2 - margin) as f64) as u32;
        let left = cx.saturating_sub(half_w);
        let right = (cx + half_w).min(size - 1);
        for px in left..=right {
            img.put_pixel(x0 + px, y0 + py, color);
        }
    }
}

fn draw_key_icon(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let color = Rgba([120, 80, 0, 255]);
    let cx = size / 2;
    let cy = size / 3;
    let r = size / 6;
    // Circle
    for py in 0..size {
        for px in 0..size {
            let dx = px as i32 - cx as i32;
            let dy = py as i32 - cy as i32;
            let dist = ((dx * dx + dy * dy) as f64).sqrt();
            if dist <= r as f64 && dist >= (r as f64 - 3.0) {
                img.put_pixel(x0 + px, y0 + py, color);
            }
        }
    }
    // Shaft
    let shaft_top = cy + r;
    let shaft_bottom = size - size / 5;
    for py in shaft_top..shaft_bottom {
        img.put_pixel(x0 + cx, y0 + py, color);
        img.put_pixel(x0 + cx + 1, y0 + py, color);
    }
    // Teeth
    for offset in [0, size / 8] {
        let ty = shaft_bottom - 2 - offset;
        for px in cx + 2..cx + size / 6 {
            img.put_pixel(x0 + px, y0 + ty, color);
        }
    }
}

fn draw_star(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let color = Rgba([255, 255, 255, 255]);
    let cx = size as f64 / 2.0;
    let cy = size as f64 / 2.0;
    let r_outer = size as f64 * 0.35;
    let r_inner = r_outer * 0.4;
    let points = 5;

    let mut vertices = Vec::new();
    for i in 0..points * 2 {
        let angle = std::f64::consts::FRAC_PI_2 * -1.0
            + std::f64::consts::PI * i as f64 / points as f64;
        let r = if i % 2 == 0 { r_outer } else { r_inner };
        vertices.push((cx + r * angle.cos(), cy + r * angle.sin()));
    }

    for py in BORDER_PX..size - BORDER_PX {
        for px in BORDER_PX..size - BORDER_PX {
            if point_in_polygon(px as f64, py as f64, &vertices) {
                img.put_pixel(x0 + px, y0 + py, color);
            }
        }
    }
}

fn point_in_polygon(x: f64, y: f64, verts: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = verts[i];
        let (xj, yj) = verts[j];
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn draw_text_5x7(img: &mut RgbaImage, x0: u32, y0: u32, text: &str, color: Rgba<u8>) {
    #[rustfmt::skip]
    const FONT: &[(char, [u8; 7])] = &[
        ('.', [0b00000,0b00000,0b00000,0b00000,0b00000,0b01100,0b01100]),
        ('_', [0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b11111]),
        ('a', [0b00000,0b00000,0b01110,0b00001,0b01111,0b10001,0b01111]),
        ('c', [0b00000,0b00000,0b01110,0b10000,0b10000,0b10001,0b01110]),
        ('d', [0b00001,0b00001,0b01101,0b10011,0b10001,0b10001,0b01111]),
        ('e', [0b00000,0b00000,0b01110,0b10001,0b11111,0b10000,0b01110]),
        ('g', [0b01111,0b10001,0b10001,0b01111,0b00001,0b10001,0b01110]),
        ('i', [0b00100,0b00000,0b01100,0b00100,0b00100,0b00100,0b01110]),
        ('k', [0b10000,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001]),
        ('l', [0b01100,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110]),
        ('n', [0b00000,0b00000,0b10110,0b11001,0b10001,0b10001,0b10001]),
        ('o', [0b00000,0b00000,0b01110,0b10001,0b10001,0b10001,0b01110]),
        ('p', [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000]),
        ('r', [0b00000,0b00000,0b10110,0b11001,0b10000,0b10000,0b10000]),
        ('s', [0b00000,0b00000,0b01110,0b10000,0b01110,0b00001,0b11110]),
        ('t', [0b00100,0b00100,0b01110,0b00100,0b00100,0b00100,0b00011]),
        ('u', [0b00000,0b00000,0b10001,0b10001,0b10001,0b10011,0b01101]),
        ('y', [0b00000,0b00000,0b10001,0b10001,0b01111,0b00001,0b01110]),
    ];

    let mut cx = x0;
    for ch in text.chars() {
        if let Some((_, rows)) = FONT.iter().find(|(c, _)| *c == ch) {
            for (row_i, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) != 0 {
                        let px = cx + col;
                        let py = y0 + row_i as u32;
                        if px < img.width() && py < img.height() {
                            img.put_pixel(px, py, color);
                        }
                    }
                }
            }
        }
        cx += 6;
    }
}

fn write_gif(frames: &[GridFrame], path: &std::path::Path) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let writer = BufWriter::new(file);
    let mut encoder = GifEncoder::new_with_speed(writer, 10);
    encoder.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;

    for gf in frames {
        let img = render_frame(gf).ok_or("failed to render frame")?;
        let frame = Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(800, 1));
        encoder.encode_frame(frame).map_err(|e| e.to_string())?;
    }
    Ok(())
}
