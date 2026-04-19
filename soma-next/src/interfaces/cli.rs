use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::path::PathBuf;

use crate::bootstrap::Runtime;
use crate::errors::{Result, SomaError};
use crate::memory::checkpoint::SessionCheckpointStore;
use crate::memory::episodes::EpisodeStore;
use crate::memory::routines::RoutineStore;
use crate::memory::schemas::SchemaStore;
use crate::runtime::goal::{GoalInput, GoalRuntime};
use crate::runtime::port::PortRuntime;
use crate::runtime::session::{SessionRuntime, StepResult};
use crate::runtime::skill::SkillRuntime;
use crate::types::episode::{Episode, EpisodeOutcome, EpisodeStep};
use crate::types::goal::{GoalSource, GoalSourceType};
use crate::types::session::SessionStatus;

// ---------------------------------------------------------------------------
// CLI commands
// ---------------------------------------------------------------------------

/// All commands the CLI supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CliCommand {
    /// Submit and run a goal.
    Run { goal: String },
    /// Inspect a specific session.
    Inspect { session_id: String },
    /// Restore a session from a disk checkpoint and resume execution.
    Restore { session_id: String },
    /// List all sessions.
    ListSessions,
    /// List loaded packs.
    ListPacks,
    /// List available skills.
    ListSkills,
    /// Show runtime metrics. Optional format: "text" (default), "json", "prometheus".
    Metrics { format: Option<String> },
    /// Verify Ed25519 signature of a port library.
    VerifyPort { path: String },
    /// Dump runtime state as structured JSON for LLM context integration.
    /// Sections: "full", "belief", "episodes", "schemas", "routines",
    /// "sessions", "skills", "ports", "packs", "metrics".
    Dump { sections: Vec<String> },
    /// Start interactive REPL mode.
    Repl,
}

// ---------------------------------------------------------------------------
// CliRunner trait
// ---------------------------------------------------------------------------

/// Trait for CLI command execution.
pub trait CliRunner {
    /// Parse a vector of command-line arguments into a CliCommand.
    fn parse_args(&self, args: Vec<String>) -> Result<CliCommand>;

    /// Execute a parsed command and return human-readable output.
    fn execute(&self, command: CliCommand) -> Result<String>;
}

// ---------------------------------------------------------------------------
// DefaultCliRunner
// ---------------------------------------------------------------------------

/// Default implementation of the CLI runner.
///
/// When constructed with `with_runtime`, uses a real session controller to
/// execute goals. When constructed with `stub`, returns placeholder data
/// (used in tests and when no packs are loaded).
pub struct DefaultCliRunner {
    runtime: Option<Arc<Mutex<Runtime>>>,
}

impl DefaultCliRunner {
    /// Create a runner backed by a real runtime. Goals will be parsed,
    /// sessions created, and skills executed through the session controller.
    pub fn with_runtime(runtime: Runtime) -> Self {
        Self {
            runtime: Some(Arc::new(Mutex::new(runtime))),
        }
    }

    /// Create a runner backed by a shared runtime. Used when the runtime
    /// is also shared with a TCP listener for incoming peer connections.
    pub fn with_runtime_arc(runtime: Arc<Mutex<Runtime>>) -> Self {
        Self {
            runtime: Some(runtime),
        }
    }

    /// Create a stub runner that returns placeholder data. Used in tests
    /// and when the binary starts without packs.
    pub fn stub() -> Self {
        Self { runtime: None }
    }

    /// Legacy constructor — delegates to stub() for backward compatibility.
    pub fn new() -> Self {
        Self::stub()
    }
}

impl Default for DefaultCliRunner {
    fn default() -> Self {
        Self::stub()
    }
}

impl CliRunner for DefaultCliRunner {
    /// Parse command-line arguments.
    ///
    /// Expected forms:
    ///   soma run "goal text"
    ///   soma inspect <session_id>
    ///   soma sessions
    ///   soma packs
    ///   soma skills
    ///   soma metrics
    ///   soma repl
    ///
    /// Also accepts --flags:
    ///   soma --goal "goal text"       (alias for run)
    ///   soma --session <session_id>   (alias for inspect)
    fn parse_args(&self, args: Vec<String>) -> Result<CliCommand> {
        // Skip the binary name if present (args[0] is typically the binary).
        let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        // Find the subcommand. Skip leading args that look like the binary path.
        let start = if !args.is_empty() && (args[0].contains('/') || args[0] == "soma") {
            1
        } else {
            0
        };

        let remaining = &args[start..];

        if remaining.is_empty() {
            return Err(SomaError::Interface(
                "no command provided. usage: soma <run|inspect|sessions|packs|skills|metrics|dump|repl>"
                    .to_string(),
            ));
        }

        match remaining[0] {
            "run" => {
                if remaining.len() < 2 {
                    return Err(SomaError::Interface(
                        "run requires a goal string. usage: soma run \"goal text\"".to_string(),
                    ));
                }
                let goal = remaining[1..].join(" ");
                Ok(CliCommand::Run { goal })
            }
            "inspect" => {
                if remaining.len() < 2 {
                    return Err(SomaError::Interface(
                        "inspect requires a session_id. usage: soma inspect <session_id>"
                            .to_string(),
                    ));
                }
                Ok(CliCommand::Inspect {
                    session_id: remaining[1].to_string(),
                })
            }
            "restore" => {
                if remaining.len() < 2 {
                    return Err(SomaError::Interface(
                        "restore requires a session_id. usage: soma restore <session_id>"
                            .to_string(),
                    ));
                }
                Ok(CliCommand::Restore {
                    session_id: remaining[1].to_string(),
                })
            }
            "sessions" | "list-sessions" => Ok(CliCommand::ListSessions),
            "packs" | "list-packs" => Ok(CliCommand::ListPacks),
            "skills" | "list-skills" => Ok(CliCommand::ListSkills),
            "metrics" => {
                // Check for --format <fmt> after "metrics"
                let mut fmt = None;
                let mut i = 1;
                while i < remaining.len() {
                    if remaining[i] == "--format"
                        && i + 1 < remaining.len() {
                            fmt = Some(remaining[i + 1].to_string());
                            i += 2;
                            continue;
                        }
                    i += 1;
                }
                Ok(CliCommand::Metrics { format: fmt })
            }
            "verify-port" => {
                if remaining.len() < 2 {
                    return Err(SomaError::Interface(
                        "verify-port requires a path. usage: soma verify-port <dylib_path>"
                            .to_string(),
                    ));
                }
                Ok(CliCommand::VerifyPort {
                    path: remaining[1].to_string(),
                })
            }
            "dump" => {
                let mut sections = Vec::new();
                let mut i = 1;
                while i < remaining.len() {
                    let arg = remaining[i];
                    match arg {
                        "--full" => sections.push("full".to_string()),
                        "--belief" => sections.push("belief".to_string()),
                        "--episodes" => sections.push("episodes".to_string()),
                        "--schemas" => sections.push("schemas".to_string()),
                        "--routines" => sections.push("routines".to_string()),
                        "--sessions" => sections.push("sessions".to_string()),
                        "--skills" => sections.push("skills".to_string()),
                        "--ports" => sections.push("ports".to_string()),
                        "--packs" => sections.push("packs".to_string()),
                        "--metrics" => sections.push("metrics".to_string()),
                        _ => {
                            return Err(SomaError::Interface(format!(
                                "unknown dump flag: '{}'. valid flags: --full, --belief, --episodes, --schemas, --routines, --sessions, --skills, --ports, --packs, --metrics",
                                arg
                            )));
                        }
                    }
                    i += 1;
                }
                if sections.is_empty() {
                    sections.push("full".to_string());
                }
                Ok(CliCommand::Dump { sections })
            }
            "repl" => Ok(CliCommand::Repl),

            // Flag-style arguments
            "--goal" => {
                if remaining.len() < 2 {
                    return Err(SomaError::Interface(
                        "--goal requires a value".to_string(),
                    ));
                }
                let goal = remaining[1..].join(" ");
                Ok(CliCommand::Run { goal })
            }
            "--session" => {
                if remaining.len() < 2 {
                    return Err(SomaError::Interface(
                        "--session requires a session_id".to_string(),
                    ));
                }
                Ok(CliCommand::Inspect {
                    session_id: remaining[1].to_string(),
                })
            }
            "--metrics" => Ok(CliCommand::Metrics { format: None }),
            "--repl" => Ok(CliCommand::Repl),

            other => Err(SomaError::Interface(format!(
                "unknown command: '{}'. usage: soma <run|inspect|restore|sessions|packs|skills|metrics|dump|verify-port|repl>",
                other
            ))),
        }
    }

    /// Execute a CLI command and return formatted output.
    fn execute(&self, command: CliCommand) -> Result<String> {
        match command {
            CliCommand::Run { goal } => self.execute_run(&goal),
            CliCommand::Inspect { session_id } => self.execute_inspect(&session_id),
            CliCommand::Restore { session_id } => self.execute_restore(&session_id),
            CliCommand::ListSessions => self.execute_list_sessions(),
            CliCommand::ListPacks => self.execute_list_packs(),
            CliCommand::ListSkills => self.execute_list_skills(),
            CliCommand::Metrics { format } => self.execute_metrics(format.as_deref()),
            CliCommand::VerifyPort { path } => self.execute_verify_port(&path),
            CliCommand::Dump { sections } => self.execute_dump(&sections),
            CliCommand::Repl => self.execute_repl(),
        }
    }
}

impl DefaultCliRunner {
    fn execute_run(&self, goal_text: &str) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => {
                // Stub mode: return placeholder
                let session_id = Uuid::new_v4();
                let goal_id = Uuid::new_v4();
                return Ok(format!(
                    "Goal submitted.\n  goal_id:    {}\n  session_id: {}\n  objective:  {}\n  status:     created",
                    goal_id, session_id, goal_text
                ));
            }
        };

        let mut rt = rt_arc.lock().unwrap();

        // Parse the natural language goal
        let input = GoalInput::NaturalLanguage {
            text: goal_text.to_string(),
            source: GoalSource {
                source_type: GoalSourceType::User,
                identity: None,
                session_id: None,
                peer_id: None,
            },
        };

        let mut goal = rt.goal_runtime.parse_goal(input)?;
        rt.goal_runtime.normalize_goal(&mut goal);
        rt.goal_runtime.validate_goal(&goal)?;

        // Create session
        let mut session = rt.session_controller.create_session(goal)?;
        let _ = goal_text;

        // Run the session loop until terminal. Errors from run_step bubble
        // back to the CLI caller (preserving existing CLI semantics).
        // Mid-run checkpointing is driven by `runtime.checkpoint_every_n_steps`.
        let checkpoint_store = std::sync::Arc::clone(&rt.checkpoint_store);
        let checkpoint_every_n = rt.checkpoint_every_n_steps;
        let final_step = crate::runtime::goal_executor::run_loop_with_checkpoint(
            &mut rt.session_controller,
            &mut session,
            Some(checkpoint_store.as_ref()),
            checkpoint_every_n,
        )?;
        let last_result = Some(final_step);

        let is_terminal = matches!(
            session.status,
            SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Aborted
        );
        if is_terminal {
            let ctx = crate::runtime::goal_executor::EpisodeContext {
                episode_store: &rt.episode_store,
                schema_store: &rt.schema_store,
                routine_store: &rt.routine_store,
                embedder: &rt.embedder,
                world_state: &rt.world_state,
                skill_stats: Some(&rt.skill_stats),
            };
            crate::runtime::goal_executor::finalize_episode(&session, &ctx);
        }

        // Format output from session trace
        format_session_output(&session, last_result.as_ref())
    }

    fn execute_inspect(&self, session_id: &str) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => {
                return Ok(format!(
                    "Session: {}\n  status: unknown (stub mode)",
                    session_id
                ));
            }
        };

        let rt = rt_arc.lock().unwrap();

        let uuid = Uuid::parse_str(session_id).map_err(|e| {
            SomaError::Interface(format!("invalid session_id '{}': {}", session_id, e))
        })?;

        let session = match rt.session_controller.get_session_by_id(&uuid) {
            Some(s) => s,
            None => {
                return Ok(format!("Session {} not found.", session_id));
            }
        };

        let mut lines = Vec::new();
        lines.push(format!("Session: {}", session.session_id));
        lines.push(format!("  status:            {:?}", session.status));
        lines.push(format!(
            "  objective:         {}",
            session.goal.objective.description
        ));
        lines.push("  budget_remaining:".to_string());
        lines.push(format!(
            "    risk:            {:.2}",
            session.budget_remaining.risk_remaining
        ));
        lines.push(format!(
            "    latency_ms:      {}",
            session.budget_remaining.latency_remaining_ms
        ));
        lines.push(format!(
            "    resource:        {:.2}",
            session.budget_remaining.resource_remaining
        ));
        lines.push(format!(
            "    steps:           {}",
            session.budget_remaining.steps_remaining
        ));
        lines.push("  working_memory:".to_string());
        lines.push(format!(
            "    active_bindings: {}",
            session.working_memory.active_bindings.len()
        ));
        lines.push(format!(
            "    unresolved_slots: {}",
            session.working_memory.unresolved_slots.len()
        ));
        lines.push(format!(
            "  trace_steps:       {}",
            session.trace.steps.len()
        ));

        for step in &session.trace.steps {
            lines.push(String::new());
            lines.push(format!("  step {}:", step.step_index));
            lines.push(format!("    skill:         {}", step.selected_skill));
            lines.push(format!("    observation:   {}", step.observation_id));
            lines.push(format!("    critic:        {}", step.critic_decision));
            lines.push(format!("    progress:      {:.2}", step.progress_delta));
            if !step.candidate_skills.is_empty() {
                lines.push(format!(
                    "    candidates:    {}",
                    step.candidate_skills.join(", ")
                ));
            }
            for pc in &step.port_calls {
                let status = if pc.success { "ok" } else { "failed" };
                lines.push(format!(
                    "    port_call:     {}:{} [{}]",
                    pc.port_id, pc.capability_id, status
                ));
            }
        }

        Ok(lines.join("\n"))
    }

    fn execute_restore(&self, session_id: &str) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => {
                return Err(SomaError::Interface(
                    "restore requires a running runtime (not available in stub mode)".to_string(),
                ));
            }
        };

        let uuid = Uuid::parse_str(session_id).map_err(|e| {
            SomaError::Interface(format!("invalid session_id '{}': {}", session_id, e))
        })?;

        // Load from disk checkpoint
        let data_dir = checkpoint_data_dir();
        let store = SessionCheckpointStore::new(&data_dir);
        let checkpoint_data = {
            let session = store.load(&uuid)?;
            serde_json::to_vec(&session).map_err(SomaError::from)?
        };

        let mut rt = rt_arc.lock().unwrap();

        // Restore into the session controller
        let restored_id = rt.session_controller.restore_session(&checkpoint_data)?;
        let session = rt.session_controller.get_session_by_id(&restored_id)
            .ok_or_else(|| SomaError::SessionNotFound(restored_id.to_string()))?;

        let status = format!("{:?}", session.status);
        let objective = session.goal.objective.description.clone();
        let steps = session.trace.steps.len();

        Ok(format!(
            "Session restored from checkpoint.\n  session_id: {}\n  status:     {}\n  objective:  {}\n  steps:      {}",
            restored_id, status, objective, steps
        ))
    }

    fn execute_list_sessions(&self) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => return Ok("Sessions: (none)".to_string()),
        };

        let rt = rt_arc.lock().unwrap();
        let sessions = rt.session_controller.list_sessions();

        if sessions.is_empty() {
            return Ok("Sessions: (none)".to_string());
        }

        let mut lines = Vec::new();
        lines.push(format!("Sessions: {}", sessions.len()));
        for (id, status) in &sessions {
            lines.push(format!("  {} [{}]", id, status));
        }
        Ok(lines.join("\n"))
    }

    fn execute_list_packs(&self) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => return Ok("Packs: (none loaded)".to_string()),
        };

        let rt = rt_arc.lock().unwrap();

        if rt.pack_specs.is_empty() {
            return Ok("Packs: (none loaded)".to_string());
        }

        let mut lines = Vec::new();
        lines.push(format!("Packs: {}", rt.pack_specs.len()));
        for pack in &rt.pack_specs {
            lines.push(format!("  {} v{} ({})", pack.id, pack.version, pack.namespace));
            lines.push(format!(
                "    skills: {}  ports: {}",
                pack.skills.len(),
                pack.ports.len()
            ));
            if let Some(ref desc) = pack.description {
                lines.push(format!("    {}", desc));
            }
        }
        Ok(lines.join("\n"))
    }

    fn execute_list_skills(&self) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => return Ok("Skills: (none available)".to_string()),
        };

        let rt = rt_arc.lock().unwrap();
        let skills = rt.skill_runtime.list_skills(None);

        if skills.is_empty() {
            return Ok("Skills: (none available)".to_string());
        }

        let mut lines = Vec::new();
        lines.push(format!("Skills: {}", skills.len()));
        for skill in &skills {
            lines.push(format!("  {} [{:?}]", skill.skill_id, skill.kind));
            lines.push(format!("    {}", skill.description));
            if !skill.capability_requirements.is_empty() {
                lines.push(format!(
                    "    requires: {}",
                    skill.capability_requirements.join(", ")
                ));
            }
        }
        Ok(lines.join("\n"))
    }

    fn execute_metrics(&self, format: Option<&str>) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => {
                return Ok("Metrics: (no runtime loaded)".to_string());
            }
        };

        let rt = rt_arc.lock().unwrap();

        // Sync memory store counts into the atomic counters so the snapshot
        // reflects current state. These are gauge-style values that can go
        // up or down as episodes/schemas/routines are added or invalidated.
        use std::sync::atomic::Ordering;
        let ep_count = rt.episode_store.lock().map(|s| s.count()).unwrap_or(0);
        rt.metrics.episodes_stored.store(ep_count as u64, Ordering::Relaxed);

        let snap = rt.metrics.snapshot();
        let self_model = rt.self_model();

        match format {
            Some("json") => {
                let mut json = snap.format_json();
                if let serde_json::Value::Object(ref mut map) = json {
                    map.insert("self_model".to_string(), self_model.to_json());
                }
                Ok(serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_string()))
            }
            Some("prometheus") => {
                let prom = snap.format_prometheus();
                Ok(format!(
                    "{prom}\n\
                     # HELP soma_rss_bytes Current resident set size in bytes\n\
                     # TYPE soma_rss_bytes gauge\n\
                     soma_rss_bytes {}\n\
                     # HELP soma_cpu_percent Estimated CPU usage percentage\n\
                     # TYPE soma_cpu_percent gauge\n\
                     soma_cpu_percent {:.2}\n\
                     # HELP soma_load_factor Normalized load factor (0.0-1.0)\n\
                     # TYPE soma_load_factor gauge\n\
                     soma_load_factor {:.4}\n\
                     # HELP soma_loaded_packs Number of packs loaded\n\
                     # TYPE soma_loaded_packs gauge\n\
                     soma_loaded_packs {}\n\
                     # HELP soma_registered_skills Number of skills registered\n\
                     # TYPE soma_registered_skills gauge\n\
                     soma_registered_skills {}\n\
                     # HELP soma_registered_ports Number of ports registered\n\
                     # TYPE soma_registered_ports gauge\n\
                     soma_registered_ports {}\n\
                     # HELP soma_peer_connections Number of peer connections\n\
                     # TYPE soma_peer_connections gauge\n\
                     soma_peer_connections {}",
                    self_model.rss_bytes,
                    self_model.cpu_percent,
                    self_model.load_factor(),
                    self_model.loaded_packs,
                    self_model.registered_skills,
                    self_model.registered_ports,
                    self_model.peer_connections,
                ))
            }
            _ => {
                let metrics_text = snap.format_text();
                let proprio_text = self_model.report();
                Ok(format!("{metrics_text}\n\n{proprio_text}"))
            }
        }
    }

    fn execute_dump(&self, sections: &[String]) -> Result<String> {
        let rt_arc = match &self.runtime {
            Some(rt) => rt,
            None => {
                return Ok(serde_json::to_string_pretty(
                    &serde_json::json!({"error": "no runtime loaded (stub mode)"}),
                ).unwrap_or_else(|_| "{}".to_string()));
            }
        };

        let rt = rt_arc.lock().unwrap();
        let dump = build_state_dump(&rt, sections);
        Ok(serde_json::to_string_pretty(&dump).unwrap_or_else(|_| "{}".to_string()))
    }

    fn execute_verify_port(&self, path: &str) -> Result<String> {
        #[cfg(feature = "dylib-ports")]
        {
            let dylib_path = std::path::Path::new(path);
            Ok(crate::runtime::port_verify::verify_port_report(dylib_path))
        }
        #[cfg(not(feature = "dylib-ports"))]
        {
            let _ = path;
            Err(crate::errors::SomaError::Port(
                "verify_port requires the `dylib-ports` feature".to_string(),
            ))
        }
    }

    fn execute_repl(&self) -> Result<String> {
        // In a real implementation this would enter a read-eval-print loop.
        // For now, return a message indicating REPL mode would start.
        Ok("REPL mode. Type 'help' for commands, 'quit' to exit.".to_string())
    }
}

// ---------------------------------------------------------------------------
// Checkpoint helpers
// ---------------------------------------------------------------------------

/// Resolve the data directory used for session checkpoints.
/// Uses `$SOMA_DATA_DIR` if set, otherwise defaults to `./data`.
fn checkpoint_data_dir() -> PathBuf {
    std::env::var("SOMA_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
}

/// Build an Episode from a completed ControlSession.
///
/// Maps the session trace into episode steps and determines the outcome
/// from the terminal session status. If an embedder is provided, computes
/// and attaches an embedding for the goal fingerprint.
pub fn build_episode_from_session(
    session: &crate::types::session::ControlSession,
    embedder: Option<&dyn crate::memory::embedder::GoalEmbedder>,
) -> Episode {
    let outcome = match session.status {
        SessionStatus::Completed => EpisodeOutcome::Success,
        SessionStatus::Failed => EpisodeOutcome::Failure,
        SessionStatus::Aborted => EpisodeOutcome::Aborted,
        _ => EpisodeOutcome::Failure,
    };
    let success = session.status == SessionStatus::Completed;

    let steps: Vec<EpisodeStep> = session
        .trace
        .steps
        .iter()
        .map(|ts| {
            let observation = crate::types::observation::Observation {
                observation_id: ts.observation_id,
                session_id: session.session_id,
                skill_id: Some(ts.selected_skill.clone()),
                port_calls: ts.port_calls.clone(),
                raw_result: serde_json::Value::Null,
                structured_result: if let Some(pc) = ts.port_calls.first() {
                    pc.structured_result.clone()
                } else {
                    serde_json::Value::Null
                },
                effect_patch: None,
                success: ts.port_calls.iter().all(|pc| pc.success),
                failure_class: None,
                failure_detail: None,
                latency_ms: ts.port_calls.iter().map(|pc| pc.latency_ms).sum(),
                resource_cost: crate::types::observation::default_cost_profile(),
                confidence: 1.0,
                timestamp: ts.timestamp,
            };

            EpisodeStep {
                step_index: ts.step_index,
                belief_summary: ts.belief_summary_before.clone(),
                candidates_considered: ts.candidate_skills.clone(),
                predicted_scores: ts
                    .predicted_scores
                    .iter()
                    .map(|s| s.score)
                    .collect(),
                selected_skill: ts.selected_skill.clone(),
                observation,
                belief_patch: ts.belief_patch.clone(),
                progress_delta: ts.progress_delta,
                critic_decision: ts.critic_decision.clone(),
                timestamp: ts.timestamp,
            }
        })
        .collect();

    let observations: Vec<crate::types::observation::Observation> = steps
        .iter()
        .map(|s| s.observation.clone())
        .collect();

    let total_cost: f64 = session
        .working_memory
        .budget_deltas
        .iter()
        .map(|d| d.resource_spent)
        .sum();

    let fingerprint = session.goal.objective.description.clone();
    let embedding = embedder.map(|e| e.embed(&fingerprint));

    let salience = {
        let outcome_weight = match outcome {
            EpisodeOutcome::Success => 1.0_f64,
            EpisodeOutcome::PartialSuccess => 0.5,
            EpisodeOutcome::Failure => 0.2,
            EpisodeOutcome::Aborted
            | EpisodeOutcome::Timeout
            | EpisodeOutcome::BudgetExhausted => 0.1,
        };
        // Efficiency: lower cost relative to starting budget = higher salience
        let efficiency = if total_cost > 0.0 {
            (1.0 - (total_cost / 100.0).min(1.0)).max(0.0)
        } else {
            0.5
        };
        (outcome_weight * 0.7 + efficiency * 0.3).clamp(0.0, 1.0)
    };

    Episode {
        episode_id: Uuid::new_v4(),
        goal_fingerprint: fingerprint,
        initial_belief_summary: serde_json::json!({}),
        steps,
        observations,
        outcome,
        total_cost,
        success,
        tags: vec![],
        embedding,
        created_at: Utc::now(),
        salience,
        world_state_context: serde_json::json!({}),
    }
}

/// Format the session output for CLI display.
fn format_session_output(
    session: &crate::types::session::ControlSession,
    _last_result: Option<&StepResult>,
) -> Result<String> {
    let mut lines = Vec::new();

    lines.push(format!("Session: {}", session.session_id));
    lines.push(format!("  status: {:?}", session.status));
    lines.push(format!(
        "  objective: {}",
        session.goal.objective.description
    ));
    lines.push(format!("  steps: {}", session.trace.steps.len()));

    // Show the observation result from the last trace step
    if let Some(last_step) = session.trace.steps.last() {
        let obs_id = last_step.observation_id;
        lines.push(format!("  last_observation: {}", obs_id));

        // Find the port call results from the last step
        for port_call in &last_step.port_calls {
            if port_call.success {
                // Format the structured result for display
                let result = &port_call.structured_result;

                if let Some(entries) = result.get("entries").and_then(|e| e.as_array()) {
                    lines.push(String::new());
                    for entry in entries {
                        if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                            let is_dir = entry
                                .get("is_dir")
                                .and_then(|d| d.as_bool())
                                .unwrap_or(false);
                            let marker = if is_dir { "/" } else { "" };
                            lines.push(format!("  {}{}", name, marker));
                        }
                    }
                } else if let Some(content) = result.get("content").and_then(|c| c.as_str())
                {
                    lines.push(format!("\n{}", content));
                } else {
                    lines.push(format!("  result: {}", result));
                }
            } else {
                let err = port_call
                    .structured_result
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown error");
                lines.push(format!("  error: {}", err));
            }
        }
    }

    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------------------
// State dump
// ---------------------------------------------------------------------------

/// Build a JSON dump of the requested runtime state sections.
///
/// Used by both the CLI `dump` command and the MCP `dump_state` tool to give
/// an LLM a complete snapshot of everything SOMA currently holds.
pub(crate) fn build_state_dump(rt: &Runtime, sections: &[String]) -> serde_json::Value {
    let full = sections.iter().any(|s| s == "full");
    let mut result = serde_json::Map::new();

    if full || sections.iter().any(|s| s == "belief") {
        let sessions = rt.session_controller.list_sessions();
        let mut beliefs = Vec::new();
        for (id, _status) in &sessions {
            if let Some(session) = rt.session_controller.get_session_by_id(id) {
                let belief = &session.belief;
                beliefs.push(serde_json::json!({
                    "session_id": id.to_string(),
                    "belief_id": belief.belief_id.to_string(),
                    "resources": serde_json::to_value(&belief.resources).unwrap_or_default(),
                    "facts": belief.facts.iter().map(|f| {
                        serde_json::json!({
                            "fact_id": f.fact_id,
                            "subject": f.subject,
                            "predicate": f.predicate,
                            "value": f.value,
                            "confidence": f.confidence,
                        })
                    }).collect::<Vec<_>>(),
                    "uncertainties": &belief.uncertainties,
                }));
            }
        }
        result.insert("belief".to_string(), serde_json::json!(beliefs));
    }

    if full || sections.iter().any(|s| s == "episodes") {
        let episodes = rt.episode_store.lock()
            .map(|es| {
                es.list(1000, 0)
                    .into_iter()
                    .map(|ep| serde_json::to_value(ep).unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        result.insert("episodes".to_string(), serde_json::json!(episodes));
    }

    if full || sections.iter().any(|s| s == "schemas") {
        let schemas = rt.schema_store.lock()
            .map(|ss| {
                ss.list_all()
                    .into_iter()
                    .map(|s| serde_json::to_value(s).unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        result.insert("schemas".to_string(), serde_json::json!(schemas));
    }

    if full || sections.iter().any(|s| s == "routines") {
        let routines = rt.routine_store.lock()
            .map(|rs| {
                rs.list_all()
                    .into_iter()
                    .map(|r| serde_json::to_value(r).unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        result.insert("routines".to_string(), serde_json::json!(routines));
    }

    if full || sections.iter().any(|s| s == "sessions") {
        let session_list = rt.session_controller.list_sessions();
        let mut session_details = Vec::new();
        for (id, status) in &session_list {
            if let Some(session) = rt.session_controller.get_session_by_id(id) {
                session_details.push(serde_json::json!({
                    "session_id": id.to_string(),
                    "status": status,
                    "objective": session.goal.objective.description,
                    "budget_remaining": {
                        "risk_remaining": session.budget_remaining.risk_remaining,
                        "latency_remaining_ms": session.budget_remaining.latency_remaining_ms,
                        "resource_remaining": session.budget_remaining.resource_remaining,
                        "steps_remaining": session.budget_remaining.steps_remaining,
                    },
                    "trace_steps": session.trace.steps.len(),
                    "working_memory": {
                        "active_bindings": session.working_memory.active_bindings.len(),
                        "unresolved_slots": &session.working_memory.unresolved_slots,
                        "current_subgoal": &session.working_memory.current_subgoal,
                        "candidate_shortlist": &session.working_memory.candidate_shortlist,
                    },
                    "trace": session.trace.steps.iter().map(|step| {
                        serde_json::json!({
                            "step_index": step.step_index,
                            "selected_skill": step.selected_skill,
                            "observation_id": step.observation_id.to_string(),
                            "critic_decision": step.critic_decision,
                            "progress_delta": step.progress_delta,
                            "timestamp": step.timestamp.to_rfc3339(),
                        })
                    }).collect::<Vec<_>>(),
                    "created_at": session.created_at.to_rfc3339(),
                    "updated_at": session.updated_at.to_rfc3339(),
                }));
            } else {
                session_details.push(serde_json::json!({
                    "session_id": id.to_string(),
                    "status": status,
                }));
            }
        }
        result.insert("sessions".to_string(), serde_json::json!(session_details));
    }

    if full || sections.iter().any(|s| s == "skills") {
        let skills = rt.skill_runtime.list_skills(None);
        let skill_json: Vec<serde_json::Value> = skills
            .iter()
            .map(|s| {
                serde_json::json!({
                    "skill_id": s.skill_id,
                    "name": s.name,
                    "namespace": s.namespace,
                    "pack": s.pack,
                    "kind": format!("{:?}", s.kind),
                    "description": s.description,
                    "inputs": s.inputs.schema,
                    "outputs": s.outputs.schema,
                    "risk_class": format!("{:?}", s.risk_class),
                    "determinism": format!("{:?}", s.determinism),
                    "capability_requirements": s.capability_requirements,
                })
            })
            .collect();
        result.insert("skills".to_string(), serde_json::json!(skill_json));
    }

    if full || sections.iter().any(|s| s == "ports") {
        let ports = rt.port_runtime.lock()
            .map(|pr| {
                pr.list_ports(None)
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "port_id": p.port_id,
                            "name": p.name,
                            "namespace": p.namespace,
                            "kind": format!("{:?}", p.kind),
                            "capabilities": p.capabilities.iter().map(|c| {
                                serde_json::json!({
                                    "capability_id": c.capability_id,
                                    "name": c.name,
                                    "purpose": &c.purpose,
                                })
                            }).collect::<Vec<_>>(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        result.insert("ports".to_string(), serde_json::json!(ports));
    }

    if full || sections.iter().any(|s| s == "packs") {
        let packs: Vec<serde_json::Value> = rt.pack_specs
            .iter()
            .map(|p| {
                serde_json::json!({
                    "pack_id": p.id,
                    "name": p.name,
                    "namespace": p.namespace,
                    "version": p.version.to_string(),
                    "description": p.description,
                    "skills_count": p.skills.len(),
                    "ports_count": p.ports.len(),
                    "schemas_count": p.schemas.len(),
                    "routines_count": p.routines.len(),
                    "policies_count": p.policies.len(),
                })
            })
            .collect();
        result.insert("packs".to_string(), serde_json::json!(packs));
    }

    if full || sections.iter().any(|s| s == "metrics") {
        use std::sync::atomic::Ordering;

        // Sync memory store counts into metrics before snapshotting.
        let ep_count = rt.episode_store.lock().map(|s| s.count()).unwrap_or(0);
        rt.metrics.episodes_stored.store(ep_count as u64, Ordering::Relaxed);

        let snap = rt.metrics.snapshot();
        let self_model = rt.self_model();

        let mut metrics_json = snap.format_json();
        if let serde_json::Value::Object(ref mut map) = metrics_json {
            map.insert("self_model".to_string(), self_model.to_json());
        }
        result.insert("metrics".to_string(), metrics_json);
    }

    serde_json::Value::Object(result)
}

// ---------------------------------------------------------------------------
// Learning loop
// ---------------------------------------------------------------------------

/// After storing an episode, check whether accumulated experience justifies
/// inducing a new schema and compiling a routine. This is the runtime's
/// mechanism for learning from repeated successful episodes.
pub fn attempt_learning(
    episode_store: &Arc<Mutex<dyn EpisodeStore + Send>>,
    schema_store: &Arc<Mutex<dyn SchemaStore + Send>>,
    routine_store: &Arc<Mutex<dyn RoutineStore + Send>>,
    goal_fingerprint: &str,
    embedder: &dyn crate::memory::embedder::GoalEmbedder,
) {
    // Compute query embedding and try embedding-based retrieval first.
    let query_embedding = embedder.embed(goal_fingerprint);
    let episodes = {
        let es = match episode_store.lock() {
            Ok(es) => es,
            Err(_) => return,
        };

        let by_embedding = es.retrieve_by_embedding(&query_embedding, 0.8, 50);
        if !by_embedding.is_empty() {
            by_embedding.into_iter().cloned().collect::<Vec<_>>()
        } else {
            // Fallback to prefix matching with exact fingerprint filter.
            es.retrieve_nearest(goal_fingerprint, 50)
                .into_iter()
                .filter(|ep| ep.goal_fingerprint == goal_fingerprint)
                .cloned()
                .collect::<Vec<_>>()
        }
    };

    if episodes.len() < 3 {
        return;
    }

    // Attempt schema induction using the embedding-aware path.
    let episode_refs: Vec<&Episode> = episodes.iter().collect();
    let induced_schemas = {
        let ss = match schema_store.lock() {
            Ok(ss) => ss,
            Err(_) => return,
        };
        ss.induce_from_episodes_with_embedder(&episode_refs, embedder)
    };

    if induced_schemas.is_empty() {
        return;
    }

    let mut contributing_episode_ids: Vec<uuid::Uuid> = Vec::new();

    for schema in &induced_schemas {
        tracing::info!(
            schema_id = %schema.schema_id,
            fingerprint = %goal_fingerprint,
            skills = ?schema.candidate_skill_ordering,
            confidence = schema.confidence,
            "schema induced from {} episodes",
            episodes.len(),
        );

        // Register the induced schema.
        {
            let mut ss = match schema_store.lock() {
                Ok(ss) => ss,
                Err(_) => return,
            };
            if let Err(e) = ss.register(schema.clone()) {
                tracing::warn!(error = %e, "failed to register induced schema");
                continue;
            }
        }

        // Attempt routine compilation from the schema.
        let compiled_routine = {
            let rs = match routine_store.lock() {
                Ok(rs) => rs,
                Err(_) => return,
            };
            rs.compile_from_schema(schema, &episode_refs)
        };

        if let Some(routine) = compiled_routine {
            tracing::info!(
                routine_id = %routine.routine_id,
                schema_id = %schema.schema_id,
                skill_path = ?routine.compiled_skill_path,
                confidence = routine.confidence,
                "routine compiled from schema",
            );

            let mut rs = match routine_store.lock() {
                Ok(rs) => rs,
                Err(_) => return,
            };
            if let Err(e) = rs.register(routine) {
                tracing::warn!(error = %e, "failed to register compiled routine");
            }
        }

        // Track episode IDs that contributed to this schema.
        contributing_episode_ids.extend(episodes.iter().map(|ep| ep.episode_id));
    }

    // If the episode store needs consolidation, evict episodes that contributed to schemas.
    if !contributing_episode_ids.is_empty() {
        let mut es = match episode_store.lock() {
            Ok(es) => es,
            Err(_) => return,
        };
        if es.needs_consolidation() {
            contributing_episode_ids.sort();
            contributing_episode_ids.dedup();
            let evicted = es.evict_consolidated(&contributing_episode_ids);
            if evicted > 0 {
                tracing::info!(
                    evicted,
                    "evicted consolidated episodes after schema induction"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn runner() -> DefaultCliRunner {
        DefaultCliRunner::new()
    }

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    // --- parse_args ---

    #[test]
    fn test_parse_run() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "run", "list files"])).unwrap();
        assert_eq!(cmd, CliCommand::Run { goal: "list files".to_string() });
    }

    #[test]
    fn test_parse_run_multi_word() {
        let r = runner();
        let cmd = r
            .parse_args(args(&["soma", "run", "list", "files", "in", "/tmp"]))
            .unwrap();
        assert_eq!(
            cmd,
            CliCommand::Run {
                goal: "list files in /tmp".to_string()
            }
        );
    }

    #[test]
    fn test_parse_run_missing_goal() {
        let r = runner();
        let result = r.parse_args(args(&["soma", "run"]));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_inspect() {
        let r = runner();
        let sid = Uuid::new_v4().to_string();
        let cmd = r
            .parse_args(args(&["soma", "inspect", &sid]))
            .unwrap();
        assert_eq!(
            cmd,
            CliCommand::Inspect {
                session_id: sid
            }
        );
    }

    #[test]
    fn test_parse_inspect_missing_id() {
        let r = runner();
        let result = r.parse_args(args(&["soma", "inspect"]));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_sessions() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "sessions"])).unwrap();
        assert_eq!(cmd, CliCommand::ListSessions);
    }

    #[test]
    fn test_parse_list_sessions_alias() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "list-sessions"])).unwrap();
        assert_eq!(cmd, CliCommand::ListSessions);
    }

    #[test]
    fn test_parse_packs() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "packs"])).unwrap();
        assert_eq!(cmd, CliCommand::ListPacks);
    }

    #[test]
    fn test_parse_list_packs_alias() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "list-packs"])).unwrap();
        assert_eq!(cmd, CliCommand::ListPacks);
    }

    #[test]
    fn test_parse_skills() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "skills"])).unwrap();
        assert_eq!(cmd, CliCommand::ListSkills);
    }

    #[test]
    fn test_parse_list_skills_alias() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "list-skills"])).unwrap();
        assert_eq!(cmd, CliCommand::ListSkills);
    }

    #[test]
    fn test_parse_metrics() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "metrics"])).unwrap();
        assert_eq!(cmd, CliCommand::Metrics { format: None });
    }

    #[test]
    fn test_parse_repl() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "repl"])).unwrap();
        assert_eq!(cmd, CliCommand::Repl);
    }

    #[test]
    fn test_parse_restore() {
        let r = runner();
        let sid = Uuid::new_v4().to_string();
        let cmd = r
            .parse_args(args(&["soma", "restore", &sid]))
            .unwrap();
        assert_eq!(
            cmd,
            CliCommand::Restore {
                session_id: sid
            }
        );
    }

    #[test]
    fn test_parse_restore_missing_id() {
        let r = runner();
        let result = r.parse_args(args(&["soma", "restore"]));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dump_default() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "dump"])).unwrap();
        assert_eq!(cmd, CliCommand::Dump { sections: vec!["full".to_string()] });
    }

    #[test]
    fn test_parse_dump_full() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "dump", "--full"])).unwrap();
        assert_eq!(cmd, CliCommand::Dump { sections: vec!["full".to_string()] });
    }

    #[test]
    fn test_parse_dump_specific_sections() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "dump", "--belief", "--episodes"])).unwrap();
        assert_eq!(cmd, CliCommand::Dump { sections: vec!["belief".to_string(), "episodes".to_string()] });
    }

    #[test]
    fn test_parse_dump_all_flags() {
        let r = runner();
        let cmd = r.parse_args(args(&[
            "soma", "dump", "--belief", "--episodes", "--schemas", "--routines",
            "--sessions", "--skills", "--ports", "--packs", "--metrics",
        ])).unwrap();
        if let CliCommand::Dump { sections } = cmd {
            assert_eq!(sections.len(), 9);
        } else {
            panic!("expected Dump command");
        }
    }

    #[test]
    fn test_parse_dump_invalid_flag() {
        let r = runner();
        let result = r.parse_args(args(&["soma", "dump", "--nonsense"]));
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_dump_stub_mode() {
        let r = runner();
        let output = r.execute(CliCommand::Dump { sections: vec!["full".to_string()] }).unwrap();
        assert!(output.contains("error"));
        assert!(output.contains("stub mode"));
    }

    #[test]
    fn test_execute_restore_stub_mode_fails() {
        let r = runner();
        let sid = Uuid::new_v4().to_string();
        let result = r.execute(CliCommand::Restore { session_id: sid });
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_flag_goal() {
        let r = runner();
        let cmd = r
            .parse_args(args(&["soma", "--goal", "send email"]))
            .unwrap();
        assert_eq!(
            cmd,
            CliCommand::Run {
                goal: "send email".to_string()
            }
        );
    }

    #[test]
    fn test_parse_flag_session() {
        let r = runner();
        let cmd = r
            .parse_args(args(&["soma", "--session", "abc-123"]))
            .unwrap();
        assert_eq!(
            cmd,
            CliCommand::Inspect {
                session_id: "abc-123".to_string()
            }
        );
    }

    #[test]
    fn test_parse_flag_metrics() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "--metrics"])).unwrap();
        assert_eq!(cmd, CliCommand::Metrics { format: None });
    }

    #[test]
    fn test_parse_flag_repl() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "--repl"])).unwrap();
        assert_eq!(cmd, CliCommand::Repl);
    }

    #[test]
    fn test_parse_unknown_command() {
        let r = runner();
        let result = r.parse_args(args(&["soma", "frobnicate"]));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_no_command() {
        let r = runner();
        let result = r.parse_args(args(&["soma"]));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_args() {
        let r = runner();
        let result = r.parse_args(args(&[]));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_without_binary_name() {
        let r = runner();
        let cmd = r.parse_args(args(&["metrics"])).unwrap();
        assert_eq!(cmd, CliCommand::Metrics { format: None });
    }

    #[test]
    fn test_parse_with_path_binary() {
        let r = runner();
        let cmd = r
            .parse_args(args(&["/usr/local/bin/soma", "metrics"]))
            .unwrap();
        assert_eq!(cmd, CliCommand::Metrics { format: None });
    }

    // --- execute ---

    #[test]
    fn test_execute_run() {
        let r = runner();
        let output = r
            .execute(CliCommand::Run {
                goal: "list files".to_string(),
            })
            .unwrap();
        assert!(output.contains("Goal submitted"));
        assert!(output.contains("list files"));
        assert!(output.contains("session_id"));
        assert!(output.contains("goal_id"));
        assert!(output.contains("created"));
    }

    #[test]
    fn test_execute_inspect() {
        let r = runner();
        let sid = Uuid::new_v4().to_string();
        let output = r
            .execute(CliCommand::Inspect {
                session_id: sid.clone(),
            })
            .unwrap();
        assert!(output.contains(&sid));
        assert!(output.contains("status"));
    }

    #[test]
    fn test_execute_list_sessions() {
        let r = runner();
        let output = r.execute(CliCommand::ListSessions).unwrap();
        assert!(output.contains("Sessions"));
    }

    #[test]
    fn test_execute_list_packs() {
        let r = runner();
        let output = r.execute(CliCommand::ListPacks).unwrap();
        assert!(output.contains("Packs"));
    }

    #[test]
    fn test_execute_list_skills() {
        let r = runner();
        let output = r.execute(CliCommand::ListSkills).unwrap();
        assert!(output.contains("Skills"));
    }

    #[test]
    fn test_execute_metrics_stub() {
        let r = runner();
        let output = r.execute(CliCommand::Metrics { format: None }).unwrap();
        assert!(output.contains("Metrics"));
    }

    #[test]
    fn test_execute_repl() {
        let r = runner();
        let output = r.execute(CliCommand::Repl).unwrap();
        assert!(output.contains("REPL"));
    }

    // --- Full round-trip ---

    #[test]
    fn test_roundtrip_run() {
        let r = runner();
        let cmd = r
            .parse_args(args(&["soma", "run", "show processes"]))
            .unwrap();
        let output = r.execute(cmd).unwrap();
        assert!(output.contains("show processes"));
    }

    #[test]
    fn test_roundtrip_inspect() {
        let r = runner();
        let sid = Uuid::new_v4().to_string();
        let cmd = r
            .parse_args(args(&["soma", "inspect", &sid]))
            .unwrap();
        let output = r.execute(cmd).unwrap();
        assert!(output.contains(&sid));
    }

    #[test]
    fn test_roundtrip_metrics() {
        let r = runner();
        let cmd = r.parse_args(args(&["soma", "metrics"])).unwrap();
        let output = r.execute(cmd).unwrap();
        assert!(output.contains("Metrics"));
    }

    // --- Serialization ---

    #[test]
    fn test_cli_command_serialization() {
        let cmd = CliCommand::Run {
            goal: "test".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let deserialized: CliCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, cmd);
    }

    #[test]
    fn test_cli_command_all_variants_serialize() {
        let commands = vec![
            CliCommand::Run { goal: "test".to_string() },
            CliCommand::Inspect { session_id: "abc".to_string() },
            CliCommand::ListSessions,
            CliCommand::ListPacks,
            CliCommand::ListSkills,
            CliCommand::Metrics { format: None },
            CliCommand::Dump { sections: vec!["full".to_string()] },
            CliCommand::Repl,
        ];
        for cmd in commands {
            let json = serde_json::to_string(&cmd).unwrap();
            let back: CliCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(back, cmd);
        }
    }

    #[test]
    fn test_default_impl() {
        let r = DefaultCliRunner::default();
        let cmd = r.parse_args(args(&["soma", "metrics"])).unwrap();
        assert_eq!(cmd, CliCommand::Metrics { format: None });
    }

    // --- Flag-style missing values ---

    #[test]
    fn test_flag_goal_missing_value() {
        let r = runner();
        let result = r.parse_args(args(&["soma", "--goal"]));
        assert!(result.is_err());
    }

    #[test]
    fn test_flag_session_missing_value() {
        let r = runner();
        let result = r.parse_args(args(&["soma", "--session"]));
        assert!(result.is_err());
    }

    // --- Learning loop tests ---

    #[test]
    fn test_attempt_learning_needs_three_episodes() {
        use crate::memory::episodes::DefaultEpisodeStore;
        use crate::memory::routines::DefaultRoutineStore;
        use crate::memory::schemas::DefaultSchemaStore;
        use crate::memory::embedder::HashEmbedder;
        use std::sync::{Arc, Mutex};

        let episode_store: Arc<Mutex<dyn EpisodeStore + Send>> = Arc::new(Mutex::new(DefaultEpisodeStore::new()));
        let schema_store: Arc<Mutex<dyn SchemaStore + Send>> = Arc::new(Mutex::new(DefaultSchemaStore::new()));
        let routine_store: Arc<Mutex<dyn RoutineStore + Send>> = Arc::new(Mutex::new(DefaultRoutineStore::new()));
        let embedder = HashEmbedder::new();

        // Store only 2 episodes — not enough for induction.
        for _ in 0..2 {
            let ep = make_test_episode("list files", &["readdir"], true);
            let mut es = episode_store.lock().unwrap();
            let _ = es.store(ep).unwrap();
        }

        attempt_learning(&episode_store, &schema_store, &routine_store, "list files", &embedder);

        // No schema should be induced with only 2 episodes.
        let ss = schema_store.lock().unwrap();
        assert!(ss.get("induced_list files").is_none());
    }

    #[test]
    fn test_attempt_learning_induces_schema_from_three_episodes() {
        use crate::memory::episodes::DefaultEpisodeStore;
        use crate::memory::routines::DefaultRoutineStore;
        use crate::memory::schemas::DefaultSchemaStore;
        use crate::memory::embedder::HashEmbedder;
        use std::sync::{Arc, Mutex};

        let episode_store: Arc<Mutex<dyn EpisodeStore + Send>> = Arc::new(Mutex::new(DefaultEpisodeStore::new()));
        let schema_store: Arc<Mutex<dyn SchemaStore + Send>> = Arc::new(Mutex::new(DefaultSchemaStore::new()));
        let routine_store: Arc<Mutex<dyn RoutineStore + Send>> = Arc::new(Mutex::new(DefaultRoutineStore::new()));
        let embedder = HashEmbedder::new();

        // Store 3 successful episodes with the same fingerprint and skill sequence.
        for _ in 0..3 {
            let ep = make_test_episode("list files", &["readdir"], true);
            let mut es = episode_store.lock().unwrap();
            let _ = es.store(ep).unwrap();
        }

        attempt_learning(&episode_store, &schema_store, &routine_store, "list files", &embedder);

        // Schema should be induced.
        let ss = schema_store.lock().unwrap();
        let schema = ss.get("induced_list files");
        assert!(schema.is_some(), "schema should be induced after 3 episodes");
        let schema = schema.unwrap();
        assert_eq!(schema.candidate_skill_ordering, vec!["readdir"]);
    }

    #[test]
    fn test_attempt_learning_routine_compiled_with_enough_evidence() {
        use crate::memory::episodes::DefaultEpisodeStore;
        use crate::memory::routines::DefaultRoutineStore;
        use crate::memory::schemas::DefaultSchemaStore;
        use crate::memory::embedder::HashEmbedder;
        use std::sync::{Arc, Mutex};

        let episode_store: Arc<Mutex<dyn EpisodeStore + Send>> = Arc::new(Mutex::new(DefaultEpisodeStore::new()));
        let schema_store: Arc<Mutex<dyn SchemaStore + Send>> = Arc::new(Mutex::new(DefaultSchemaStore::new()));
        let routine_store: Arc<Mutex<dyn RoutineStore + Send>> = Arc::new(Mutex::new(DefaultRoutineStore::new()));
        let embedder = HashEmbedder::new();

        // Store 8 episodes — schema confidence will be 8/10 = 0.8, above the
        // 0.7 threshold needed for routine compilation.
        for _ in 0..8 {
            let ep = make_test_episode("read config", &["open", "read", "close"], true);
            let mut es = episode_store.lock().unwrap();
            let _ = es.store(ep).unwrap();
        }

        attempt_learning(
            &episode_store,
            &schema_store,
            &routine_store,
            "read config",
            &embedder,
        );

        // Schema should be induced with high confidence.
        let ss = schema_store.lock().unwrap();
        let schema = ss.get("induced_read config");
        assert!(schema.is_some(), "schema should be induced after 8 episodes");
        assert!(schema.unwrap().confidence >= 0.7);

        // Routine should now be compiled.
        let rs = routine_store.lock().unwrap();
        let routine = rs.get("compiled_induced_read config");
        assert!(
            routine.is_some(),
            "routine should be compiled from high-confidence schema"
        );
        let routine = routine.unwrap();
        assert_eq!(
            routine.compiled_skill_path,
            vec!["open", "read", "close"]
        );

        // Verify the routine can be found by matching.
        let context = serde_json::json!({ "goal_fingerprint": "read config" });
        let matching = rs.find_matching(&context);
        assert_eq!(matching.len(), 1);
        assert_eq!(
            matching[0].compiled_skill_path,
            vec!["open", "read", "close"]
        );
    }

    fn make_test_episode(fingerprint: &str, skills: &[&str], success: bool) -> Episode {
        use crate::types::episode::EpisodeStep;
        use crate::types::observation::Observation;

        let steps: Vec<EpisodeStep> = skills
            .iter()
            .enumerate()
            .map(|(i, skill)| EpisodeStep {
                step_index: i as u32,
                belief_summary: serde_json::json!({}),
                candidates_considered: vec![skill.to_string()],
                predicted_scores: vec![0.9],
                selected_skill: skill.to_string(),
                observation: Observation {
                    observation_id: Uuid::new_v4(),
                    session_id: Uuid::new_v4(),
                    skill_id: Some(skill.to_string()),
                    port_calls: Vec::new(),
                    raw_result: serde_json::json!({}),
                    structured_result: serde_json::json!({}),
                    effect_patch: None,
                    success,
                    failure_class: None,
                    failure_detail: None,
                    latency_ms: 10,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 0.9,
                    timestamp: Utc::now(),
                },
                belief_patch: serde_json::json!({}),
                progress_delta: 0.5,
                critic_decision: "continue".into(),
                timestamp: Utc::now(),
            })
            .collect();

        Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: fingerprint.to_string(),
            initial_belief_summary: serde_json::json!({}),
            steps,
            observations: Vec::new(),
            outcome: if success {
                EpisodeOutcome::Success
            } else {
                EpisodeOutcome::Failure
            },
            total_cost: 0.1,
            success,
            tags: Vec::new(),
            embedding: None,
            created_at: Utc::now(),
            salience: 1.0,
            world_state_context: serde_json::json!({}),
        }
    }
}
