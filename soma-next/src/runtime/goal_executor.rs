use std::sync::{Arc, Mutex};

use crate::adapters::EpisodeMemoryAdapter;
use crate::errors::Result;
use crate::memory::checkpoint::SessionCheckpointStore;
use crate::memory::embedder::GoalEmbedder;
use crate::memory::episodes::EpisodeStore;
use crate::memory::routines::RoutineStore;
use crate::memory::schemas::SchemaStore;
use crate::memory::skill_stats::SharedSkillStats;
use crate::runtime::port::DefaultPortRuntime;
use crate::runtime::session::{SessionController, SessionRuntime, StepResult};
use crate::runtime::world_state::WorldStateStore;
use crate::types::session::ControlSession;

/// References to the stores needed to finalize an episode after a goal
/// reaches a terminal state.
pub struct EpisodeContext<'a> {
    pub episode_store: &'a Arc<Mutex<dyn EpisodeStore + Send>>,
    pub schema_store: &'a Arc<Mutex<dyn SchemaStore + Send>>,
    pub routine_store: &'a Arc<Mutex<dyn RoutineStore + Send>>,
    pub embedder: &'a Arc<dyn GoalEmbedder + Send + Sync>,
    pub world_state: &'a Arc<Mutex<dyn WorldStateStore + Send>>,
    /// Optional skill-stats store. When present, every finalized episode
    /// updates the per-skill EMA stats so the predictor sees calibrated
    /// latency/cost/success priors on subsequent goals.
    pub skill_stats: Option<&'a SharedSkillStats>,
    /// When present and a "brain" port is loaded, completed episodes are
    /// forwarded to the brain for SDM consolidation.
    pub port_runtime: Option<&'a Arc<Mutex<DefaultPortRuntime>>>,
}

/// Drive the 16-step control loop until it returns any non-Continue
/// `StepResult`. Errors from `run_step` bubble up — callers decide whether
/// to finalize an episode in that case.
pub fn run_loop(
    ctrl: &mut SessionController,
    session: &mut ControlSession,
) -> Result<StepResult> {
    run_loop_with_checkpoint(ctrl, session, None, 0)
}

/// Same as `run_loop` but persists a session checkpoint every
/// `checkpoint_every_n` step iterations. Pass `None` / `0` to disable.
/// A final checkpoint is written before returning the terminal result.
pub fn run_loop_with_checkpoint(
    ctrl: &mut SessionController,
    session: &mut ControlSession,
    checkpoint_store: Option<&SessionCheckpointStore>,
    checkpoint_every_n: u32,
) -> Result<StepResult> {
    let mut steps_since_save: u32 = 0;
    let result = loop {
        match ctrl.run_step(session)? {
            StepResult::Continue => {
                if let Some(store) = checkpoint_store
                    && checkpoint_every_n > 0
                {
                    steps_since_save = steps_since_save.saturating_add(1);
                    if steps_since_save >= checkpoint_every_n {
                        if let Err(e) = store.save(session) {
                            tracing::warn!(
                                session_id = %session.session_id,
                                error = %e,
                                "failed to write mid-run checkpoint"
                            );
                        }
                        steps_since_save = 0;
                    }
                }
                continue;
            }
            other => break other,
        }
    };
    if let Some(store) = checkpoint_store
        && let Err(e) = store.save(session)
    {
        tracing::warn!(
            session_id = %session.session_id,
            error = %e,
            "failed to write terminal checkpoint"
        );
    }
    Ok(result)
}

/// Store an episode and trigger learning. Skips when a successful session
/// used plan-following (the routine already captures that behavior, so the
/// episode would be noise). Does NOT gate on terminal status — callers
/// decide when to invoke this (e.g. after terminal StepResult or after a
/// run_step error that should still produce an episode).
pub fn finalize_episode(session: &ControlSession, ctx: &EpisodeContext<'_>) {
    let used_routine = session.working_memory.used_plan_following;
    let succeeded = session.status == crate::types::session::SessionStatus::Completed;
    let should_store = !used_routine || !succeeded;
    if !should_store {
        return;
    }

    let mut episode =
        crate::interfaces::cli::build_episode_from_session(session, Some(&**ctx.embedder));
    episode.world_state_context = ctx
        .world_state
        .lock()
        .ok()
        .map(|ws| ws.snapshot())
        .unwrap_or(serde_json::json!({}));
    let fingerprint = episode.goal_fingerprint.clone();

    // Self-calibration: feed the episode to the skill-stats EMA before it
    // gets stored / mined. Failures here are non-fatal — calibration is a
    // best-effort optimization, never a learning blocker.
    if let Some(stats) = ctx.skill_stats {
        if let Err(e) = stats.update_from_episode(&episode) {
            tracing::warn!(error = %e, "skill-stats update failed");
        }
        if let Err(e) = stats.save() {
            tracing::debug!(error = %e, "skill-stats save failed (non-fatal)");
        }
    }

    let episode_json = if ctx.port_runtime.is_some() {
        serde_json::to_value(&episode).ok()
    } else {
        None
    };

    let adapter =
        EpisodeMemoryAdapter::new(Arc::clone(ctx.episode_store), Arc::clone(ctx.embedder));
    if let Err(e) = adapter.store(episode) {
        tracing::warn!(error = %e, "failed to store episode from goal executor");
        return;
    }
    crate::interfaces::cli::attempt_learning(
        ctx.episode_store,
        ctx.schema_store,
        ctx.routine_store,
        &fingerprint,
        &**ctx.embedder,
    );

    if let (Some(port_rt), Some(ep_json)) = (ctx.port_runtime, episode_json) {
        crate::runtime::brain_fallback::consolidate_episode_to_brain(port_rt, &ep_json);
    }
}
