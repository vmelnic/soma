use serde::{Deserialize, Serialize};
use semver::Version;

use super::common::Precondition;

/// Schema — a reusable abstract structure for control.
/// Induced from repeated episodes or provided by packs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub schema_id: String,
    pub namespace: String,
    pub pack: String,
    pub name: String,
    pub version: Version,
    pub trigger_conditions: Vec<Precondition>,
    pub resource_requirements: Vec<String>,
    pub subgoal_structure: Vec<SubgoalNode>,
    pub candidate_skill_ordering: Vec<String>,
    pub stop_conditions: Vec<Precondition>,
    pub rollback_bias: RollbackBias,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgoalNode {
    pub subgoal_id: String,
    pub description: String,
    pub skill_candidates: Vec<String>,
    pub dependencies: Vec<String>,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackBias {
    Eager,
    Cautious,
    Minimal,
    None,
}
