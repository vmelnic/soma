//! SOMA Calendar port pack — local iCalendar (.ics) file management.
//!
//! Provides 4 capabilities:
//!
//! - `create_event` — creates a .ics file for a new event
//! - `list_events` — reads .ics files with optional date filtering
//! - `delete_event` — deletes a .ics file by event ID
//! - `list_calendars` — lists subdirectories (calendars) in the calendar dir
//!
//! Each capability accepts JSON input and returns JSON output via the Port trait.
//! The calendar directory is read from `SOMA_CALENDAR_DIR` or defaults to
//! `~/.soma/calendars/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use chrono::{NaiveDateTime, Utc};
use semver::Version;
use soma_port_sdk::prelude::*;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct CalendarPort {
    spec: PortSpec,
    calendar_dir: OnceLock<PathBuf>,
}

#[derive(Clone, Copy)]
struct CapabilityBehavior {
    effect_class: SideEffectClass,
    rollback_support: RollbackSupport,
    determinism_class: DeterminismClass,
    idempotence_class: IdempotenceClass,
    risk_class: RiskClass,
}

impl CapabilityBehavior {
    fn new(
        effect_class: SideEffectClass,
        rollback_support: RollbackSupport,
        determinism_class: DeterminismClass,
        idempotence_class: IdempotenceClass,
        risk_class: RiskClass,
    ) -> Self {
        Self {
            effect_class,
            rollback_support,
            determinism_class,
            idempotence_class,
            risk_class,
        }
    }
}

impl Default for CalendarPort {
    fn default() -> Self {
        Self::new()
    }
}

impl CalendarPort {
    pub fn new() -> Self {
        let spec = Self::build_spec();
        Self {
            spec,
            calendar_dir: OnceLock::new(),
        }
    }

    fn calendar_dir(&self) -> &Path {
        self.calendar_dir.get_or_init(|| {
            if let Ok(dir) = std::env::var("SOMA_CALENDAR_DIR") {
                PathBuf::from(dir)
            } else if let Some(home) = dirs_home() {
                home.join(".soma").join("calendars")
            } else {
                PathBuf::from("/tmp/soma-calendars")
            }
        })
    }

    /// Ensure the calendar subdirectory exists.
    fn ensure_calendar_dir(&self, calendar: &str) -> std::result::Result<PathBuf, String> {
        Self::validate_name(calendar)?;
        let path = self.calendar_dir().join(calendar);
        fs::create_dir_all(&path)
            .map_err(|e| format!("failed to create calendar directory: {e}"))?;
        Ok(path)
    }

    /// Validate a calendar or event name contains only safe characters.
    fn validate_name(name: &str) -> std::result::Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("empty name".into());
        }
        for ch in trimmed.chars() {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
                continue;
            }
            return Err(format!("invalid character '{ch}' in name '{trimmed}'"));
        }
        Ok(())
    }

    /// Format a datetime as iCalendar DTSTART/DTEND value.
    fn format_ical_datetime(dt: &str) -> String {
        // Accept ISO 8601 format and convert to iCalendar format
        dt.replace('-', "")
            .replace(':', "")
            .replace(' ', "T")
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == 'T' || *c == 'Z')
            .collect()
    }

    /// Parse a .ics file into a JSON object with basic VEVENT fields.
    fn parse_ics(content: &str) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for line in content.lines() {
            let line = line.trim();
            if let Some((key, value)) = line.split_once(':') {
                let key = key.split(';').next().unwrap_or(key);
                match key {
                    "UID" => {
                        map.insert("event_id".to_string(), serde_json::json!(value));
                    }
                    "SUMMARY" => {
                        map.insert("summary".to_string(), serde_json::json!(value));
                    }
                    "DTSTART" => {
                        map.insert("start".to_string(), serde_json::json!(value));
                    }
                    "DTEND" => {
                        map.insert("end".to_string(), serde_json::json!(value));
                    }
                    "DESCRIPTION" => {
                        map.insert("description".to_string(), serde_json::json!(value));
                    }
                    "LOCATION" => {
                        map.insert("location".to_string(), serde_json::json!(value));
                    }
                    _ => {}
                }
            }
        }
        serde_json::Value::Object(map)
    }

    /// Try to parse an iCalendar datetime string into a NaiveDateTime for comparison.
    fn parse_ical_dt(s: &str) -> Option<NaiveDateTime> {
        // Try iCalendar format: 20260415T100000 or 20260415T100000Z
        let clean = s.trim_end_matches('Z');
        if let Ok(dt) = NaiveDateTime::parse_from_str(clean, "%Y%m%dT%H%M%S") {
            return Some(dt);
        }
        // Try ISO 8601 format
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
            return Some(dt);
        }
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
            return Some(dt);
        }
        None
    }

    // -----------------------------------------------------------------------
    // Observation helpers
    // -----------------------------------------------------------------------

    fn success_record(
        &self,
        capability_id: &str,
        result: serde_json::Value,
        effect_summary: &str,
        latency_ms: u64,
    ) -> PortCallRecord {
        PortCallRecord {
            observation_id: Uuid::new_v4(),
            port_id: self.spec.port_id.clone(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: true,
            failure_class: None,
            raw_result: result.clone(),
            structured_result: result,
            effect_patch: None,
            side_effect_summary: Some(effect_summary.to_string()),
            latency_ms,
            resource_cost: 0.0,
            confidence: 1.0,
            timestamp: Utc::now(),
            retry_safe: true,
            input_hash: None,
            session_id: None,
            goal_id: None,
            caller_identity: None,
            auth_result: None,
            policy_result: None,
            sandbox_result: None,
        }
    }

    fn failure_record(
        &self,
        capability_id: &str,
        failure_class: PortFailureClass,
        message: &str,
        latency_ms: u64,
    ) -> PortCallRecord {
        let retry_safe = matches!(
            failure_class,
            PortFailureClass::Timeout
                | PortFailureClass::DependencyUnavailable
                | PortFailureClass::TransportError
                | PortFailureClass::ExternalError
                | PortFailureClass::Unknown
        );
        PortCallRecord {
            observation_id: Uuid::new_v4(),
            port_id: self.spec.port_id.clone(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: false,
            failure_class: Some(failure_class),
            raw_result: serde_json::Value::Null,
            structured_result: serde_json::json!({ "error": message }),
            effect_patch: None,
            side_effect_summary: Some("none".to_string()),
            latency_ms,
            resource_cost: 0.0,
            confidence: 0.0,
            timestamp: Utc::now(),
            retry_safe,
            input_hash: None,
            session_id: None,
            goal_id: None,
            caller_identity: None,
            auth_result: None,
            policy_result: None,
            sandbox_result: None,
        }
    }

    // -----------------------------------------------------------------------
    // Capability implementations
    // -----------------------------------------------------------------------

    /// `create_event` -- creates a .ics file for a new event.
    fn do_create_event(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let calendar = input
            .get("calendar")
            .and_then(|v| v.as_str())
            .ok_or("missing 'calendar' field")?;

        let summary = input
            .get("summary")
            .and_then(|v| v.as_str())
            .ok_or("missing 'summary' field")?;

        let start = input
            .get("start")
            .and_then(|v| v.as_str())
            .ok_or("missing 'start' field")?;

        let end = input
            .get("end")
            .and_then(|v| v.as_str())
            .ok_or("missing 'end' field")?;

        let description = input.get("description").and_then(|v| v.as_str());
        let location = input.get("location").and_then(|v| v.as_str());

        let cal_dir = self.ensure_calendar_dir(calendar)?;
        let event_id = Uuid::new_v4().to_string();
        let filename = format!("{event_id}.ics");

        let dtstart = Self::format_ical_datetime(start);
        let dtend = Self::format_ical_datetime(end);
        let now = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();

        let mut ics = String::new();
        ics.push_str("BEGIN:VCALENDAR\r\n");
        ics.push_str("VERSION:2.0\r\n");
        ics.push_str("PRODID:-//SOMA//Calendar Port//EN\r\n");
        ics.push_str("BEGIN:VEVENT\r\n");
        ics.push_str(&format!("UID:{event_id}\r\n"));
        ics.push_str(&format!("DTSTAMP:{now}\r\n"));
        ics.push_str(&format!("DTSTART:{dtstart}\r\n"));
        ics.push_str(&format!("DTEND:{dtend}\r\n"));
        ics.push_str(&format!("SUMMARY:{summary}\r\n"));
        if let Some(desc) = description {
            ics.push_str(&format!("DESCRIPTION:{desc}\r\n"));
        }
        if let Some(loc) = location {
            ics.push_str(&format!("LOCATION:{loc}\r\n"));
        }
        ics.push_str("END:VEVENT\r\n");
        ics.push_str("END:VCALENDAR\r\n");

        let filepath = cal_dir.join(&filename);
        fs::write(&filepath, &ics)
            .map_err(|e| format!("failed to write .ics file: {e}"))?;

        Ok(serde_json::json!({
            "event_id": event_id,
            "file": filepath.to_string_lossy(),
            "calendar": calendar,
        }))
    }

    /// `list_events` -- reads .ics files with optional date filtering.
    fn do_list_events(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let calendar = input
            .get("calendar")
            .and_then(|v| v.as_str())
            .ok_or("missing 'calendar' field")?;

        let cal_dir = self.ensure_calendar_dir(calendar)?;

        let date_from = input
            .get("date_from")
            .and_then(|v| v.as_str())
            .and_then(Self::parse_ical_dt);
        let date_to = input
            .get("date_to")
            .and_then(|v| v.as_str())
            .and_then(Self::parse_ical_dt);

        let entries = fs::read_dir(&cal_dir)
            .map_err(|e| format!("failed to read calendar directory: {e}"))?;

        let mut events = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("directory entry error: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("ics") {
                continue;
            }

            let content = fs::read_to_string(&path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let event = Self::parse_ics(&content);

            // Apply date filtering if requested
            if let Some(ref from) = date_from {
                if let Some(start_str) = event.get("start").and_then(|v| v.as_str()) {
                    if let Some(start_dt) = Self::parse_ical_dt(start_str) {
                        if start_dt < *from {
                            continue;
                        }
                    }
                }
            }
            if let Some(ref to) = date_to {
                if let Some(start_str) = event.get("start").and_then(|v| v.as_str()) {
                    if let Some(start_dt) = Self::parse_ical_dt(start_str) {
                        if start_dt > *to {
                            continue;
                        }
                    }
                }
            }

            events.push(event);
        }

        Ok(serde_json::json!({ "events": events, "count": events.len() }))
    }

    /// `delete_event` -- deletes a .ics file by event ID.
    fn do_delete_event(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let calendar = input
            .get("calendar")
            .and_then(|v| v.as_str())
            .ok_or("missing 'calendar' field")?;

        let event_id = input
            .get("event_id")
            .and_then(|v| v.as_str())
            .ok_or("missing 'event_id' field")?;
        Self::validate_name(event_id)?;

        let cal_dir = self.ensure_calendar_dir(calendar)?;
        let filename = format!("{event_id}.ics");
        let filepath = cal_dir.join(&filename);

        if !filepath.exists() {
            return Err(format!("event '{event_id}' not found in calendar '{calendar}'"));
        }

        fs::remove_file(&filepath)
            .map_err(|e| format!("failed to delete event: {e}"))?;

        Ok(serde_json::json!({
            "deleted": event_id,
            "calendar": calendar,
        }))
    }

    /// `list_calendars` -- lists subdirectories in the calendar dir.
    fn do_list_calendars(&self) -> std::result::Result<serde_json::Value, String> {
        let base = self.calendar_dir();
        if !base.exists() {
            fs::create_dir_all(base)
                .map_err(|e| format!("failed to create calendar base directory: {e}"))?;
            return Ok(serde_json::json!({ "calendars": [], "count": 0 }));
        }

        let entries = fs::read_dir(base)
            .map_err(|e| format!("failed to read calendar directory: {e}"))?;

        let mut calendars = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("directory entry error: {e}"))?;
            if entry.file_type().map_or(false, |t| t.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    calendars.push(serde_json::json!(name));
                }
            }
        }

        Ok(serde_json::json!({ "calendars": calendars, "count": calendars.len() }))
    }

    // -----------------------------------------------------------------------
    // PortSpec builder
    // -----------------------------------------------------------------------

    fn build_spec() -> PortSpec {
        let any_schema = SchemaRef {
            schema: serde_json::json!({ "type": "object" }),
        };

        let low_cost = CostProfile {
            cpu_cost_class: CostClass::Negligible,
            memory_cost_class: CostClass::Negligible,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Negligible,
            energy_cost_class: CostClass::Negligible,
        };

        let fs_latency = LatencyProfile {
            expected_latency_ms: 5,
            p95_latency_ms: 50,
            max_latency_ms: 5_000,
        };

        let capabilities = vec![
            Self::cap(
                "create_event",
                "Create a new calendar event as a .ics file",
                CapabilityBehavior::new(
                    SideEffectClass::ExternalStateMutation,
                    RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::NonIdempotent,
                    RiskClass::Low,
                ),
                &fs_latency,
                &low_cost,
            ),
            Self::cap(
                "list_events",
                "List events in a calendar with optional date filtering",
                CapabilityBehavior::new(
                    SideEffectClass::ReadOnly,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Negligible,
                ),
                &fs_latency,
                &low_cost,
            ),
            Self::cap(
                "delete_event",
                "Delete an event by ID from a calendar",
                CapabilityBehavior::new(
                    SideEffectClass::Destructive,
                    RollbackSupport::Irreversible,
                    DeterminismClass::Deterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Medium,
                ),
                &fs_latency,
                &low_cost,
            ),
            Self::cap(
                "list_calendars",
                "List available calendars (subdirectories)",
                CapabilityBehavior::new(
                    SideEffectClass::ReadOnly,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Negligible,
                ),
                &fs_latency,
                &low_cost,
            ),
        ];

        PortSpec {
            port_id: "soma.calendar".to_string(),
            name: "calendar".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Filesystem,
            description:
                "Local iCalendar (.ics) file management: create, list, delete events and calendars"
                    .to_string(),
            namespace: "soma.ports".to_string(),
            trust_level: TrustLevel::Verified,
            capabilities,
            input_schema: any_schema.clone(),
            output_schema: any_schema,
            failure_modes: vec![
                PortFailureClass::ValidationError,
                PortFailureClass::ValidationError,
                PortFailureClass::ExternalError,
            ],
            side_effect_class: SideEffectClass::ExternalStateMutation,
            latency_profile: fs_latency,
            cost_profile: low_cost,
            auth_requirements: AuthRequirements {
                methods: vec![AuthMethod::LocalProcessTrust],
                required: false,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: true,
                network_access: false,
                device_access: false,
                process_access: false,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: Some(5_000),
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
        }
    }

    fn cap(
        name: &str,
        purpose: &str,
        behavior: CapabilityBehavior,
        latency_profile: &LatencyProfile,
        cost_profile: &CostProfile,
    ) -> PortCapabilitySpec {
        let any_schema = SchemaRef {
            schema: serde_json::json!({ "type": "object" }),
        };
        PortCapabilitySpec {
            capability_id: name.to_string(),
            name: name.to_string(),
            purpose: purpose.to_string(),
            input_schema: any_schema.clone(),
            output_schema: any_schema,
            effect_class: behavior.effect_class,
            rollback_support: behavior.rollback_support,
            determinism_class: behavior.determinism_class,
            idempotence_class: behavior.idempotence_class,
            risk_class: behavior.risk_class,
            latency_profile: latency_profile.clone(),
            cost_profile: cost_profile.clone(),
            remote_exposable: false,
            auth_override: None,
        }
    }
}

/// Get the user's home directory without pulling in a heavy dependency.
fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for CalendarPort {
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
            "create_event" => self.do_create_event(&input),
            "list_events" => self.do_list_events(&input),
            "delete_event" => self.do_delete_event(&input),
            "list_calendars" => self.do_list_calendars(),
            _ => {
                let latency_ms = start.elapsed().as_millis() as u64;
                return Ok(self.failure_record(
                    capability_id,
                    PortFailureClass::ValidationError,
                    &format!("unknown capability: {capability_id}"),
                    latency_ms,
                ));
            }
        };

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => {
                let effect = match capability_id {
                    "list_events" | "list_calendars" => "read_only",
                    "create_event" => "external_state_mutation",
                    "delete_event" => "destructive",
                    _ => "unknown",
                };
                Ok(self.success_record(capability_id, value, effect, latency_ms))
            }
            Err(msg) => {
                let failure_class = if msg.contains("not found") {
                    PortFailureClass::ValidationError
                } else {
                    PortFailureClass::ExternalError
                };
                Ok(self.failure_record(capability_id, failure_class, &msg, latency_ms))
            }
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        if !input.is_object() {
            return Err(PortError::Validation("input must be a JSON object".into()));
        }

        match capability_id {
            "create_event" => {
                if input.get("calendar").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'calendar' field".into()));
                }
                if input.get("summary").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'summary' field".into()));
                }
                if input.get("start").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'start' field".into()));
                }
                if input.get("end").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'end' field".into()));
                }
            }
            "list_events" => {
                if input.get("calendar").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'calendar' field".into()));
                }
            }
            "delete_event" => {
                if input.get("calendar").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'calendar' field".into()));
                }
                if input.get("event_id").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'event_id' field".into()));
                }
            }
            "list_calendars" => {}
            _ => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {capability_id}"
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
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    let port = CalendarPort::new();
    Box::into_raw(Box::new(port))
}
