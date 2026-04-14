use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::observation::Observation;

/// Episode — a full trace of a completed session.
/// Stored in episode memory for retrieval and learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub episode_id: Uuid,
    pub goal_fingerprint: String,
    pub initial_belief_summary: serde_json::Value,
    pub steps: Vec<EpisodeStep>,
    pub observations: Vec<Observation>,
    pub outcome: EpisodeOutcome,
    pub total_cost: f64,
    pub success: bool,
    pub tags: Vec<String>,
    pub embedding: Option<Vec<f32>>,
    pub created_at: DateTime<Utc>,
    /// Salience score for consolidation weighting. Higher = more valuable for
    /// pattern extraction. Computed from outcome quality and efficiency.
    #[serde(default = "default_salience")]
    pub salience: f64,
    /// World state facts present when this episode was created.
    /// Used by schema induction to derive world-state-triggered routines.
    #[serde(default)]
    pub world_state_context: serde_json::Value,
}

fn default_salience() -> f64 {
    1.0
}

/// A single step within an episode trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeStep {
    pub step_index: u32,
    pub belief_summary: serde_json::Value,
    pub candidates_considered: Vec<String>,
    pub predicted_scores: Vec<f64>,
    pub selected_skill: String,
    pub observation: Observation,
    pub belief_patch: serde_json::Value,
    pub progress_delta: f64,
    pub critic_decision: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeOutcome {
    Success,
    Failure,
    PartialSuccess,
    Aborted,
    Timeout,
    BudgetExhausted,
}
