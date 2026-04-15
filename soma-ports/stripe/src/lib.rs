//! SOMA Stripe Port -- payment processing via the Stripe REST API.
//!
//! Five capabilities:
//!
//! | ID | Name                    | Description                           |
//! |----|-------------------------|---------------------------------------|
//! | 0  | `create_charge`         | Create a charge                       |
//! | 1  | `create_customer`       | Create a customer                     |
//! | 2  | `list_charges`          | List charges                          |
//! | 3  | `create_payment_intent` | Create a payment intent               |
//! | 4  | `get_balance`           | Retrieve account balance              |
//!
//! Uses `reqwest::blocking::Client` with Bearer auth against the Stripe API.
//! If no API key is configured, the port loads (lifecycle Active) but returns
//! an error on invoke explaining the missing config.

use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.stripe";
const STRIPE_API_BASE: &str = "https://api.stripe.com/v1";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct StripePort {
    spec: PortSpec,
    secret_key: Option<String>,
    client: reqwest::blocking::Client,
}

impl StripePort {
    pub fn new() -> Self {
        let secret_key = std::env::var("SOMA_STRIPE_SECRET_KEY")
            .ok()
            .or_else(|| std::env::var("STRIPE_SECRET_KEY").ok())
            .filter(|v| !v.is_empty());

        Self {
            spec: build_spec(),
            secret_key,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn require_key(&self) -> soma_port_sdk::Result<&str> {
        self.secret_key.as_deref().ok_or_else(|| {
            PortError::DependencyUnavailable(
                "Stripe API key not configured. Set SOMA_STRIPE_SECRET_KEY or STRIPE_SECRET_KEY"
                    .into(),
            )
        })
    }

    fn post_form(
        &self,
        url: &str,
        params: &[(&str, &str)],
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let key = self.require_key()?;
        let resp = self
            .client
            .post(url)
            .bearer_auth(key)
            .form(params)
            .send()
            .map_err(|e| PortError::TransportError(format!("Stripe request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse Stripe response: {e}")))?;

        if !status.is_success() {
            let msg = body["error"]["message"]
                .as_str()
                .unwrap_or("unknown Stripe error");
            return Err(PortError::ExternalError(format!(
                "Stripe API error ({status}): {msg}"
            )));
        }

        Ok(body)
    }

    fn get_json(&self, url: &str) -> soma_port_sdk::Result<serde_json::Value> {
        let key = self.require_key()?;
        let resp = self
            .client
            .get(url)
            .bearer_auth(key)
            .send()
            .map_err(|e| PortError::TransportError(format!("Stripe request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse Stripe response: {e}")))?;

        if !status.is_success() {
            let msg = body["error"]["message"]
                .as_str()
                .unwrap_or("unknown Stripe error");
            return Err(PortError::ExternalError(format!(
                "Stripe API error ({status}): {msg}"
            )));
        }

        Ok(body)
    }
}

impl Default for StripePort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for StripePort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "create_charge" => self.create_charge(&input),
            "create_customer" => self.create_customer(&input),
            "list_charges" => self.list_charges(&input),
            "create_payment_intent" => self.create_payment_intent(&input),
            "get_balance" => self.get_balance(),
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => Ok(PortCallRecord::success(
                PORT_ID,
                capability_id,
                value,
                latency_ms,
            )),
            Err(e) => Ok(PortCallRecord::failure(
                PORT_ID,
                capability_id,
                e.failure_class(),
                &e.to_string(),
                latency_ms,
            )),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        match capability_id {
            "create_charge" => {
                require_field(input, "amount")?;
                require_field(input, "currency")?;
                require_field(input, "source")?;
            }
            "create_customer" => {
                require_field(input, "email")?;
            }
            "list_charges" => { /* all optional */ }
            "create_payment_intent" => {
                require_field(input, "amount")?;
                require_field(input, "currency")?;
            }
            "get_balance" => { /* no input */ }
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl StripePort {
    fn create_charge(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let amount = get_str_or_number(input, "amount")?;
        let currency = get_str(input, "currency")?;
        let source = get_str(input, "source")?;

        let mut params: Vec<(&str, &str)> = vec![
            ("amount", &amount),
            ("currency", currency),
            ("source", source),
        ];

        let description;
        if let Some(d) = input.get("description").and_then(|v| v.as_str()) {
            description = d.to_string();
            params.push(("description", &description));
        }

        self.post_form(&format!("{STRIPE_API_BASE}/charges"), &params)
    }

    fn create_customer(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let email = get_str(input, "email")?;

        let mut params: Vec<(&str, &str)> = vec![("email", email)];

        let name;
        if let Some(n) = input.get("name").and_then(|v| v.as_str()) {
            name = n.to_string();
            params.push(("name", &name));
        }

        let description;
        if let Some(d) = input.get("description").and_then(|v| v.as_str()) {
            description = d.to_string();
            params.push(("description", &description));
        }

        self.post_form(&format!("{STRIPE_API_BASE}/customers"), &params)
    }

    fn list_charges(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let mut url = format!("{STRIPE_API_BASE}/charges");
        let mut query_parts: Vec<String> = Vec::new();

        if let Some(limit) = input.get("limit") {
            let l = limit
                .as_u64()
                .or_else(|| limit.as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(10);
            query_parts.push(format!("limit={l}"));
        }
        if let Some(sa) = input.get("starting_after").and_then(|v| v.as_str()) {
            query_parts.push(format!("starting_after={sa}"));
        }
        if !query_parts.is_empty() {
            url.push('?');
            url.push_str(&query_parts.join("&"));
        }

        self.get_json(&url)
    }

    fn create_payment_intent(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let amount = get_str_or_number(input, "amount")?;
        let currency = get_str(input, "currency")?;

        let mut params: Vec<(&str, &str)> = vec![
            ("amount", &amount),
            ("currency", currency),
        ];

        // payment_method_types is an array; Stripe expects payment_method_types[]=card etc.
        let pmt_strings: Vec<String>;
        if let Some(types) = input.get("payment_method_types").and_then(|v| v.as_array()) {
            pmt_strings = types
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            for t in &pmt_strings {
                params.push(("payment_method_types[]", t));
            }
        } else {
            params.push(("payment_method_types[]", "card"));
        }

        self.post_form(&format!("{STRIPE_API_BASE}/payment_intents"), &params)
    }

    fn get_balance(&self) -> soma_port_sdk::Result<serde_json::Value> {
        self.get_json(&format!("{STRIPE_API_BASE}/balance"))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_field(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<()> {
    if input.get(field).is_none() {
        return Err(PortError::Validation(format!("missing field: {field}")));
    }
    Ok(())
}

fn get_str<'a>(input: &'a serde_json::Value, field: &str) -> soma_port_sdk::Result<&'a str> {
    input[field]
        .as_str()
        .ok_or_else(|| PortError::Validation(format!("{field} must be a string")))
}

/// Get a field as a string, converting numbers to string representation.
/// Stripe API expects form-encoded values, so numbers need to be stringified.
fn get_str_or_number(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<String> {
    let val = input
        .get(field)
        .ok_or_else(|| PortError::Validation(format!("missing field: {field}")))?;
    if let Some(s) = val.as_str() {
        Ok(s.to_string())
    } else if let Some(n) = val.as_u64() {
        Ok(n.to_string())
    } else if let Some(n) = val.as_i64() {
        Ok(n.to_string())
    } else if let Some(n) = val.as_f64() {
        Ok(format!("{n:.0}"))
    } else {
        Err(PortError::Validation(format!(
            "{field} must be a string or number"
        )))
    }
}

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "stripe".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Stripe payment processing: charges, customers, payment intents, balance"
            .into(),
        namespace: "soma.stripe".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "create_charge".into(),
                name: "create_charge".into(),
                purpose: "Create a charge against a payment source".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "amount": {"type": "integer", "description": "Amount in cents"},
                    "currency": {"type": "string", "description": "Three-letter ISO currency code"},
                    "source": {"type": "string", "description": "Payment source token"},
                    "description": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "id": {"type": "string"},
                    "amount": {"type": "integer"},
                    "currency": {"type": "string"},
                    "status": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::High,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1000,
                    p95_latency_ms: 5000,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "create_customer".into(),
                name: "create_customer".into(),
                purpose: "Create a new Stripe customer".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "email": {"type": "string"},
                    "name": {"type": "string"},
                    "description": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "id": {"type": "string"},
                    "email": {"type": "string"},
                    "name": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 3000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "list_charges".into(),
                name: "list_charges".into(),
                purpose: "List recent charges".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "limit": {"type": "integer", "description": "Max number of charges to return"},
                    "starting_after": {"type": "string", "description": "Cursor for pagination"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "array"},
                    "has_more": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 3000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "create_payment_intent".into(),
                name: "create_payment_intent".into(),
                purpose: "Create a payment intent for client-side confirmation".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "amount": {"type": "integer", "description": "Amount in cents"},
                    "currency": {"type": "string", "description": "Three-letter ISO currency code"},
                    "payment_method_types": {"type": "array", "items": {"type": "string"}},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "id": {"type": "string"},
                    "client_secret": {"type": "string"},
                    "status": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Medium,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1000,
                    p95_latency_ms: 5000,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "get_balance".into(),
                name: "get_balance".into(),
                purpose: "Retrieve the current Stripe account balance".into(),
                input_schema: SchemaRef::object(serde_json::json!({})),
                output_schema: SchemaRef::object(serde_json::json!({
                    "available": {"type": "array"},
                    "pending": {"type": "array"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 300,
                    p95_latency_ms: 2000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::ExternalError,
            PortFailureClass::TransportError,
            PortFailureClass::Timeout,
            PortFailureClass::DependencyUnavailable,
            PortFailureClass::AuthorizationDenied,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1000,
            p95_latency_ms: 5000,
            max_latency_ms: 30000,
        },
        cost_profile: CostProfile {
            network_cost_class: CostClass::Medium,
            ..CostProfile::default()
        },
        auth_requirements: AuthRequirements {
            methods: vec![AuthMethod::ApiKey],
            required: true,
        },
        sandbox_requirements: SandboxRequirements {
            network_access: true,
            ..SandboxRequirements::default()
        },
        observable_fields: vec![],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(StripePort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = StripePort::new();
        assert_eq!(port.spec().port_id, "soma.stripe");
        assert_eq!(port.spec().capabilities.len(), 5);
    }

    #[test]
    fn test_lifecycle_active() {
        let port = StripePort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn test_validate_create_charge_missing_fields() {
        let port = StripePort::new();
        assert!(port
            .validate_input("create_charge", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_create_charge_ok() {
        let port = StripePort::new();
        let input = serde_json::json!({
            "amount": 1000,
            "currency": "usd",
            "source": "tok_visa"
        });
        assert!(port.validate_input("create_charge", &input).is_ok());
    }

    #[test]
    fn test_validate_get_balance_no_input() {
        let port = StripePort::new();
        assert!(port
            .validate_input("get_balance", &serde_json::json!({}))
            .is_ok());
    }

    #[test]
    fn test_unknown_capability() {
        let port = StripePort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }

    #[test]
    fn test_invoke_without_key_returns_failure_record() {
        let port = StripePort::new();
        // Without STRIPE_SECRET_KEY set, invoke should return a failure PortCallRecord
        let input = serde_json::json!({"amount": 1000, "currency": "usd", "source": "tok_visa"});
        let record = port.invoke("create_charge", input).unwrap();
        assert!(!record.success);
    }
}
