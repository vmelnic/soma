use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use soma_next::runtime::port::PortRuntime as _;
use soma_next::runtime::session::{SessionRuntime as _, StepResult};
use soma_next::types::belief::{Binding, Fact};
use soma_next::types::common::{CostClass, CostProfile, FactProvenance};
use soma_next::types::episode::{Episode, EpisodeOutcome, EpisodeStep};
use soma_next::types::goal::{
    ExplorationStrategy, GoalSource, GoalSourceType, GoalSpec, Objective, Priority,
};
use soma_next::types::observation::Observation;
use soma_next::types::port::InvocationContext;

use crate::vis::GridFrame;
use crate::world::WorldSpec;

#[derive(Debug, Serialize, Deserialize)]
pub struct TraceStep {
    pub step_index: usize,
    pub skill: String,
    pub success: bool,
    pub observation: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorldTrace {
    pub world: WorldSpec,
    pub solved: bool,
    pub plan_following: bool,
    pub step_count: usize,
    pub skills: Vec<String>,
    pub steps: Vec<TraceStep>,
}

pub struct RunResult {
    pub trace: WorldTrace,
    pub frames: Vec<GridFrame>,
    pub episode: Episode,
}

pub fn run_world(
    runtime: &mut soma_next::bootstrap::Runtime,
    world: &WorldSpec,
) -> Result<RunResult, String> {
    let mut reset_input = world.to_port_input();
    {
        let ws = runtime.world_state.lock().unwrap();
        let snap = ws.snapshot();
        let mask_key = format!("grid.{}.explored_mask", world.name);
        if let Some(mask) = snap.get(&mask_key) {
            reset_input["explored_mask"] = mask.clone();
        }
    }
    {
        let pr = runtime.port_runtime.lock().unwrap();
        pr.invoke("gridworld", "reset", reset_input, &InvocationContext::local())
            .map_err(|e| format!("reset grid: {e}"))?;
    }

    let goal = GoalSpec {
        goal_id: Uuid::new_v4(),
        source: GoalSource {
            source_type: GoalSourceType::Internal,
            identity: Some("minigrid-runner".to_string()),
            session_id: None,
            peer_id: None,
        },
        objective: Objective {
            description: world.goal_fingerprint().to_string(),
            structured: Some(world.to_port_input()),
        },
        constraints: Vec::new(),
        success_conditions: vec![
            soma_next::types::goal::SuccessCondition {
                description: "goal reached".to_string(),
                expression: serde_json::json!({"observation_field": "done", "equals": true}),
            },
        ],
        risk_budget: (world.size * world.size) as f64 / 10.0,
        latency_budget_ms: 300_000,
        resource_budget: (world.size * world.size) as f64,
        deadline: None,
        permissions_scope: vec!["read_only".to_string()],
        priority: Priority::Normal,
        max_steps: Some((world.size * world.size * 4) as u32),
        exploration: ExplorationStrategy::Greedy,
    };

    let mut session = runtime.session_controller
        .create_session(goal)
        .map_err(|e| format!("create_session: {e}"))?;

    let port_input = world.to_port_input();
    for (key, value) in port_input.as_object().unwrap() {
        session.belief.active_bindings.push(Binding {
            name: key.clone(),
            value: value.clone(),
            source: "goal_structured".to_string(),
            confidence: 1.0,
        });
    }

    let max_loop = world.size * world.size * 4;
    for _ in 0..max_loop {
        match runtime.session_controller.run_step(&mut session) {
            Ok(StepResult::Continue) => continue,
            Ok(_) => break,
            Err(_) => break,
        }
    }

    let skills: Vec<String> = session.trace.steps.iter()
        .map(|s| s.selected_skill.clone())
        .collect();
    let plan_following = session.working_memory.used_plan_following;
    let solved = session.trace.steps.iter().any(|s| {
        s.port_calls.iter().any(|pc| {
            pc.success && pc.structured_result.get("done")
                .and_then(|v: &serde_json::Value| v.as_bool()).unwrap_or(false)
        })
    });

    let mut trace_steps = Vec::new();
    let mut frames = Vec::new();
    let mut episode_steps = Vec::new();
    let mut observations = Vec::new();
    let now = Utc::now();
    let cost_profile = CostProfile {
        cpu_cost_class: CostClass::Negligible,
        memory_cost_class: CostClass::Negligible,
        io_cost_class: CostClass::Low,
        network_cost_class: CostClass::Negligible,
        energy_cost_class: CostClass::Negligible,
    };

    for (idx, step) in session.trace.steps.iter().enumerate() {
        let obs_raw = step.port_calls.first()
            .map(|pc| pc.structured_result.clone())
            .unwrap_or(json!(null));
        let step_success = step.port_calls.iter().any(|pc| pc.success);

        trace_steps.push(TraceStep {
            step_index: idx,
            skill: step.selected_skill.clone(),
            success: step_success,
            observation: obs_raw.clone(),
        });

        if let Some(cells) = obs_raw.get("cells").cloned() {
            frames.push(GridFrame {
                cells,
                label: step.selected_skill.clone(),
            });
        }

        let obs_summary = json!({
            "success": step_success,
            "done": obs_raw.get("done").cloned().unwrap_or(json!(false)),
        });
        let observation = Observation {
            observation_id: step.observation_id,
            session_id: session.session_id,
            skill_id: Some(step.selected_skill.clone()),
            port_calls: Vec::new(),
            raw_result: obs_summary.clone(),
            structured_result: obs_summary,
            effect_patch: None,
            success: step_success,
            failure_class: None,
            failure_detail: None,
            latency_ms: step.port_calls.first().map(|pc| pc.latency_ms).unwrap_or(0),
            resource_cost: cost_profile.clone(),
            confidence: if step_success { 0.95 } else { 0.3 },
            timestamp: step.timestamp,
        };
        observations.push(observation.clone());

        episode_steps.push(EpisodeStep {
            step_index: idx as u32,
            belief_summary: json!({}),
            candidates_considered: step.candidate_skills.clone(),
            predicted_scores: step.predicted_scores.iter().map(|s| s.score).collect(),
            selected_skill: step.selected_skill.clone(),
            observation,
            belief_patch: json!({}),
            progress_delta: step.progress_delta,
            critic_decision: step.critic_decision.clone(),
            timestamp: step.timestamp,
        });
    }

    if let Some(last_step) = session.trace.steps.last() {
        if let Some(mask) = last_step.port_calls.first()
            .and_then(|pc| pc.structured_result.get("explored_mask").cloned())
        {
            let fact = Fact {
                fact_id: format!("grid.{}.explored_mask", world.name),
                subject: "grid".to_string(),
                predicate: format!("{}.explored_mask", world.name),
                value: mask,
                confidence: 1.0,
                provenance: FactProvenance::Observed,
                timestamp: now,
                ttl_ms: None,
                prior_confidence: None,
                prediction_error: None,
            };
            let _ = runtime.world_state.lock().unwrap().add_fact(fact);
        }
    }

    let total_cost = episode_steps.len() as f64 * 0.01;
    let episode = Episode {
        episode_id: Uuid::new_v4(),
        goal_fingerprint: world.goal_fingerprint(),
        initial_belief_summary: json!({ "world": world.name }),
        steps: episode_steps,
        observations,
        outcome: if solved { EpisodeOutcome::Success } else { EpisodeOutcome::Failure },
        total_cost,
        success: solved,
        tags: vec!["minigrid_execution".to_string()],
        embedding: None,
        salience: if solved { 1.0 } else { 0.5 },
        world_state_context: serde_json::Value::Null,
        created_at: now,
    };

    Ok(RunResult {
        trace: WorldTrace {
            world: world.clone(),
            solved,
            plan_following,
            step_count: skills.len(),
            skills,
            steps: trace_steps,
        },
        frames,
        episode,
    })
}

