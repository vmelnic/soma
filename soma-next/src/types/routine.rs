use serde::{Deserialize, Serialize};

use super::common::{EffectDescriptor, Precondition};

/// Routine — a compiled habitual shortcut.
/// High-confidence, bounded, deterministic under declared conditions.
/// Bypasses deeper deliberation when match confidence and policy allow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub routine_id: String,
    pub namespace: String,
    pub origin: RoutineOrigin,
    pub match_conditions: Vec<Precondition>,
    pub compiled_skill_path: Vec<String>,
    pub guard_conditions: Vec<Precondition>,
    pub expected_cost: f64,
    pub expected_effect: Vec<EffectDescriptor>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineOrigin {
    PackAuthored,
    EpisodeInduced,
    SchemaCompiled,
    PeerTransferred,
}
