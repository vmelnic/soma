use serde::{Deserialize, Serialize};

use super::common::{EffectDescriptor, Precondition};

/// Maximum call depth for sub-routine nesting.
pub const MAX_CALL_DEPTH: usize = 16;

/// A single step in a compiled routine's execution path.
/// Supports sub-routine calls and branching on success/failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompiledStep {
    /// Execute a single skill by ID.
    Skill {
        skill_id: String,
        #[serde(default)]
        on_success: NextStep,
        #[serde(default)]
        on_failure: NextStep,
    },
    /// Call another routine as a sub-routine. Pushes the current
    /// plan state onto the stack, executes the referenced routine,
    /// and pops back on completion.
    SubRoutine {
        routine_id: String,
        #[serde(default)]
        on_success: NextStep,
        #[serde(default)]
        on_failure: NextStep,
    },
}

impl CompiledStep {
    pub fn on_success(&self) -> &NextStep {
        match self {
            CompiledStep::Skill { on_success, .. } => on_success,
            CompiledStep::SubRoutine { on_success, .. } => on_success,
        }
    }

    pub fn on_failure(&self) -> &NextStep {
        match self {
            CompiledStep::Skill { on_failure, .. } => on_failure,
            CompiledStep::SubRoutine { on_failure, .. } => on_failure,
        }
    }
}

/// What to do after a step succeeds or fails.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NextStep {
    /// Advance to the next sequential step.
    #[default]
    Continue,
    /// Jump to a specific step index within the current routine.
    Goto { step_index: usize },
    /// Call a sub-routine.
    CallRoutine { routine_id: String },
    /// Mark the current routine as successfully complete.
    Complete,
    /// Abandon the current plan and fall back to deliberation.
    Abandon,
}

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
    /// Rich step representation with branching and sub-routine support.
    /// When non-empty, takes precedence over `compiled_skill_path`.
    #[serde(default)]
    pub compiled_steps: Vec<CompiledStep>,
    pub guard_conditions: Vec<Precondition>,
    pub expected_cost: f64,
    pub expected_effect: Vec<EffectDescriptor>,
    pub confidence: f64,
    #[serde(default)]
    pub autonomous: bool,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub exclusive: bool,
    #[serde(default)]
    pub policy_scope: Option<String>,
    #[serde(default)]
    pub version: u32,
}

impl Routine {
    /// Returns the effective steps for this routine. If `compiled_steps`
    /// is populated, returns it directly. Otherwise, converts the legacy
    /// `compiled_skill_path` into simple `CompiledStep::Skill` entries.
    pub fn effective_steps(&self) -> Vec<CompiledStep> {
        if !self.compiled_steps.is_empty() {
            return self.compiled_steps.clone();
        }
        self.compiled_skill_path
            .iter()
            .map(|sid| CompiledStep::Skill {
                skill_id: sid.clone(),
                on_success: NextStep::Continue,
                on_failure: NextStep::Abandon,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineOrigin {
    PackAuthored,
    EpisodeInduced,
    SchemaCompiled,
    PeerTransferred,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_routine(
        skill_path: Vec<&str>,
        steps: Vec<CompiledStep>,
    ) -> Routine {
        Routine {
            routine_id: "test_routine".to_string(),
            namespace: "test".to_string(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions: Vec::new(),
            compiled_skill_path: skill_path.into_iter().map(String::from).collect(),
            compiled_steps: steps,
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence: 0.9,
            autonomous: false,
            priority: 0,
            exclusive: false,
            policy_scope: None,
            version: 0,
        }
    }

    #[test]
    fn test_effective_steps_uses_compiled_steps_when_present() {
        let steps = vec![
            CompiledStep::Skill {
                skill_id: "x".to_string(),
                on_success: NextStep::Continue,
                on_failure: NextStep::Abandon,
            },
            CompiledStep::SubRoutine {
                routine_id: "sub_r".to_string(),
                on_success: NextStep::Complete,
                on_failure: NextStep::Abandon,
            },
        ];
        let r = make_test_routine(vec!["a", "b"], steps);

        let effective = r.effective_steps();
        assert_eq!(effective.len(), 2);

        // Should return compiled_steps content, not compiled_skill_path.
        match &effective[0] {
            CompiledStep::Skill { skill_id, .. } => assert_eq!(skill_id, "x"),
            other => panic!("expected Skill, got {:?}", other),
        }
        match &effective[1] {
            CompiledStep::SubRoutine { routine_id, .. } => assert_eq!(routine_id, "sub_r"),
            other => panic!("expected SubRoutine, got {:?}", other),
        }
    }

    #[test]
    fn test_effective_steps_falls_back_to_compiled_skill_path() {
        let r = make_test_routine(vec!["a", "b"], vec![]);

        let effective = r.effective_steps();
        assert_eq!(effective.len(), 2);

        for (i, expected_id) in ["a", "b"].iter().enumerate() {
            match &effective[i] {
                CompiledStep::Skill {
                    skill_id,
                    on_success,
                    on_failure,
                } => {
                    assert_eq!(skill_id, expected_id);
                    assert!(matches!(on_success, NextStep::Continue));
                    assert!(matches!(on_failure, NextStep::Abandon));
                }
                other => panic!("expected Skill at index {}, got {:?}", i, other),
            }
        }
    }

    #[test]
    fn test_effective_steps_empty_when_both_empty() {
        let r = make_test_routine(vec![], vec![]);
        assert!(r.effective_steps().is_empty());
    }

    #[test]
    fn test_compiled_step_on_success_on_failure_accessors() {
        let skill_step = CompiledStep::Skill {
            skill_id: "s1".to_string(),
            on_success: NextStep::Goto { step_index: 3 },
            on_failure: NextStep::Abandon,
        };
        assert!(matches!(skill_step.on_success(), NextStep::Goto { step_index: 3 }));
        assert!(matches!(skill_step.on_failure(), NextStep::Abandon));

        let sub_step = CompiledStep::SubRoutine {
            routine_id: "sub".to_string(),
            on_success: NextStep::Complete,
            on_failure: NextStep::CallRoutine {
                routine_id: "fallback".to_string(),
            },
        };
        assert!(matches!(sub_step.on_success(), NextStep::Complete));
        match sub_step.on_failure() {
            NextStep::CallRoutine { routine_id } => assert_eq!(routine_id, "fallback"),
            other => panic!("expected CallRoutine, got {:?}", other),
        }
    }

    #[test]
    fn test_next_step_default_is_continue() {
        let default: NextStep = NextStep::default();
        assert!(matches!(default, NextStep::Continue));
    }

    #[test]
    fn test_compiled_step_serde_roundtrip() {
        // Skill variant
        let skill = CompiledStep::Skill {
            skill_id: "read_file".to_string(),
            on_success: NextStep::Goto { step_index: 2 },
            on_failure: NextStep::Abandon,
        };
        let json = serde_json::to_string(&skill).unwrap();
        let deserialized: CompiledStep = serde_json::from_str(&json).unwrap();
        match &deserialized {
            CompiledStep::Skill {
                skill_id,
                on_success,
                on_failure,
            } => {
                assert_eq!(skill_id, "read_file");
                assert!(matches!(on_success, NextStep::Goto { step_index: 2 }));
                assert!(matches!(on_failure, NextStep::Abandon));
            }
            other => panic!("expected Skill, got {:?}", other),
        }

        // SubRoutine variant
        let sub = CompiledStep::SubRoutine {
            routine_id: "cleanup".to_string(),
            on_success: NextStep::Complete,
            on_failure: NextStep::CallRoutine {
                routine_id: "error_handler".to_string(),
            },
        };
        let json = serde_json::to_string(&sub).unwrap();
        let deserialized: CompiledStep = serde_json::from_str(&json).unwrap();
        match &deserialized {
            CompiledStep::SubRoutine {
                routine_id,
                on_success,
                on_failure,
            } => {
                assert_eq!(routine_id, "cleanup");
                assert!(matches!(on_success, NextStep::Complete));
                match on_failure {
                    NextStep::CallRoutine { routine_id } => {
                        assert_eq!(routine_id, "error_handler")
                    }
                    other => panic!("expected CallRoutine, got {:?}", other),
                }
            }
            other => panic!("expected SubRoutine, got {:?}", other),
        }
    }

    #[test]
    fn test_next_step_serde_roundtrip() {
        let variants: Vec<NextStep> = vec![
            NextStep::Continue,
            NextStep::Goto { step_index: 5 },
            NextStep::CallRoutine {
                routine_id: "sub_r".to_string(),
            },
            NextStep::Complete,
            NextStep::Abandon,
        ];

        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let deserialized: NextStep = serde_json::from_str(&json).unwrap();

            // Verify round-trip by re-serializing and comparing JSON.
            let json2 = serde_json::to_string(&deserialized).unwrap();
            assert_eq!(json, json2, "round-trip failed for {:?}", variant);
        }
    }
}
