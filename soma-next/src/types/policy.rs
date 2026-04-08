use serde::{Deserialize, Serialize};

use super::common::{CapabilityScope, TrustLevel};

/// PolicySpec — policy contribution from a pack or host.
/// Covers all 7 required policy fields from pack-spec.md Section "PolicySpec Requirements":
/// allowed capabilities, denied capabilities, scope limits, trust classification,
/// confirmation requirements, destructive-action constraints, remote exposure limits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicySpec {
    pub policy_id: String,
    pub namespace: String,
    pub rules: Vec<PolicyRule>,
    /// Explicitly allowed capability patterns.
    #[serde(default)]
    pub allowed_capabilities: Vec<String>,
    /// Explicitly denied capability patterns.
    #[serde(default)]
    pub denied_capabilities: Vec<String>,
    /// Maximum scope for capabilities governed by this policy.
    pub scope_limits: Option<CapabilityScope>,
    /// Trust classification this policy requires.
    pub trust_classification: Option<TrustLevel>,
    /// Capabilities requiring explicit confirmation before execution.
    #[serde(default)]
    pub confirmation_requirements: Vec<String>,
    /// Constraints on destructive actions.
    #[serde(default)]
    pub destructive_action_constraints: Vec<String>,
    /// Limits on remote exposure for capabilities governed by this policy.
    #[serde(default)]
    pub remote_exposure_limits: Vec<String>,
}

/// A single policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub rule_id: String,
    pub rule_type: PolicyRuleType,
    pub target: PolicyTarget,
    pub effect: PolicyEffect,
    pub conditions: Vec<PolicyCondition>,
    pub priority: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyRuleType {
    Allow,
    Deny,
    RequireConfirmation,
    RequireEscalation,
    ConstrainBudget,
    DowngradeTrust,
    ForceDelegationRejection,
    RateLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyTarget {
    pub target_type: PolicyTargetType,
    pub identifiers: Vec<String>,
    pub scope: Option<CapabilityScope>,
    pub trust_level: Option<TrustLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyTargetType {
    Skill,
    Port,
    Resource,
    Pack,
    Peer,
    Session,
    Capability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffect {
    Allow,
    Deny,
    Constrain,
    RequireConfirmation,
    RequireEscalation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCondition {
    pub condition_type: String,
    pub expression: serde_json::Value,
}

/// Result of a policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub effect: PolicyEffect,
    pub matched_rules: Vec<String>,
    pub reason: String,
    pub constraints: Option<serde_json::Value>,
}
