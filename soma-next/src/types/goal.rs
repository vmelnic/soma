use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// GoalSpec — the typed goal, not a program.
/// Output of the Goal Runtime. Input to the Session Runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSpec {
    pub goal_id: Uuid,
    pub source: GoalSource,
    pub objective: Objective,
    pub constraints: Vec<Constraint>,
    pub success_conditions: Vec<SuccessCondition>,
    pub risk_budget: f64,
    pub latency_budget_ms: u64,
    pub resource_budget: f64,
    pub deadline: Option<DateTime<Utc>>,
    pub permissions_scope: Vec<String>,
    pub priority: Priority,
    /// Per-goal override for the session step budget. When `None` the
    /// session controller uses its configured `default_max_steps`.
    #[serde(default)]
    pub max_steps: Option<u32>,
    /// Skill-selection exploration policy for this goal. Defaults to
    /// `Greedy` (always pick the highest predictor score). Brains submit
    /// goals with `EpsilonGreedy { epsilon }` to opt into exploration —
    /// useful when the brain wants the body to discover whether a
    /// lower-ranked skill performs better in this context.
    #[serde(default)]
    pub exploration: ExplorationStrategy,
}

/// Selection policy applied when scoring skill candidates. The body
/// honors this per-goal so the brain can flip between exploit and explore
/// without changing the runtime defaults.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExplorationStrategy {
    #[default]
    Greedy,
    EpsilonGreedy { epsilon: f64 },
    ActiveInference {
        #[serde(default = "default_one")]
        pragmatic_weight: f64,
        #[serde(default = "default_one")]
        epistemic_weight: f64,
    },
}

fn default_one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSource {
    pub source_type: GoalSourceType,
    pub identity: Option<String>,
    pub session_id: Option<Uuid>,
    pub peer_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalSourceType {
    User,
    Api,
    Mcp,
    Peer,
    Scheduler,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objective {
    pub description: String,
    pub structured: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    pub constraint_type: String,
    pub expression: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessCondition {
    pub description: String,
    pub expression: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}
