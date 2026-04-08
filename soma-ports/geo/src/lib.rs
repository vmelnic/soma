//! SOMA Geo Port Pack -- 5 geolocation capabilities.
//!
//! | Capability       | Description                                        |
//! |------------------|----------------------------------------------------|
//! | distance         | Haversine great-circle distance between two points |
//! | radius_filter    | Filter a point array to entries within radius      |
//! | geocode          | Address to coordinates (stub without API key)      |
//! | reverse_geocode  | Coordinates to address (stub without API key)      |
//! | bounds_check     | Check if a point is within a bounding box          |
//!
//! Pure math for distance, radius_filter, and bounds_check -- no external
//! dependencies. Geocoding capabilities return errors unless an API key is
//! configured (there is no mock data; a production deployment would call
//! Nominatim, Google Maps, or similar).

use std::time::Instant;

use soma_port_sdk::prelude::*;

/// Earth's mean radius in kilometers (WGS-84 volumetric mean).
const EARTH_RADIUS_KM: f64 = 6371.0;

pub struct GeoPort {
    spec: PortSpec,
}

impl GeoPort {
    pub fn new() -> Self {
        Self { spec: build_spec() }
    }
}

impl Port for GeoPort {
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
            "distance" => exec_distance(&input),
            "radius_filter" => exec_radius_filter(&input),
            "geocode" => exec_geocode(&input),
            "reverse_geocode" => exec_reverse_geocode(&input),
            "bounds_check" => exec_bounds_check(&input),
            _ => {
                return Err(PortError::NotFound(format!(
                    "unknown capability: {capability_id}"
                )));
            }
        };
        let elapsed = start.elapsed().as_millis() as u64;
        match result {
            Ok(val) => Ok(PortCallRecord::success("geo", capability_id, val, elapsed)),
            Err(e) => Ok(PortCallRecord::failure(
                "geo",
                capability_id,
                e.failure_class(),
                &e.to_string(),
                elapsed,
            )),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        _input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        if self.spec.capabilities.iter().any(|c| c.capability_id == capability_id) {
            Ok(())
        } else {
            Err(PortError::NotFound(format!(
                "unknown capability: {capability_id}"
            )))
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Haversine
// ---------------------------------------------------------------------------

fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let a = (lat1_rad.cos() * lat2_rad.cos()).mul_add(
        (dlon / 2.0).sin().powi(2),
        (dlat / 2.0).sin().powi(2),
    );
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    EARTH_RADIUS_KM * c
}

// ---------------------------------------------------------------------------
// Input helpers
// ---------------------------------------------------------------------------

fn get_f64(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<f64> {
    input
        .get(field)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| PortError::Validation(format!("missing float field: {field}")))
}

fn get_str<'a>(input: &'a serde_json::Value, field: &str) -> soma_port_sdk::Result<&'a str> {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| PortError::Validation(format!("missing string field: {field}")))
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

fn exec_distance(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let lat1 = get_f64(input, "lat1")?;
    let lon1 = get_f64(input, "lon1")?;
    let lat2 = get_f64(input, "lat2")?;
    let lon2 = get_f64(input, "lon2")?;
    let km = haversine(lat1, lon1, lat2, lon2);
    Ok(serde_json::json!({ "distance_km": (km * 1000.0).round() / 1000.0 }))
}

fn exec_radius_filter(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let center_lat = get_f64(input, "lat")?;
    let center_lon = get_f64(input, "lon")?;
    let radius_km = get_f64(input, "radius_km")?;
    let points_json = get_str(input, "points")?;

    let points: Vec<serde_json::Value> = serde_json::from_str(points_json)
        .map_err(|e| PortError::Validation(format!("invalid JSON points array: {e}")))?;

    let mut results: Vec<serde_json::Value> = Vec::new();
    for point in &points {
        let lat = point
            .get("lat")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| PortError::Validation("each point must have a numeric 'lat' field".into()))?;
        let lon = point
            .get("lon")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| PortError::Validation("each point must have a numeric 'lon' field".into()))?;
        let dist = haversine(center_lat, center_lon, lat, lon);
        if dist <= radius_km {
            let mut entry = point.clone();
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "distance_km".into(),
                    serde_json::Value::Number(
                        serde_json::Number::from_f64((dist * 1000.0).round() / 1000.0)
                            .unwrap_or_else(|| serde_json::Number::from(0)),
                    ),
                );
            }
            results.push(entry);
        }
    }
    Ok(serde_json::json!({ "matches": results, "count": results.len() }))
}

fn exec_geocode(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let _address = get_str(input, "address")?;
    let api_key = input.get("api_key").and_then(|v| v.as_str());
    if api_key.is_none() {
        return Err(PortError::DependencyUnavailable(
            "geocoding requires an API key (set 'api_key' in input); \
             configure a Nominatim or Google Maps key for production use"
                .into(),
        ));
    }
    // With an API key present, a production implementation would make an HTTP
    // call here. For now, return an error indicating the capability is not
    // yet wired to a geocoding backend.
    Err(PortError::ExternalError(
        "geocoding backend not yet implemented; API key accepted but no HTTP call is made".into(),
    ))
}

fn exec_reverse_geocode(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let _lat = get_f64(input, "lat")?;
    let _lon = get_f64(input, "lon")?;
    let api_key = input.get("api_key").and_then(|v| v.as_str());
    if api_key.is_none() {
        return Err(PortError::DependencyUnavailable(
            "reverse geocoding requires an API key (set 'api_key' in input); \
             configure a Nominatim or Google Maps key for production use"
                .into(),
        ));
    }
    Err(PortError::ExternalError(
        "reverse geocoding backend not yet implemented; API key accepted but no HTTP call is made"
            .into(),
    ))
}

fn exec_bounds_check(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let lat = get_f64(input, "lat")?;
    let lon = get_f64(input, "lon")?;
    let min_lat = get_f64(input, "min_lat")?;
    let max_lat = get_f64(input, "max_lat")?;
    let min_lon = get_f64(input, "min_lon")?;
    let max_lon = get_f64(input, "max_lon")?;

    let within = lat >= min_lat && lat <= max_lat && lon >= min_lon && lon <= max_lon;
    Ok(serde_json::json!({ "within": within, "lat": lat, "lon": lon }))
}

// ---------------------------------------------------------------------------
// PortSpec builder
// ---------------------------------------------------------------------------

fn cap(
    id: &str,
    name: &str,
    purpose: &str,
    effect: SideEffectClass,
    determinism: DeterminismClass,
    latency_ms: u64,
) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.to_string(),
        name: name.to_string(),
        purpose: purpose.to_string(),
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        effect_class: effect,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: determinism,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile {
            expected_latency_ms: latency_ms,
            p95_latency_ms: latency_ms * 5,
            max_latency_ms: latency_ms * 20,
        },
        cost_profile: CostProfile::default(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: "geo".to_string(),
        name: "Geo".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Geolocation: distance calculation, radius filtering, geocoding, bounds checking".to_string(),
        namespace: "soma.ports.geo".to_string(),
        trust_level: TrustLevel::Trusted,
        capabilities: vec![
            cap("distance", "Haversine distance", "Compute great-circle distance between two lat/lon points", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("radius_filter", "Radius filter", "Filter a JSON array of points to those within a radius", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("geocode", "Geocode", "Convert address to lat/lon coordinates (requires API key)", SideEffectClass::None, DeterminismClass::DelegatedVariant, 500),
            cap("reverse_geocode", "Reverse geocode", "Convert lat/lon to address (requires API key)", SideEffectClass::None, DeterminismClass::DelegatedVariant, 500),
            cap("bounds_check", "Bounds check", "Check if a point falls within a bounding box", SideEffectClass::None, DeterminismClass::Deterministic, 1),
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError, PortFailureClass::DependencyUnavailable],
        side_effect_class: SideEffectClass::None,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 10,
            max_latency_ms: 1000,
        },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["capability_id".into(), "latency_ms".into()],
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
    Box::into_raw(Box::new(GeoPort::new()))
}
