use serde::{Deserialize, Serialize};

use super::common::{EffectDescriptor, Precondition};

/// Maximum call depth for sub-routine nesting.
pub const MAX_CALL_DEPTH: usize = 16;

/// A condition evaluated against the observation's structured_result.
/// If the expression matches, the associated next_step fires instead
/// of on_success. Conditions are checked in order; the first match wins.
/// Only evaluated when the step succeeds — failures always use on_failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataCondition {
    /// JSON expression to match against structured_result.
    /// Object keys are checked against the result: `true` means "key exists",
    /// any other value means "key equals this value exactly".
    pub expression: serde_json::Value,
    /// Human-readable description of what this condition checks.
    pub description: String,
    /// What to do if this condition matches.
    pub next_step: NextStep,
}

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
        #[serde(default)]
        conditions: Vec<DataCondition>,
        /// Per-step input overrides injected into active_bindings before
        /// this step's input binding pass. Values can be literals or
        /// `$field` references resolved from the current belief/bindings.
        #[serde(default)]
        input_overrides: std::collections::HashMap<String, serde_json::Value>,
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
        #[serde(default)]
        conditions: Vec<DataCondition>,
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

    pub fn conditions(&self) -> &[DataCondition] {
        match self {
            CompiledStep::Skill { conditions, .. } => conditions,
            CompiledStep::SubRoutine { conditions, .. } => conditions,
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
    /// If `max_iterations` is set, the jump is allowed at most that many
    /// times before auto-completing. This enables bounded loops.
    Goto {
        step_index: usize,
        #[serde(default)]
        max_iterations: Option<u32>,
    },
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
    #[serde(default)]
    pub model_evidence: f64,
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
                conditions: vec![],
                input_overrides: Default::default(),
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
            model_evidence: 0.0,
        }
    }

    #[test]
    fn test_effective_steps_uses_compiled_steps_when_present() {
        let steps = vec![
            CompiledStep::Skill {
                skill_id: "x".to_string(),
                on_success: NextStep::Continue,
                on_failure: NextStep::Abandon,
                conditions: vec![],
                input_overrides: Default::default(),
            },
            CompiledStep::SubRoutine {
                routine_id: "sub_r".to_string(),
                on_success: NextStep::Complete,
                on_failure: NextStep::Abandon,
                conditions: vec![],
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
                    ..
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
            on_success: NextStep::Goto { step_index: 3, max_iterations: None },
            on_failure: NextStep::Abandon,
            conditions: vec![],
            input_overrides: Default::default(),
        };
        assert!(matches!(skill_step.on_success(), NextStep::Goto { step_index: 3, .. }));
        assert!(matches!(skill_step.on_failure(), NextStep::Abandon));

        let sub_step = CompiledStep::SubRoutine {
            routine_id: "sub".to_string(),
            on_success: NextStep::Complete,
            on_failure: NextStep::CallRoutine {
                routine_id: "fallback".to_string(),
            },
            conditions: vec![],
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
            on_success: NextStep::Goto { step_index: 2, max_iterations: None },
            on_failure: NextStep::Abandon,
            conditions: vec![],
            input_overrides: Default::default(),
        };
        let json = serde_json::to_string(&skill).unwrap();
        let deserialized: CompiledStep = serde_json::from_str(&json).unwrap();
        match &deserialized {
            CompiledStep::Skill {
                skill_id,
                on_success,
                on_failure,
                ..
            } => {
                assert_eq!(skill_id, "read_file");
                assert!(matches!(on_success, NextStep::Goto { step_index: 2, .. }));
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
            conditions: vec![],
        };
        let json = serde_json::to_string(&sub).unwrap();
        let deserialized: CompiledStep = serde_json::from_str(&json).unwrap();
        match &deserialized {
            CompiledStep::SubRoutine {
                routine_id,
                on_success,
                on_failure,
                ..
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
            NextStep::Goto { step_index: 5, max_iterations: None },
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

    #[test]
    fn test_compiled_step_conditions_accessor() {
        let conds = vec![
            DataCondition {
                expression: serde_json::json!({"count": 0}),
                description: "no results".to_string(),
                next_step: NextStep::Goto { step_index: 2, max_iterations: None },
            },
        ];

        let skill_step = CompiledStep::Skill {
            skill_id: "query".to_string(),
            on_success: NextStep::Continue,
            on_failure: NextStep::Abandon,
            conditions: conds.clone(),
            input_overrides: Default::default(),
        };
        assert_eq!(skill_step.conditions().len(), 1);
        assert_eq!(skill_step.conditions()[0].description, "no results");

        let sub_step = CompiledStep::SubRoutine {
            routine_id: "sub".to_string(),
            on_success: NextStep::Continue,
            on_failure: NextStep::Abandon,
            conditions: conds,
        };
        assert_eq!(sub_step.conditions().len(), 1);
        assert_eq!(sub_step.conditions()[0].description, "no results");

        // Empty conditions
        let empty_step = CompiledStep::Skill {
            skill_id: "x".to_string(),
            on_success: NextStep::Continue,
            on_failure: NextStep::Abandon,
            conditions: vec![],
            input_overrides: Default::default(),
        };
        assert!(empty_step.conditions().is_empty());
    }

    #[test]
    fn test_data_condition_serde_roundtrip() {
        let cond = DataCondition {
            expression: serde_json::json!({"row_count": 0, "status": "empty"}),
            description: "no rows returned".to_string(),
            next_step: NextStep::Goto { step_index: 3, max_iterations: Some(5) },
        };

        let json = serde_json::to_string(&cond).unwrap();
        let deserialized: DataCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.description, "no rows returned");
        assert_eq!(deserialized.expression, serde_json::json!({"row_count": 0, "status": "empty"}));
        assert!(matches!(deserialized.next_step, NextStep::Goto { step_index: 3, max_iterations: Some(5) }));

        // Round-trip a step with conditions
        let step = CompiledStep::Skill {
            skill_id: "check".to_string(),
            on_success: NextStep::Continue,
            on_failure: NextStep::Abandon,
            conditions: vec![cond],
            input_overrides: Default::default(),
        };
        let step_json = serde_json::to_string(&step).unwrap();
        let step_back: CompiledStep = serde_json::from_str(&step_json).unwrap();
        assert_eq!(step_back.conditions().len(), 1);
        assert_eq!(step_back.conditions()[0].description, "no rows returned");
    }
}
