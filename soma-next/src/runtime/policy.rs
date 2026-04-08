use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::common::{SideEffectClass, TrustLevel};
use crate::types::policy::{
    PolicyCondition, PolicyDecision, PolicyEffect, PolicyRule, PolicyRuleType, PolicySpec,
    PolicyTarget, PolicyTargetType,
};

// ---------------------------------------------------------------------------
// PolicyRequest & PolicyContext — the inputs to every evaluation
// ---------------------------------------------------------------------------

/// A request to evaluate a policy decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub action: String,
    pub target_type: PolicyTargetType,
    pub target_id: String,
    pub context: PolicyContext,
}

/// Contextual information carried into every policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyContext {
    pub session_id: Option<Uuid>,
    pub trust_level: TrustLevel,
    pub namespace: String,
    pub budget_remaining: Option<f64>,
    /// Side-effect class of the action being evaluated, if known.
    /// Used for default-deny on destructive/irreversible actions.
    #[serde(default)]
    pub side_effect_class: Option<SideEffectClass>,
}

// ---------------------------------------------------------------------------
// Rate-limit tracking
// ---------------------------------------------------------------------------

/// Key for the rate-limiter: (target_id, rule_id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RateLimitKey {
    target_id: String,
    rule_id: String,
}

/// A ring of timestamps for sliding-window rate limiting.
struct RateLimitBucket {
    window_ms: u64,
    max_count: u64,
    timestamps: Vec<Instant>,
}

impl RateLimitBucket {
    fn new(window_ms: u64, max_count: u64) -> Self {
        Self {
            window_ms,
            max_count,
            timestamps: Vec::new(),
        }
    }

    /// Prune old entries outside the current window, then check whether
    /// adding one more invocation would exceed the limit.
    fn check_and_record(&mut self) -> bool {
        let now = Instant::now();
        let window = std::time::Duration::from_millis(self.window_ms);
        self.timestamps.retain(|t| now.duration_since(*t) < window);
        if self.timestamps.len() as u64 >= self.max_count {
            false // exceeded
        } else {
            self.timestamps.push(now);
            true // allowed
        }
    }

    /// Check without recording (peek).
    pub fn would_exceed(&mut self) -> bool {
        let now = Instant::now();
        let window = std::time::Duration::from_millis(self.window_ms);
        self.timestamps.retain(|t| now.duration_since(*t) < window);
        self.timestamps.len() as u64 >= self.max_count
    }
}

// ---------------------------------------------------------------------------
// PolicyRuntime trait
// ---------------------------------------------------------------------------

/// The Policy Runtime trait: register policies and evaluate requests at every
/// hook point in the execution lifecycle.
pub trait PolicyRuntime: Send + Sync {
    /// Register a policy spec (from a pack manifest or host config).
    /// Host policies and pack policies are stored separately;
    /// pack policies MUST NOT widen privilege beyond host policy.
    fn register_policy(&self, spec: PolicySpec) -> Result<()>;

    /// General evaluation: find matching rules for a request and return
    /// the aggregate decision. Most restrictive rule wins.
    /// Host rules always take precedence over pack rules.
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyDecision>;

    /// Convenience: check whether a skill invocation is allowed.
    fn check_skill(&self, skill_id: &str, context: &PolicyContext) -> Result<PolicyDecision>;

    /// Convenience: check whether a port capability invocation is allowed.
    fn check_port(
        &self,
        port_id: &str,
        capability_id: &str,
        context: &PolicyContext,
    ) -> Result<PolicyDecision>;

    /// Convenience: check whether delegation to a peer is allowed.
    fn check_delegation(
        &self,
        peer_id: &str,
        action: &str,
        context: &PolicyContext,
    ) -> Result<PolicyDecision>;

    /// Convenience: check whether a capability may be exposed remotely.
    fn check_remote_exposure(
        &self,
        capability: &str,
        context: &PolicyContext,
    ) -> Result<PolicyDecision>;
}

// ---------------------------------------------------------------------------
// PolicyOrigin — tracks whether a registered spec is host or pack
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyOrigin {
    Host,
    Pack,
}

#[derive(Debug, Clone)]
struct RegisteredRule {
    rule: PolicyRule,
    origin: PolicyOrigin,
    namespace: String,
}

// ---------------------------------------------------------------------------
// DefaultPolicyRuntime
// ---------------------------------------------------------------------------

/// Default implementation that stores host and pack rules separately,
/// merges them during evaluation (host wins), and applies rate limiting.
pub struct DefaultPolicyRuntime {
    /// All registered rules (host and pack).
    rules: RwLock<Vec<RegisteredRule>>,
    /// Rate-limit buckets keyed by (target_id, rule_id).
    rate_limits: RwLock<HashMap<RateLimitKey, RateLimitBucket>>,
}

impl DefaultPolicyRuntime {
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(Vec::new()),
            rate_limits: RwLock::new(HashMap::new()),
        }
    }

    /// Validate that a pack policy does not widen privilege beyond what the host
    /// allows. For every Allow rule in the pack spec, check if any existing host
    /// rule explicitly Denies (or ForceDelegationRejection) the same target. If
    /// any pack Allow conflicts with a host Deny, the entire spec is rejected.
    ///
    /// This checks ALL identifiers in the pack rule (not just the first), and
    /// accounts for wildcard patterns in both the host deny and the pack allow.
    fn validate_pack_does_not_widen(
        &self,
        spec: &PolicySpec,
        existing_rules: &[RegisteredRule],
    ) -> Result<()> {
        for rule in &spec.rules {
            if rule.rule_type != PolicyRuleType::Allow {
                continue;
            }

            // Collect the identifiers the pack claims to allow. If empty, the
            // pack rule is a wildcard for this target type (matches everything).
            let pack_ids: Vec<&str> = if rule.target.identifiers.is_empty() {
                vec!["*"]
            } else {
                rule.target.identifiers.iter().map(|s| s.as_str()).collect()
            };

            for pack_id in &pack_ids {
                let host_denies = existing_rules.iter().any(|reg| {
                    if reg.origin != PolicyOrigin::Host {
                        return false;
                    }
                    let is_deny = reg.rule.rule_type == PolicyRuleType::Deny
                        || reg.rule.rule_type == PolicyRuleType::ForceDelegationRejection;
                    if !is_deny {
                        return false;
                    }
                    if reg.rule.target.target_type != rule.target.target_type {
                        return false;
                    }
                    // Check if the host deny covers any identifier the pack
                    // tries to allow. This handles wildcards in both directions.
                    targets_overlap(&reg.rule.target, pack_id)
                });

                if host_denies {
                    return Err(SomaError::PolicyDenied {
                        action: format!("register_policy({})", spec.policy_id),
                        reason: format!(
                            "pack rule {} attempts to allow target '{}' that host denies",
                            rule.rule_id, pack_id,
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Check whether a destructive or irreversible operation is explicitly
    /// authorized. Unlike the general `evaluate` path which defaults-allow for
    /// unknown actions, destructive operations require an explicit Allow rule.
    /// If no explicit rule exists, the decision is Deny.
    pub fn check_destructive_operation(
        &self,
        action: &str,
        context: &PolicyContext,
    ) -> PolicyDecision {
        // Build a context that carries the destructive side-effect class so the
        // default_decision path knows to deny.
        let mut ctx = context.clone();
        if ctx.side_effect_class.is_none() {
            ctx.side_effect_class = Some(SideEffectClass::Destructive);
        }

        match self.evaluate_inner(PolicyTargetType::Skill, action, &ctx) {
            Ok(decision) => {
                // If no rules matched at all, evaluate_inner already returns
                // default-deny for destructive actions. But if only pack rules
                // matched with Allow, that's still not an explicit host grant.
                // We need an explicit Allow from at least one host rule.
                if decision.effect == PolicyEffect::Allow {
                    // Verify there is at least one explicit host Allow covering this action.
                    let has_explicit_host_allow = self.has_explicit_host_allow(
                        PolicyTargetType::Skill,
                        action,
                        &ctx,
                    );
                    if !has_explicit_host_allow {
                        return PolicyDecision {
                            allowed: false,
                            effect: PolicyEffect::Deny,
                            matched_rules: decision.matched_rules,
                            reason: "destructive operation requires explicit host Allow rule"
                                .to_string(),
                            constraints: None,
                        };
                    }
                }
                decision
            }
            Err(_) => PolicyDecision {
                allowed: false,
                effect: PolicyEffect::Deny,
                matched_rules: vec![],
                reason: "policy evaluation error on destructive operation".to_string(),
                constraints: None,
            },
        }
    }

    /// Returns true if at least one host-origin Allow rule matches the given
    /// target and context (conditions met).
    fn has_explicit_host_allow(
        &self,
        target_type: PolicyTargetType,
        target_id: &str,
        context: &PolicyContext,
    ) -> bool {
        let rules = match self.rules.read() {
            Ok(r) => r,
            Err(_) => return false,
        };
        rules.iter().any(|reg| {
            reg.origin == PolicyOrigin::Host
                && reg.rule.rule_type == PolicyRuleType::Allow
                && target_matches(&reg.rule.target, target_type, target_id)
                && conditions_met(&reg.rule.conditions, context)
        })
    }

    /// Returns a snapshot of which rate-limit counters are at or above their
    /// thresholds. The key is "{target_id}:{rule_id}" and the value is true
    /// when the next invocation would exceed the limit (peek, no recording).
    pub fn policy_rate_status(&self) -> HashMap<String, bool> {
        let mut buckets = match self.rate_limits.write() {
            Ok(b) => b,
            Err(_) => return HashMap::new(),
        };
        buckets
            .iter_mut()
            .map(|(key, bucket)| {
                let label = format!("{}:{}", key.target_id, key.rule_id);
                (label, bucket.would_exceed())
            })
            .collect()
    }

    /// Returns which rate-limit counters would exceed their thresholds.
    /// Each entry is ("{target_id}:{rule_id}", would_exceed).
    pub fn rate_status(&self) -> Vec<(String, bool)> {
        let mut buckets = match self.rate_limits.write() {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        buckets
            .iter_mut()
            .map(|(key, bucket)| {
                let label = format!("{}:{}", key.target_id, key.rule_id);
                (label, bucket.would_exceed())
            })
            .collect()
    }
}

impl Default for DefaultPolicyRuntime {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Matching helpers
// ---------------------------------------------------------------------------

/// Returns true if a PolicyTarget matches the given target_type + target_id.
fn target_matches(target: &PolicyTarget, target_type: PolicyTargetType, target_id: &str) -> bool {
    if target.target_type != target_type {
        return false;
    }
    // If identifiers list is empty, the rule is a wildcard for this target type.
    if target.identifiers.is_empty() {
        return true;
    }
    // Exact match or glob-style trailing wildcard ("fs.*" matches "fs.read").
    target.identifiers.iter().any(|pattern| {
        if pattern == "*" {
            true
        } else if let Some(prefix) = pattern.strip_suffix('*') {
            target_id.starts_with(prefix)
        } else {
            pattern == target_id
        }
    })
}

/// Returns true if a host deny target overlaps with a pack allow identifier.
/// Handles wildcards in both directions:
///   - Host denies "fs.*" and pack allows "fs.delete" -> overlap (pack id starts with prefix)
///   - Host denies "fs.delete" and pack allows "fs.*" -> overlap (host id starts with pack prefix)
///   - Host denies "*" (empty identifiers = wildcard) and pack allows anything -> overlap
fn targets_overlap(host_target: &PolicyTarget, pack_id: &str) -> bool {
    // Empty identifiers on the host deny means it's a wildcard for the target type.
    if host_target.identifiers.is_empty() {
        return true;
    }

    // Pack wildcard ("*") overlaps with any host deny on the same target type.
    if pack_id == "*" {
        return true;
    }

    host_target.identifiers.iter().any(|host_pattern| {
        if host_pattern == "*" {
            // Host denies everything of this target type.
            true
        } else if let Some(host_prefix) = host_pattern.strip_suffix('*') {
            // Host denies a prefix pattern: "fs.*" denies anything starting with "fs.".
            // If pack allows "fs.delete", overlap.
            // If pack allows "fs.*" (itself a prefix), overlap if prefixes intersect.
            if let Some(pack_prefix) = pack_id.strip_suffix('*') {
                // Both are prefixes: overlap if one is a prefix of the other.
                host_prefix.starts_with(pack_prefix) || pack_prefix.starts_with(host_prefix)
            } else {
                pack_id.starts_with(host_prefix)
            }
        } else if let Some(pack_prefix) = pack_id.strip_suffix('*') {
            // Pack allows a prefix: "fs.*" would widen if host denies "fs.delete".
            host_pattern.starts_with(pack_prefix)
        } else {
            // Both are exact identifiers.
            host_pattern == pack_id
        }
    })
}

/// Evaluate whether all conditions on a rule are satisfied by the context.
fn conditions_met(conditions: &[PolicyCondition], context: &PolicyContext) -> bool {
    for cond in conditions {
        match cond.condition_type.as_str() {
            "trust_level_min" => {
                if let Some(required) = cond.expression.as_str() {
                    let required_level = parse_trust_level(required);
                    if let Some(req) = required_level
                        && context.trust_level < req
                    {
                        return false;
                    }
                }
            }
            "trust_level_max" => {
                if let Some(required) = cond.expression.as_str() {
                    let required_level = parse_trust_level(required);
                    if let Some(req) = required_level
                        && context.trust_level > req
                    {
                        return false;
                    }
                }
            }
            "namespace_eq" => {
                if let Some(ns) = cond.expression.as_str()
                    && context.namespace != ns
                {
                    return false;
                }
            }
            "namespace_prefix" => {
                if let Some(prefix) = cond.expression.as_str()
                    && !context.namespace.starts_with(prefix)
                {
                    return false;
                }
            }
            "budget_min" => {
                if let Some(min) = cond.expression.as_f64() {
                    match context.budget_remaining {
                        Some(b) if b >= min => {}
                        _ => return false,
                    }
                }
            }
            "session_required" => {
                if cond.expression.as_bool() == Some(true) && context.session_id.is_none() {
                    return false;
                }
            }
            // Rate-limit conditions are evaluated separately in the execution path,
            // not as a precondition filter. Always pass here.
            "rate_limit" => {}
            _ => {
                // Unknown condition type: treat as not met (restrictive default).
                return false;
            }
        }
    }
    true
}

fn parse_trust_level(s: &str) -> Option<TrustLevel> {
    match s {
        "untrusted" => Some(TrustLevel::Untrusted),
        "restricted" => Some(TrustLevel::Restricted),
        "verified" => Some(TrustLevel::Verified),
        "trusted" => Some(TrustLevel::Trusted),
        "built_in" => Some(TrustLevel::BuiltIn),
        _ => None,
    }
}

/// Map PolicyRuleType to PolicyEffect.
fn rule_type_to_effect(rt: PolicyRuleType) -> PolicyEffect {
    match rt {
        PolicyRuleType::Allow => PolicyEffect::Allow,
        PolicyRuleType::Deny => PolicyEffect::Deny,
        PolicyRuleType::RequireConfirmation => PolicyEffect::RequireConfirmation,
        PolicyRuleType::RequireEscalation => PolicyEffect::RequireEscalation,
        PolicyRuleType::ConstrainBudget => PolicyEffect::Constrain,
        PolicyRuleType::DowngradeTrust => PolicyEffect::Constrain,
        PolicyRuleType::ForceDelegationRejection => PolicyEffect::Deny,
        PolicyRuleType::RateLimit => PolicyEffect::Deny, // becomes deny when exceeded
    }
}

/// Restrictiveness ordering for effects. Higher = more restrictive.
fn effect_restrictiveness(effect: PolicyEffect) -> u8 {
    match effect {
        PolicyEffect::Allow => 0,
        PolicyEffect::Constrain => 1,
        PolicyEffect::RequireConfirmation => 2,
        PolicyEffect::RequireEscalation => 3,
        PolicyEffect::Deny => 4,
    }
}

// ---------------------------------------------------------------------------
// Rate-limit helpers on condition expressions
// ---------------------------------------------------------------------------

/// Extract rate-limit parameters from a RateLimit rule's conditions.
/// Looks for condition_type "rate_limit" with {"window_ms": N, "max_count": M}.
fn extract_rate_limit_params(conditions: &[PolicyCondition]) -> Option<(u64, u64)> {
    for cond in conditions {
        if cond.condition_type == "rate_limit" {
            let window = cond.expression.get("window_ms").and_then(|v| v.as_u64());
            let count = cond.expression.get("max_count").and_then(|v| v.as_u64());
            if let (Some(w), Some(c)) = (window, count) {
                return Some((w, c));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// DefaultPolicyRuntime implementation
// ---------------------------------------------------------------------------

impl DefaultPolicyRuntime {
    /// Core evaluation logic. Finds all matching rules, applies host-over-pack
    /// precedence and most-restrictive-wins semantics.
    fn evaluate_inner(
        &self,
        target_type: PolicyTargetType,
        target_id: &str,
        context: &PolicyContext,
    ) -> Result<PolicyDecision> {
        let rules = self.rules.read().map_err(|e| {
            SomaError::Policy(format!("lock poisoned: {}", e))
        })?;

        // Collect matching rules, partitioned by origin.
        let mut host_matches: Vec<&RegisteredRule> = Vec::new();
        let mut pack_matches: Vec<&RegisteredRule> = Vec::new();

        for reg in rules.iter() {
            if !target_matches(&reg.rule.target, target_type, target_id) {
                continue;
            }
            // Pack rules are scoped to their namespace. A rule registered under
            // namespace "pack_a" should not affect evaluations in "pack_b".
            // Host rules (namespace "host" or "system") apply globally.
            if reg.origin == PolicyOrigin::Pack
                && !reg.namespace.is_empty()
                && reg.namespace != context.namespace
            {
                continue;
            }
            if !conditions_met(&reg.rule.conditions, context) {
                continue;
            }
            match reg.origin {
                PolicyOrigin::Host => host_matches.push(reg),
                PolicyOrigin::Pack => pack_matches.push(reg),
            }
        }

        // If no rules matched at all, apply defaults.
        if host_matches.is_empty() && pack_matches.is_empty() {
            return Ok(self.default_decision(context));
        }

        // Evaluate host rules first — they always win.
        // Among host rules, pick the most restrictive.
        let host_decision = self.most_restrictive_decision(&host_matches, target_id)?;

        // Evaluate pack rules — pick the most restrictive.
        let pack_decision = self.most_restrictive_decision(&pack_matches, target_id)?;

        // Merge: host wins. If host is present, use it. If only pack, use pack.
        // If both present: host decision is authoritative, but if pack is MORE
        // restrictive, we adopt the pack restriction (pack can only tighten).
        let decision = match (host_decision, pack_decision) {
            (Some(h), None) => h,
            (None, Some(p)) => p,
            (Some(h), Some(p)) => {
                // Pack MUST NOT widen privilege. If pack says Allow but host
                // says Deny, host wins. If pack says Deny but host says Allow,
                // we adopt the more restrictive (pack). This is the
                // "contradiction resolves toward the more restrictive rule" rule.
                if effect_restrictiveness(p.effect) > effect_restrictiveness(h.effect) {
                    // Pack is more restrictive — adopt pack effect, but attribute to both.
                    let mut merged = p;
                    for rule_id in &h.matched_rules {
                        if !merged.matched_rules.contains(rule_id) {
                            merged.matched_rules.push(rule_id.clone());
                        }
                    }
                    merged
                } else {
                    h
                }
            }
            (None, None) => {
                // No rules produced a decision (all were rate-limit or similar)
                self.default_decision(context)
            }
        };

        Ok(decision)
    }

    /// From a set of matching rules, compute the most restrictive decision.
    /// Returns None if the slice is empty.
    fn most_restrictive_decision(
        &self,
        matches: &[&RegisteredRule],
        target_id: &str,
    ) -> Result<Option<PolicyDecision>> {
        if matches.is_empty() {
            return Ok(None);
        }

        // Sort by priority descending (higher priority = evaluated first),
        // then by restrictiveness descending.
        let mut sorted: Vec<&&RegisteredRule> = matches.iter().collect();
        sorted.sort_by(|a, b| {
            b.rule
                .priority
                .cmp(&a.rule.priority)
                .then_with(|| {
                    let ra = effect_restrictiveness(rule_type_to_effect(a.rule.rule_type));
                    let rb = effect_restrictiveness(rule_type_to_effect(b.rule.rule_type));
                    rb.cmp(&ra)
                })
        });

        let mut result_effect = PolicyEffect::Allow;
        let mut matched_rules: Vec<String> = Vec::new();
        let mut reasons: Vec<String> = Vec::new();
        let mut constraints: Option<serde_json::Value> = None;

        for reg in &sorted {
            let rule = &reg.rule;

            // Handle rate-limit rules specially.
            if rule.rule_type == PolicyRuleType::RateLimit {
                if let Some((window_ms, max_count)) =
                    extract_rate_limit_params(&rule.conditions)
                {
                    let key = RateLimitKey {
                        target_id: target_id.to_string(),
                        rule_id: rule.rule_id.clone(),
                    };
                    let mut buckets = self.rate_limits.write().map_err(|e| {
                        SomaError::Policy(format!("rate-limit lock poisoned: {}", e))
                    })?;
                    let bucket = buckets
                        .entry(key)
                        .or_insert_with(|| RateLimitBucket::new(window_ms, max_count));
                    if !bucket.check_and_record() {
                        // Rate limit exceeded: force deny.
                        matched_rules.push(rule.rule_id.clone());
                        reasons.push(format!(
                            "rate limit exceeded: {} invocations in {}ms window",
                            max_count, window_ms,
                        ));
                        result_effect = PolicyEffect::Deny;
                        // Deny is maximally restrictive, no need to continue.
                        break;
                    }
                }
                continue; // rate-limit rule that passed does not affect the decision
            }

            let effect = rule_type_to_effect(rule.rule_type);
            matched_rules.push(rule.rule_id.clone());

            if effect_restrictiveness(effect) > effect_restrictiveness(result_effect) {
                result_effect = effect;
                reasons.push(format!(
                    "rule {} ({:?}) on target {:?}",
                    rule.rule_id, rule.rule_type, rule.target.target_type,
                ));
            }

            // Collect constraints from ConstrainBudget / DowngradeTrust.
            if (rule.rule_type == PolicyRuleType::ConstrainBudget
                || rule.rule_type == PolicyRuleType::DowngradeTrust)
                && !rule.conditions.is_empty()
            {
                let constraint_data: Vec<_> = rule
                    .conditions
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "condition_type": c.condition_type,
                            "expression": c.expression,
                        })
                    })
                    .collect();
                constraints = Some(serde_json::json!(constraint_data));
            }
        }

        let allowed = result_effect == PolicyEffect::Allow;
        let reason = if reasons.is_empty() {
            "matched rules with no additional reason".to_string()
        } else {
            reasons.join("; ")
        };

        Ok(Some(PolicyDecision {
            allowed,
            effect: result_effect,
            matched_rules,
            reason,
            constraints,
        }))
    }

    /// Default decision when no rules match:
    /// - Default deny for destructive/irreversible actions.
    /// - Default allow for read-only or no-side-effect actions.
    /// - Default require-confirmation for external state mutations.
    fn default_decision(&self, context: &PolicyContext) -> PolicyDecision {
        match context.side_effect_class {
            Some(SideEffectClass::Destructive) | Some(SideEffectClass::Irreversible) => {
                PolicyDecision {
                    allowed: false,
                    effect: PolicyEffect::Deny,
                    matched_rules: vec![],
                    reason: "default deny: destructive or irreversible action with no matching rules"
                        .to_string(),
                    constraints: None,
                }
            }
            Some(SideEffectClass::ExternalStateMutation) => PolicyDecision {
                allowed: false,
                effect: PolicyEffect::RequireConfirmation,
                matched_rules: vec![],
                reason: "default: external state mutation requires confirmation".to_string(),
                constraints: None,
            },
            Some(SideEffectClass::None) | Some(SideEffectClass::ReadOnly) | None => {
                PolicyDecision {
                    allowed: true,
                    effect: PolicyEffect::Allow,
                    matched_rules: vec![],
                    reason: "default allow: no matching rules for read-only action".to_string(),
                    constraints: None,
                }
            }
            Some(SideEffectClass::LocalStateMutation) => PolicyDecision {
                allowed: true,
                effect: PolicyEffect::Allow,
                matched_rules: vec![],
                reason: "default allow: local state mutation with no matching rules".to_string(),
                constraints: None,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

impl PolicyRuntime for DefaultPolicyRuntime {
    fn register_policy(&self, spec: PolicySpec) -> Result<()> {
        // Determine origin from namespace convention: "host" or "host.*" is host origin,
        // everything else is pack origin.
        let origin = if spec.namespace == "host"
            || spec.namespace.starts_with("host.")
            || spec.namespace == "system"
            || spec.namespace.starts_with("system.")
        {
            PolicyOrigin::Host
        } else {
            PolicyOrigin::Pack
        };

        let mut rules = self.rules.write().map_err(|e| {
            SomaError::Policy(format!("lock poisoned: {}", e))
        })?;

        // For pack policies: validate that no rule widens privilege beyond host.
        if origin == PolicyOrigin::Pack {
            self.validate_pack_does_not_widen(&spec, &rules)?;
        }

        for rule in spec.rules {
            rules.push(RegisteredRule {
                rule,
                origin,
                namespace: spec.namespace.clone(),
            });
        }

        Ok(())
    }

    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyDecision> {
        self.evaluate_inner(request.target_type, &request.target_id, &request.context)
    }

    fn check_skill(&self, skill_id: &str, context: &PolicyContext) -> Result<PolicyDecision> {
        self.evaluate_inner(PolicyTargetType::Skill, skill_id, context)
    }

    fn check_port(
        &self,
        port_id: &str,
        capability_id: &str,
        context: &PolicyContext,
    ) -> Result<PolicyDecision> {
        // Check port-level first.
        let port_decision = self.evaluate_inner(PolicyTargetType::Port, port_id, context)?;
        if !port_decision.allowed && port_decision.effect == PolicyEffect::Deny {
            return Ok(port_decision);
        }

        // Check capability-level.
        let cap_decision =
            self.evaluate_inner(PolicyTargetType::Capability, capability_id, context)?;

        // Merge: take the more restrictive of the two.
        if effect_restrictiveness(cap_decision.effect)
            > effect_restrictiveness(port_decision.effect)
        {
            Ok(cap_decision)
        } else {
            Ok(port_decision)
        }
    }

    fn check_delegation(
        &self,
        peer_id: &str,
        action: &str,
        context: &PolicyContext,
    ) -> Result<PolicyDecision> {
        // Check peer-level rules.
        let peer_decision = self.evaluate_inner(PolicyTargetType::Peer, peer_id, context)?;
        if !peer_decision.allowed && peer_decision.effect == PolicyEffect::Deny {
            return Ok(peer_decision);
        }

        // Also check the action itself as a skill target (the delegated action).
        let action_decision = self.evaluate_inner(PolicyTargetType::Skill, action, context)?;

        if effect_restrictiveness(action_decision.effect)
            > effect_restrictiveness(peer_decision.effect)
        {
            Ok(action_decision)
        } else {
            Ok(peer_decision)
        }
    }

    fn check_remote_exposure(
        &self,
        capability: &str,
        context: &PolicyContext,
    ) -> Result<PolicyDecision> {
        self.evaluate_inner(PolicyTargetType::Capability, capability, context)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::policy::*;

    fn host_namespace() -> String {
        "host".to_string()
    }

    fn pack_namespace() -> String {
        "pack.myapp".to_string()
    }

    fn default_context() -> PolicyContext {
        PolicyContext {
            session_id: Some(Uuid::new_v4()),
            trust_level: TrustLevel::Verified,
            namespace: "default".to_string(),
            budget_remaining: Some(100.0),
            side_effect_class: None,
        }
    }

    fn make_rule(
        rule_id: &str,
        rule_type: PolicyRuleType,
        target_type: PolicyTargetType,
        identifiers: Vec<&str>,
        priority: i32,
    ) -> PolicyRule {
        PolicyRule {
            rule_id: rule_id.to_string(),
            rule_type,
            target: PolicyTarget {
                target_type,
                identifiers: identifiers.into_iter().map(String::from).collect(),
                scope: None,
                trust_level: None,
            },
            effect: rule_type_to_effect(rule_type),
            conditions: vec![],
            priority,
        }
    }

    // --- Registration tests ---

    #[test]
    fn register_host_policy() {
        let rt = DefaultPolicyRuntime::new();
        let spec = PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["fs.read"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        };
        assert!(rt.register_policy(spec).is_ok());
        assert_eq!(rt.rules.read().unwrap().len(), 1);
    }

    #[test]
    fn register_pack_policy() {
        let rt = DefaultPolicyRuntime::new();
        let spec = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                5,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        };
        assert!(rt.register_policy(spec).is_ok());
    }

    #[test]
    fn pack_cannot_widen_host_deny() {
        let rt = DefaultPolicyRuntime::new();

        // Host denies fs.delete.
        let host = PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                100,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        };
        rt.register_policy(host).unwrap();

        // Pack tries to allow fs.delete — must fail.
        let pack = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                50,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        };
        let result = rt.register_policy(pack);
        assert!(result.is_err());
    }

    // --- Evaluation tests ---

    #[test]
    fn evaluate_allow_rule() {
        let rt = DefaultPolicyRuntime::new();
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["fs.read"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_skill("fs.read", &default_context())
            .unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Allow);
        assert!(decision.matched_rules.contains(&"r1".to_string()));
    }

    #[test]
    fn evaluate_deny_rule() {
        let rt = DefaultPolicyRuntime::new();
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_skill("fs.delete", &default_context())
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn most_restrictive_wins() {
        let rt = DefaultPolicyRuntime::new();
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![
                make_rule(
                    "allow-r",
                    PolicyRuleType::Allow,
                    PolicyTargetType::Skill,
                    vec!["fs.write"],
                    5,
                ),
                make_rule(
                    "confirm-r",
                    PolicyRuleType::RequireConfirmation,
                    PolicyTargetType::Skill,
                    vec!["fs.write"],
                    10,
                ),
            ],
        ..Default::default()
        })
        .unwrap();

        let decision = rt
            .check_skill("fs.write", &default_context())
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::RequireConfirmation);
    }

    #[test]
    fn host_precedence_over_pack() {
        let rt = DefaultPolicyRuntime::new();

        // Host allows fs.read.
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["fs.read"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        // Pack denies fs.read (more restrictive — pack can tighten).
        rt.register_policy(PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.read"],
                5,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        // Use the pack namespace in context so the pack rule is in scope.
        let mut ctx = default_context();
        ctx.namespace = pack_namespace();
        let decision = rt
            .check_skill("fs.read", &ctx)
            .unwrap();
        // Pack is more restrictive, so the merged result denies.
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn default_deny_destructive_action() {
        let rt = DefaultPolicyRuntime::new();
        let mut ctx = default_context();
        ctx.side_effect_class = Some(SideEffectClass::Destructive);

        // No rules registered — should default deny.
        let decision = rt.check_skill("unregistered.action", &ctx).unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn default_deny_irreversible_action() {
        let rt = DefaultPolicyRuntime::new();
        let mut ctx = default_context();
        ctx.side_effect_class = Some(SideEffectClass::Irreversible);

        let decision = rt.check_skill("unregistered.action", &ctx).unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn default_allow_readonly_action() {
        let rt = DefaultPolicyRuntime::new();
        let mut ctx = default_context();
        ctx.side_effect_class = Some(SideEffectClass::ReadOnly);

        let decision = rt.check_skill("unregistered.action", &ctx).unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Allow);
    }

    #[test]
    fn default_allow_no_side_effect() {
        let rt = DefaultPolicyRuntime::new();
        let ctx = default_context();
        // side_effect_class is None.
        let decision = rt.check_skill("unregistered.action", &ctx).unwrap();
        assert!(decision.allowed);
    }

    #[test]
    fn default_require_confirmation_external_mutation() {
        let rt = DefaultPolicyRuntime::new();
        let mut ctx = default_context();
        ctx.side_effect_class = Some(SideEffectClass::ExternalStateMutation);

        let decision = rt.check_skill("unregistered.action", &ctx).unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::RequireConfirmation);
    }

    // --- Wildcard matching tests ---

    #[test]
    fn wildcard_identifier_matches() {
        let rt = DefaultPolicyRuntime::new();
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.*"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_skill("fs.delete", &default_context())
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn empty_identifiers_matches_all() {
        let rt = DefaultPolicyRuntime::new();
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::RequireConfirmation,
                PolicyTargetType::Skill,
                vec![], // empty = match all skills
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_skill("anything.here", &default_context())
            .unwrap();
        assert_eq!(decision.effect, PolicyEffect::RequireConfirmation);
    }

    // --- Condition tests ---

    #[test]
    fn trust_level_condition_blocks() {
        let rt = DefaultPolicyRuntime::new();

        let mut rule = make_rule(
            "r1",
            PolicyRuleType::Allow,
            PolicyTargetType::Skill,
            vec!["admin.reset"],
            10,
        );
        rule.conditions.push(PolicyCondition {
            condition_type: "trust_level_min".to_string(),
            expression: serde_json::json!("trusted"),
        });

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![rule],
        ..Default::default()
        })
        .unwrap();

        // Verified < Trusted, so the condition is not met and rule doesn't match.
        let mut ctx = default_context();
        ctx.trust_level = TrustLevel::Verified;
        let decision = rt.check_skill("admin.reset", &ctx).unwrap();
        // No rules matched; default allow (no side_effect_class set).
        assert!(decision.allowed);
        assert!(decision.matched_rules.is_empty());

        // With Trusted level, condition is met.
        ctx.trust_level = TrustLevel::Trusted;
        let decision = rt.check_skill("admin.reset", &ctx).unwrap();
        assert!(decision.allowed);
        assert!(decision.matched_rules.contains(&"r1".to_string()));
    }

    #[test]
    fn namespace_condition() {
        let rt = DefaultPolicyRuntime::new();

        let mut rule = make_rule(
            "r1",
            PolicyRuleType::Deny,
            PolicyTargetType::Skill,
            vec!["db.drop"],
            10,
        );
        rule.conditions.push(PolicyCondition {
            condition_type: "namespace_eq".to_string(),
            expression: serde_json::json!("production"),
        });

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![rule],
        ..Default::default()
        })
        .unwrap();

        // Namespace doesn't match — rule doesn't fire.
        let mut ctx = default_context();
        ctx.namespace = "staging".to_string();
        let decision = rt.check_skill("db.drop", &ctx).unwrap();
        assert!(decision.allowed);

        // Namespace matches — deny.
        ctx.namespace = "production".to_string();
        let decision = rt.check_skill("db.drop", &ctx).unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn budget_condition() {
        let rt = DefaultPolicyRuntime::new();

        let mut rule = make_rule(
            "r1",
            PolicyRuleType::Allow,
            PolicyTargetType::Skill,
            vec!["expensive.op"],
            10,
        );
        rule.conditions.push(PolicyCondition {
            condition_type: "budget_min".to_string(),
            expression: serde_json::json!(50.0),
        });

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![rule],
        ..Default::default()
        })
        .unwrap();

        // Budget sufficient.
        let mut ctx = default_context();
        ctx.budget_remaining = Some(75.0);
        let decision = rt.check_skill("expensive.op", &ctx).unwrap();
        assert!(decision.allowed);

        // Budget insufficient — condition not met, falls to default.
        ctx.budget_remaining = Some(10.0);
        let decision = rt.check_skill("expensive.op", &ctx).unwrap();
        assert!(decision.matched_rules.is_empty());
    }

    #[test]
    fn session_required_condition() {
        let rt = DefaultPolicyRuntime::new();

        let mut rule = make_rule(
            "r1",
            PolicyRuleType::Allow,
            PolicyTargetType::Skill,
            vec!["session.op"],
            10,
        );
        rule.conditions.push(PolicyCondition {
            condition_type: "session_required".to_string(),
            expression: serde_json::json!(true),
        });

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![rule],
        ..Default::default()
        })
        .unwrap();

        // With session.
        let ctx = default_context(); // has session_id
        let decision = rt.check_skill("session.op", &ctx).unwrap();
        assert!(decision.allowed);
        assert!(!decision.matched_rules.is_empty());

        // Without session.
        let mut ctx = default_context();
        ctx.session_id = None;
        let decision = rt.check_skill("session.op", &ctx).unwrap();
        assert!(decision.matched_rules.is_empty());
    }

    // --- Rate limiting tests ---

    #[test]
    fn rate_limit_allows_under_threshold() {
        let rt = DefaultPolicyRuntime::new();

        let mut rule = make_rule(
            "rl1",
            PolicyRuleType::RateLimit,
            PolicyTargetType::Skill,
            vec!["api.call"],
            10,
        );
        rule.conditions.push(PolicyCondition {
            condition_type: "rate_limit".to_string(),
            expression: serde_json::json!({"window_ms": 60000, "max_count": 5}),
        });

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![rule],
        ..Default::default()
        })
        .unwrap();

        // First 5 calls should succeed (default allow, rate limit rule passes).
        for _ in 0..5 {
            let decision = rt
                .check_skill("api.call", &default_context())
                .unwrap();
            assert!(decision.allowed);
        }
    }

    #[test]
    fn rate_limit_denies_over_threshold() {
        let rt = DefaultPolicyRuntime::new();

        let mut rule = make_rule(
            "rl1",
            PolicyRuleType::RateLimit,
            PolicyTargetType::Skill,
            vec!["api.call"],
            10,
        );
        rule.conditions.push(PolicyCondition {
            condition_type: "rate_limit".to_string(),
            expression: serde_json::json!({"window_ms": 60000, "max_count": 3}),
        });

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![rule],
        ..Default::default()
        })
        .unwrap();

        // 3 calls within the window.
        for _ in 0..3 {
            let decision = rt
                .check_skill("api.call", &default_context())
                .unwrap();
            assert!(decision.allowed);
        }

        // 4th call should be denied.
        let decision = rt
            .check_skill("api.call", &default_context())
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    // --- Port-level checks ---

    #[test]
    fn check_port_combines_port_and_capability() {
        let rt = DefaultPolicyRuntime::new();

        // Port-level: allow
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Allow,
                PolicyTargetType::Port,
                vec!["postgres"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        // Capability-level: deny one specific capability
        rt.register_policy(PolicySpec {
            policy_id: "host-2".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r2",
                PolicyRuleType::Deny,
                PolicyTargetType::Capability,
                vec!["drop_table"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        // Port allowed, but capability denied — result is deny.
        let decision = rt
            .check_port("postgres", "drop_table", &default_context())
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);

        // Port allowed, different capability — no deny rule, default allow.
        let decision = rt
            .check_port("postgres", "select_rows", &default_context())
            .unwrap();
        assert!(decision.allowed);
    }

    // --- Delegation checks ---

    #[test]
    fn check_delegation_denies_untrusted_peer() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::ForceDelegationRejection,
                PolicyTargetType::Peer,
                vec!["untrusted-peer"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_delegation("untrusted-peer", "fs.read", &default_context())
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn check_delegation_allows_trusted_peer() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Allow,
                PolicyTargetType::Peer,
                vec!["trusted-peer"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_delegation("trusted-peer", "fs.read", &default_context())
            .unwrap();
        assert!(decision.allowed);
    }

    // --- Remote exposure checks ---

    #[test]
    fn check_remote_exposure_denied() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Deny,
                PolicyTargetType::Capability,
                vec!["internal.admin"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_remote_exposure("internal.admin", &default_context())
            .unwrap();
        assert!(!decision.allowed);
    }

    #[test]
    fn check_remote_exposure_allowed() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Allow,
                PolicyTargetType::Capability,
                vec!["public.api"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_remote_exposure("public.api", &default_context())
            .unwrap();
        assert!(decision.allowed);
    }

    // --- General evaluate() ---

    #[test]
    fn evaluate_via_policy_request() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Deny,
                PolicyTargetType::Resource,
                vec!["secret.key"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let request = PolicyRequest {
            action: "access".to_string(),
            target_type: PolicyTargetType::Resource,
            target_id: "secret.key".to_string(),
            context: default_context(),
        };
        let decision = rt.evaluate(&request).unwrap();
        assert!(!decision.allowed);
    }

    // --- Priority ordering ---

    #[test]
    fn higher_priority_rule_wins() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![
                make_rule(
                    "low-allow",
                    PolicyRuleType::Allow,
                    PolicyTargetType::Skill,
                    vec!["admin.reset"],
                    1,
                ),
                make_rule(
                    "high-deny",
                    PolicyRuleType::Deny,
                    PolicyTargetType::Skill,
                    vec!["admin.reset"],
                    100,
                ),
            ],
        ..Default::default()
        })
        .unwrap();

        let decision = rt
            .check_skill("admin.reset", &default_context())
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    // --- Multiple policies merge ---

    #[test]
    fn multiple_policies_accumulate() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["fs.read"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        rt.register_policy(PolicySpec {
            policy_id: "host-2".to_string(),
            namespace: "host.security".to_string(),
            rules: vec![make_rule(
                "r2",
                PolicyRuleType::RequireEscalation,
                PolicyTargetType::Skill,
                vec!["fs.read"],
                20,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        let decision = rt
            .check_skill("fs.read", &default_context())
            .unwrap();
        // RequireEscalation is more restrictive than Allow.
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::RequireEscalation);
    }

    // --- Edge cases ---

    #[test]
    fn no_rules_no_side_effect_allows() {
        let rt = DefaultPolicyRuntime::new();
        let decision = rt
            .check_skill("nonexistent", &default_context())
            .unwrap();
        assert!(decision.allowed);
    }

    #[test]
    fn target_type_mismatch_does_not_match() {
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "r1",
                PolicyRuleType::Deny,
                PolicyTargetType::Port, // Port, not Skill
                vec!["fs.read"],
                10,
            )],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            scope_limits: None,
            trust_classification: None,
            confirmation_requirements: vec![],
            destructive_action_constraints: vec![],
            remote_exposure_limits: vec![],
        })
        .unwrap();

        // Checking as Skill — should not match the Port rule.
        let decision = rt
            .check_skill("fs.read", &default_context())
            .unwrap();
        assert!(decision.allowed);
    }

    // --- Pack widen validation (comprehensive) ---

    #[test]
    fn pack_cannot_widen_host_deny_second_identifier() {
        // The old code only checked the first identifier. Verify all are checked.
        let rt = DefaultPolicyRuntime::new();

        // Host denies fs.delete.
        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                100,
            )],
        ..Default::default()
        })
        .unwrap();

        // Pack tries to allow ["fs.read", "fs.delete"] — must fail because
        // fs.delete is denied by host, even though fs.read is fine.
        let pack = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![PolicyRule {
                rule_id: "p1".to_string(),
                rule_type: PolicyRuleType::Allow,
                target: PolicyTarget {
                    target_type: PolicyTargetType::Skill,
                    identifiers: vec!["fs.read".to_string(), "fs.delete".to_string()],
                    scope: None,
                    trust_level: None,
                },
                effect: PolicyEffect::Allow,
                conditions: vec![],
                priority: 50,
            }],
        ..Default::default()
        };
        assert!(rt.register_policy(pack).is_err());
    }

    #[test]
    fn pack_cannot_widen_host_wildcard_deny() {
        // Host denies fs.* — pack must not allow any fs.* action.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.*"],
                100,
            )],
        ..Default::default()
        })
        .unwrap();

        let pack = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["fs.write"],
                50,
            )],
        ..Default::default()
        };
        assert!(rt.register_policy(pack).is_err());
    }

    #[test]
    fn pack_wildcard_allow_blocked_by_host_specific_deny() {
        // Host denies "fs.delete". Pack tries to allow "fs.*" — rejected because
        // the wildcard allow covers the denied identifier.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                100,
            )],
        ..Default::default()
        })
        .unwrap();

        let pack = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["fs.*"],
                50,
            )],
        ..Default::default()
        };
        assert!(rt.register_policy(pack).is_err());
    }

    #[test]
    fn pack_empty_identifiers_allow_blocked_by_any_host_deny() {
        // Pack tries to allow all skills (empty identifiers = wildcard) but host
        // denies a specific one. Must be rejected.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["admin.reset"],
                100,
            )],
        ..Default::default()
        })
        .unwrap();

        let pack = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec![], // empty = wildcard
                50,
            )],
        ..Default::default()
        };
        assert!(rt.register_policy(pack).is_err());
    }

    #[test]
    fn pack_deny_on_host_deny_target_is_allowed() {
        // A pack may *deny* or tighten a target — only Allow rules that widen
        // are rejected.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                100,
            )],
        ..Default::default()
        })
        .unwrap();

        let pack = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                50,
            )],
        ..Default::default()
        };
        // Deny on an already-denied target is fine (tightening, not widening).
        assert!(rt.register_policy(pack).is_ok());
    }

    #[test]
    fn pack_allow_different_target_type_is_ok() {
        // Host denies fs.delete as a Skill. Pack allows fs.delete as a Port.
        // Different target types — no conflict.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["fs.delete"],
                100,
            )],
        ..Default::default()
        })
        .unwrap();

        let pack = PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Allow,
                PolicyTargetType::Port, // Different target type
                vec!["fs.delete"],
                50,
            )],
        ..Default::default()
        };
        assert!(rt.register_policy(pack).is_ok());
    }

    // --- Destructive operation policy ---

    #[test]
    fn destructive_op_denied_without_explicit_allow() {
        // No rules at all. check_destructive_operation should deny.
        let rt = DefaultPolicyRuntime::new();
        let ctx = default_context();

        let decision = rt.check_destructive_operation("db.drop_table", &ctx);
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn destructive_op_denied_with_only_pack_allow() {
        // Only a pack rule allows the action — not sufficient for destructive ops.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "pack-1".to_string(),
            namespace: pack_namespace(),
            rules: vec![make_rule(
                "p1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["db.drop_table"],
                10,
            )],
        ..Default::default()
        })
        .unwrap();

        // Use the pack namespace so the pack rule is in scope.
        let mut ctx = default_context();
        ctx.namespace = pack_namespace();
        let decision = rt.check_destructive_operation("db.drop_table", &ctx);
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
        assert!(decision.reason.contains("explicit host Allow"));
    }

    #[test]
    fn destructive_op_allowed_with_explicit_host_allow() {
        // A host rule explicitly allows the destructive action.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Allow,
                PolicyTargetType::Skill,
                vec!["db.drop_table"],
                10,
            )],
        ..Default::default()
        })
        .unwrap();

        let ctx = default_context();
        let decision = rt.check_destructive_operation("db.drop_table", &ctx);
        assert!(decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Allow);
    }

    #[test]
    fn destructive_op_denied_when_host_explicitly_denies() {
        // Host explicitly denies the action.
        let rt = DefaultPolicyRuntime::new();

        rt.register_policy(PolicySpec {
            policy_id: "host-1".to_string(),
            namespace: host_namespace(),
            rules: vec![make_rule(
                "h1",
                PolicyRuleType::Deny,
                PolicyTargetType::Skill,
                vec!["db.drop_table"],
                10,
            )],
        ..Default::default()
        })
        .unwrap();

        let ctx = default_context();
        let decision = rt.check_destructive_operation("db.drop_table", &ctx);
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }

    #[test]
    fn destructive_op_preserves_side_effect_class_from_context() {
        // If context already has Irreversible, check_destructive_operation uses
        // that instead of overriding to Destructive.
        let rt = DefaultPolicyRuntime::new();

        let mut ctx = default_context();
        ctx.side_effect_class = Some(SideEffectClass::Irreversible);

        let decision = rt.check_destructive_operation("nuke.everything", &ctx);
        assert!(!decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Deny);
    }
}
