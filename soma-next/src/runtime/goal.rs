use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::goal::*;

// --- Default budget constants ---

const DEFAULT_RISK_BUDGET: f64 = 0.5;
const DEFAULT_LATENCY_BUDGET_MS: u64 = 30_000;
const DEFAULT_RESOURCE_BUDGET: f64 = 100.0;

const MAX_RISK_BUDGET: f64 = 1.0;
const MAX_LATENCY_BUDGET_MS: u64 = 600_000; // 10 minutes
const MAX_RESOURCE_BUDGET: f64 = 10_000.0;

// --- GoalInput ---

/// The input to the Goal Runtime. Can be structured JSON, natural language, or a remote goal from a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GoalInput {
    /// Structured JSON that maps directly to GoalSpec fields.
    Structured(serde_json::Value),
    /// A natural-language request string that must be parsed.
    NaturalLanguage { text: String, source: GoalSource },
    /// A goal received from a peer node via the Synaptic Protocol.
    Remote { goal: GoalSpec, peer_id: String },
}

// --- GoalRuntime trait ---

/// The Goal Runtime: parses, validates, and normalizes goals.
/// Output is a typed GoalSpec, not a program.
pub trait GoalRuntime: Send + Sync {
    /// Parse a GoalInput into a GoalSpec.
    /// Structured input is deserialized directly.
    /// Natural language is converted into a minimal GoalSpec with the text as the objective.
    /// Remote goals are accepted as-is (the peer already structured them).
    fn parse_goal(&self, input: GoalInput) -> Result<GoalSpec>;

    /// Validate a GoalSpec for correctness:
    /// - Budgets must be positive
    /// - Deadline must be in the future (if set)
    /// - Permissions scope must be non-empty
    /// - At least one success condition must exist
    /// - Objective description must be non-empty
    fn validate_goal(&self, goal: &GoalSpec) -> Result<()>;

    /// Normalize a GoalSpec in place:
    /// - Apply default budgets if zero
    /// - Cap extreme values
    /// - Ensure goal_id is set (non-nil)
    fn normalize_goal(&self, goal: &mut GoalSpec);
}

// --- DefaultGoalRuntime ---

/// Default implementation of the Goal Runtime.
pub struct DefaultGoalRuntime;

impl DefaultGoalRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultGoalRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl GoalRuntime for DefaultGoalRuntime {
    fn parse_goal(&self, input: GoalInput) -> Result<GoalSpec> {
        match input {
            GoalInput::Structured(value) => {
                let goal: GoalSpec = serde_json::from_value(value)?;
                Ok(goal)
            }
            GoalInput::NaturalLanguage { text, source } => {
                if text.trim().is_empty() {
                    return Err(SomaError::Goal(
                        "natural language input is empty".to_string(),
                    ));
                }
                let goal = GoalSpec {
                    goal_id: Uuid::new_v4(),
                    source,
                    objective: Objective {
                        description: text,
                        structured: None,
                    },
                    constraints: Vec::new(),
                    success_conditions: vec![SuccessCondition {
                        description: "objective completed".to_string(),
                        expression: serde_json::json!({"status": "completed"}),
                    }],
                    risk_budget: DEFAULT_RISK_BUDGET,
                    latency_budget_ms: DEFAULT_LATENCY_BUDGET_MS,
                    resource_budget: DEFAULT_RESOURCE_BUDGET,
                    deadline: None,
                    permissions_scope: vec!["default".to_string()],
                    priority: Priority::Normal,
                    max_steps: None,
                    exploration: crate::types::goal::ExplorationStrategy::Greedy,
                };
                Ok(goal)
            }
            GoalInput::Remote { goal, peer_id: _ } => Ok(goal),
        }
    }

    fn validate_goal(&self, goal: &GoalSpec) -> Result<()> {
        // Objective must be non-empty
        if goal.objective.description.trim().is_empty() {
            return Err(SomaError::GoalValidation(
                "objective description must not be empty".to_string(),
            ));
        }

        // Budgets must be positive
        if goal.risk_budget <= 0.0 {
            return Err(SomaError::GoalValidation(
                "risk_budget must be positive".to_string(),
            ));
        }
        if goal.latency_budget_ms == 0 {
            return Err(SomaError::GoalValidation(
                "latency_budget_ms must be positive".to_string(),
            ));
        }
        if goal.resource_budget <= 0.0 {
            return Err(SomaError::GoalValidation(
                "resource_budget must be positive".to_string(),
            ));
        }

        // Deadline must be in the future (if set)
        if let Some(deadline) = goal.deadline
            && deadline <= Utc::now()
        {
            return Err(SomaError::GoalValidation(
                "deadline must be in the future".to_string(),
            ));
        }

        // Permissions scope must be non-empty
        if goal.permissions_scope.is_empty() {
            return Err(SomaError::GoalValidation(
                "permissions_scope must not be empty".to_string(),
            ));
        }

        // At least one success condition must exist
        if goal.success_conditions.is_empty() {
            return Err(SomaError::GoalValidation(
                "at least one success condition is required".to_string(),
            ));
        }

        Ok(())
    }

    fn normalize_goal(&self, goal: &mut GoalSpec) {
        // Ensure goal_id is set (non-nil)
        if goal.goal_id.is_nil() {
            goal.goal_id = Uuid::new_v4();
        }

        // Apply default budgets if zero
        if goal.risk_budget == 0.0 {
            goal.risk_budget = DEFAULT_RISK_BUDGET;
        }
        if goal.latency_budget_ms == 0 {
            goal.latency_budget_ms = DEFAULT_LATENCY_BUDGET_MS;
        }
        if goal.resource_budget == 0.0 {
            goal.resource_budget = DEFAULT_RESOURCE_BUDGET;
        }

        // Cap extreme values
        if goal.risk_budget > MAX_RISK_BUDGET {
            goal.risk_budget = MAX_RISK_BUDGET;
        }
        if goal.latency_budget_ms > MAX_LATENCY_BUDGET_MS {
            goal.latency_budget_ms = MAX_LATENCY_BUDGET_MS;
        }
        if goal.resource_budget > MAX_RESOURCE_BUDGET {
            goal.resource_budget = MAX_RESOURCE_BUDGET;
        }

        // Ensure negative budgets become defaults (defensive)
        if goal.risk_budget < 0.0 {
            goal.risk_budget = DEFAULT_RISK_BUDGET;
        }
        if goal.resource_budget < 0.0 {
            goal.resource_budget = DEFAULT_RESOURCE_BUDGET;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn make_source() -> GoalSource {
        GoalSource {
            source_type: GoalSourceType::User,
            identity: Some("test-user".to_string()),
            session_id: None,
            peer_id: None,
        }
    }

    fn make_valid_goal() -> GoalSpec {
        GoalSpec {
            goal_id: Uuid::new_v4(),
            source: make_source(),
            objective: Objective {
                description: "list files in /tmp".to_string(),
                structured: None,
            },
            constraints: vec![],
            success_conditions: vec![SuccessCondition {
                description: "files listed".to_string(),
                expression: serde_json::json!({"status": "done"}),
            }],
            risk_budget: 0.3,
            latency_budget_ms: 5000,
            resource_budget: 50.0,
            deadline: None,
            permissions_scope: vec!["filesystem".to_string()],
            priority: Priority::Normal,
            max_steps: None,
            exploration: crate::types::goal::ExplorationStrategy::Greedy,
        }
    }

    #[test]
    fn parse_structured_goal() {
        let rt = DefaultGoalRuntime::new();
        let goal = make_valid_goal();
        let json = serde_json::to_value(&goal).unwrap();
        let input = GoalInput::Structured(json);
        let parsed = rt.parse_goal(input).unwrap();
        assert_eq!(parsed.goal_id, goal.goal_id);
        assert_eq!(parsed.objective.description, "list files in /tmp");
    }

    #[test]
    fn parse_natural_language_goal() {
        let rt = DefaultGoalRuntime::new();
        let input = GoalInput::NaturalLanguage {
            text: "show me all running processes".to_string(),
            source: make_source(),
        };
        let parsed = rt.parse_goal(input).unwrap();
        assert_eq!(
            parsed.objective.description,
            "show me all running processes"
        );
        assert!(!parsed.goal_id.is_nil());
        assert_eq!(parsed.success_conditions.len(), 1);
        assert_eq!(parsed.permissions_scope, vec!["default"]);
        assert_eq!(parsed.priority, Priority::Normal);
    }

    #[test]
    fn parse_natural_language_empty_fails() {
        let rt = DefaultGoalRuntime::new();
        let input = GoalInput::NaturalLanguage {
            text: "   ".to_string(),
            source: make_source(),
        };
        let result = rt.parse_goal(input);
        assert!(result.is_err());
    }

    #[test]
    fn parse_remote_goal() {
        let rt = DefaultGoalRuntime::new();
        let goal = make_valid_goal();
        let expected_id = goal.goal_id;
        let input = GoalInput::Remote {
            goal,
            peer_id: "peer-abc".to_string(),
        };
        let parsed = rt.parse_goal(input).unwrap();
        assert_eq!(parsed.goal_id, expected_id);
    }

    #[test]
    fn validate_valid_goal() {
        let rt = DefaultGoalRuntime::new();
        let goal = make_valid_goal();
        assert!(rt.validate_goal(&goal).is_ok());
    }

    #[test]
    fn validate_empty_objective_fails() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.objective.description = "".to_string();
        assert!(rt.validate_goal(&goal).is_err());
    }

    #[test]
    fn validate_zero_risk_budget_fails() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.risk_budget = 0.0;
        assert!(rt.validate_goal(&goal).is_err());
    }

    #[test]
    fn validate_zero_latency_budget_fails() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.latency_budget_ms = 0;
        assert!(rt.validate_goal(&goal).is_err());
    }

    #[test]
    fn validate_zero_resource_budget_fails() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.resource_budget = 0.0;
        assert!(rt.validate_goal(&goal).is_err());
    }

    #[test]
    fn validate_past_deadline_fails() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.deadline = Some(DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc));
        assert!(rt.validate_goal(&goal).is_err());
    }

    #[test]
    fn validate_empty_permissions_fails() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.permissions_scope = vec![];
        assert!(rt.validate_goal(&goal).is_err());
    }

    #[test]
    fn validate_no_success_conditions_fails() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.success_conditions = vec![];
        assert!(rt.validate_goal(&goal).is_err());
    }

    #[test]
    fn normalize_applies_defaults_for_zero_budgets() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.risk_budget = 0.0;
        goal.latency_budget_ms = 0;
        goal.resource_budget = 0.0;
        rt.normalize_goal(&mut goal);
        assert_eq!(goal.risk_budget, DEFAULT_RISK_BUDGET);
        assert_eq!(goal.latency_budget_ms, DEFAULT_LATENCY_BUDGET_MS);
        assert_eq!(goal.resource_budget, DEFAULT_RESOURCE_BUDGET);
    }

    #[test]
    fn normalize_caps_extreme_values() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.risk_budget = 999.0;
        goal.latency_budget_ms = 999_999_999;
        goal.resource_budget = 999_999.0;
        rt.normalize_goal(&mut goal);
        assert_eq!(goal.risk_budget, MAX_RISK_BUDGET);
        assert_eq!(goal.latency_budget_ms, MAX_LATENCY_BUDGET_MS);
        assert_eq!(goal.resource_budget, MAX_RESOURCE_BUDGET);
    }

    #[test]
    fn normalize_assigns_goal_id_if_nil() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.goal_id = Uuid::nil();
        rt.normalize_goal(&mut goal);
        assert!(!goal.goal_id.is_nil());
    }

    #[test]
    fn normalize_preserves_existing_goal_id() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        let original_id = goal.goal_id;
        rt.normalize_goal(&mut goal);
        assert_eq!(goal.goal_id, original_id);
    }

    #[test]
    fn normalize_fixes_negative_budgets() {
        let rt = DefaultGoalRuntime::new();
        let mut goal = make_valid_goal();
        goal.risk_budget = -1.0;
        goal.resource_budget = -50.0;
        rt.normalize_goal(&mut goal);
        assert_eq!(goal.risk_budget, DEFAULT_RISK_BUDGET);
        assert_eq!(goal.resource_budget, DEFAULT_RESOURCE_BUDGET);
    }

    #[test]
    fn parse_structured_invalid_json_fails() {
        let rt = DefaultGoalRuntime::new();
        let input = GoalInput::Structured(serde_json::json!({"not": "a goal"}));
        assert!(rt.parse_goal(input).is_err());
    }

    #[test]
    fn full_lifecycle_parse_normalize_validate() {
        let rt = DefaultGoalRuntime::new();
        let input = GoalInput::NaturalLanguage {
            text: "send an email to alice".to_string(),
            source: make_source(),
        };
        let mut goal = rt.parse_goal(input).unwrap();
        rt.normalize_goal(&mut goal);
        assert!(rt.validate_goal(&goal).is_ok());
    }
}
