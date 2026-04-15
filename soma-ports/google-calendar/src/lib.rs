//! SOMA Google Calendar Port -- manage calendar events via the Google Calendar API.
//!
//! Four capabilities:
//!
//! | ID | Name            | Description                        |
//! |----|-----------------|------------------------------------|
//! | 0  | `list_events`   | List events from a calendar        |
//! | 1  | `create_event`  | Create a new calendar event        |
//! | 2  | `get_event`     | Get a single event by ID           |
//! | 3  | `delete_event`  | Delete an event by ID              |
//!
//! Auth: OAuth2 Bearer token via `SOMA_GOOGLE_ACCESS_TOKEN` or `GOOGLE_ACCESS_TOKEN`.

use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.google.calendar";
const BASE_URL: &str = "https://www.googleapis.com/calendar/v3";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct GoogleCalendarPort {
    spec: PortSpec,
}

impl GoogleCalendarPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }

    fn access_token() -> soma_port_sdk::Result<String> {
        std::env::var("SOMA_GOOGLE_ACCESS_TOKEN")
            .or_else(|_| std::env::var("GOOGLE_ACCESS_TOKEN"))
            .map_err(|_| {
                PortError::DependencyUnavailable(
                    "Google access token not set. Set SOMA_GOOGLE_ACCESS_TOKEN or GOOGLE_ACCESS_TOKEN"
                        .into(),
                )
            })
    }

    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::new()
    }
}

impl Default for GoogleCalendarPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for GoogleCalendarPort {
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
            "list_events" => self.list_events(&input),
            "create_event" => self.create_event(&input),
            "get_event" => self.get_event(&input),
            "delete_event" => self.delete_event(&input),
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
            "list_events" => {}
            "create_event" => {
                require_field(input, "summary")?;
                require_field(input, "start")?;
                require_field(input, "end")?;
            }
            "get_event" => {
                require_field(input, "event_id")?;
            }
            "delete_event" => {
                require_field(input, "event_id")?;
            }
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        if Self::access_token().is_ok() {
            PortLifecycleState::Active
        } else {
            PortLifecycleState::Loaded
        }
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl GoogleCalendarPort {
    fn calendar_id(input: &serde_json::Value) -> &str {
        input
            .get("calendar_id")
            .and_then(|v| v.as_str())
            .unwrap_or("primary")
    }

    fn list_events(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let cal_id = Self::calendar_id(input);

        let mut request = Self::client()
            .get(format!("{BASE_URL}/calendars/{cal_id}/events"))
            .bearer_auth(&token);

        if let Some(time_min) = input.get("time_min").and_then(|v| v.as_str()) {
            request = request.query(&[("timeMin", time_min)]);
        }
        if let Some(time_max) = input.get("time_max").and_then(|v| v.as_str()) {
            request = request.query(&[("timeMax", time_max)]);
        }
        if let Some(max_results) = input.get("max_results").and_then(|v| v.as_u64()) {
            request = request.query(&[("maxResults", &max_results.to_string())]);
        }

        let resp = request
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Google Calendar API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn create_event(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let cal_id = Self::calendar_id(input);
        let summary = get_str(input, "summary")?;
        let start = get_str(input, "start")?;
        let end = get_str(input, "end")?;

        let mut event_body = serde_json::json!({
            "summary": summary,
            "start": { "dateTime": start },
            "end": { "dateTime": end },
        });

        if let Some(desc) = input.get("description").and_then(|v| v.as_str()) {
            event_body["description"] = serde_json::json!(desc);
        }
        if let Some(loc) = input.get("location").and_then(|v| v.as_str()) {
            event_body["location"] = serde_json::json!(loc);
        }

        let resp = Self::client()
            .post(format!("{BASE_URL}/calendars/{cal_id}/events"))
            .bearer_auth(&token)
            .json(&event_body)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Google Calendar API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn get_event(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let cal_id = Self::calendar_id(input);
        let event_id = get_str(input, "event_id")?;

        let resp = Self::client()
            .get(format!("{BASE_URL}/calendars/{cal_id}/events/{event_id}"))
            .bearer_auth(&token)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Google Calendar API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn delete_event(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let cal_id = Self::calendar_id(input);
        let event_id = get_str(input, "event_id")?;

        let resp = Self::client()
            .delete(format!(
                "{BASE_URL}/calendars/{cal_id}/events/{event_id}"
            ))
            .bearer_auth(&token)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body: serde_json::Value = resp.json().unwrap_or_default();
            return Err(PortError::ExternalError(format!(
                "Google Calendar API error {status}: {body}"
            )));
        }

        Ok(serde_json::json!({
            "deleted": true,
            "event_id": event_id,
        }))
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

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "google-calendar".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Http,
        description: "Google Calendar API: list, create, get, and delete calendar events".into(),
        namespace: "soma.google.calendar".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "list_events".into(),
                name: "list_events".into(),
                purpose: "List events from a Google Calendar".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "calendar_id": {"type": "string", "description": "Calendar ID (default: primary)"},
                    "time_min": {"type": "string", "description": "RFC3339 lower bound for event start"},
                    "time_max": {"type": "string", "description": "RFC3339 upper bound for event start"},
                    "max_results": {"type": "integer", "description": "Maximum number of events to return"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
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
            PortCapabilitySpec {
                capability_id: "create_event".into(),
                name: "create_event".into(),
                purpose: "Create a new event on a Google Calendar".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "calendar_id": {"type": "string", "description": "Calendar ID (default: primary)"},
                    "summary": {"type": "string"},
                    "start": {"type": "string", "description": "RFC3339 start datetime"},
                    "end": {"type": "string", "description": "RFC3339 end datetime"},
                    "description": {"type": "string"},
                    "location": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
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
                capability_id: "get_event".into(),
                name: "get_event".into(),
                purpose: "Get a single calendar event by ID".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "calendar_id": {"type": "string", "description": "Calendar ID (default: primary)"},
                    "event_id": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 200,
                    p95_latency_ms: 1500,
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
                capability_id: "delete_event".into(),
                name: "delete_event".into(),
                purpose: "Delete a calendar event by ID".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "calendar_id": {"type": "string", "description": "Calendar ID (default: primary)"},
                    "event_id": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "deleted": {"type": "boolean"},
                    "event_id": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Medium,
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
            expected_latency_ms: 300,
            p95_latency_ms: 3000,
            max_latency_ms: 10000,
        },
        cost_profile: CostProfile {
            network_cost_class: CostClass::Low,
            ..CostProfile::default()
        },
        auth_requirements: AuthRequirements {
            methods: vec![AuthMethod::BearerToken],
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
    Box::into_raw(Box::new(GoogleCalendarPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = GoogleCalendarPort::new();
        assert_eq!(port.spec().port_id, "soma.google.calendar");
        assert_eq!(port.spec().capabilities.len(), 4);
    }

    #[test]
    fn test_lifecycle_without_token() {
        // Without env var set, port should be Loaded (not Active)
        unsafe { std::env::remove_var("SOMA_GOOGLE_ACCESS_TOKEN") };
        unsafe { std::env::remove_var("GOOGLE_ACCESS_TOKEN") };
        let port = GoogleCalendarPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Loaded);
    }

    #[test]
    fn test_validate_create_event_missing_fields() {
        let port = GoogleCalendarPort::new();
        assert!(port
            .validate_input("create_event", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_create_event_all_fields() {
        let port = GoogleCalendarPort::new();
        let input = serde_json::json!({
            "summary": "Test",
            "start": "2026-04-14T10:00:00Z",
            "end": "2026-04-14T11:00:00Z",
        });
        assert!(port.validate_input("create_event", &input).is_ok());
    }

    #[test]
    fn test_validate_list_events_no_required_fields() {
        let port = GoogleCalendarPort::new();
        assert!(port
            .validate_input("list_events", &serde_json::json!({}))
            .is_ok());
    }

    #[test]
    fn test_validate_get_event_missing_id() {
        let port = GoogleCalendarPort::new();
        assert!(port
            .validate_input("get_event", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_delete_event_missing_id() {
        let port = GoogleCalendarPort::new();
        assert!(port
            .validate_input("delete_event", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_unknown_capability() {
        let port = GoogleCalendarPort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }
}
