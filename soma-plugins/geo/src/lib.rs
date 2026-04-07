//! SOMA Geo Plugin — 5 geolocation conventions.
//!
//! Provides: Haversine distance calculation, radius-based point filtering,
//! bounding box computation, geocoding (stub), and reverse geocoding (stub).
//!
//! All distance math is pure Rust with no external dependencies.
//! Geocoding conventions return mock data for well-known locations;
//! a production implementation would use Nominatim or a similar API.

use soma_plugin_sdk::prelude::*;
use std::collections::HashMap;

/// Earth's mean radius in kilometers.
const EARTH_RADIUS_KM: f64 = 6371.0;

/// The SOMA geo plugin.
pub struct GeoPlugin;

// ---------------------------------------------------------------------------
// Haversine helper
// ---------------------------------------------------------------------------

/// Compute the great-circle distance between two points using the Haversine
/// formula.  All arguments in decimal degrees; result in km.
fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();

    let a = (dlat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    EARTH_RADIUS_KM * c
}

// ---------------------------------------------------------------------------
// SomaPlugin implementation
// ---------------------------------------------------------------------------

impl SomaPlugin for GeoPlugin {
    fn name(&self) -> &str {
        "geo"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Geolocation: distance calculation, radius filtering, geocoding"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::BuiltIn
    }

    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: distance
            Convention {
                id: 0,
                name: "distance".into(),
                description: "Haversine distance between two coordinates in km".into(),
                call_pattern: "distance(lat1, lon1, lat2, lon2)".into(),
                args: vec![
                    ArgSpec {
                        name: "lat1".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Latitude of point 1 (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "lon1".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Longitude of point 1 (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "lat2".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Latitude of point 2 (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "lon2".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Longitude of point 2 (decimal degrees)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Float".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
            // 1: within_radius
            Convention {
                id: 1,
                name: "within_radius".into(),
                description: "Filter points within radius of a center point; returns JSON with distances".into(),
                call_pattern: "within_radius(lat, lon, radius_km, points)".into(),
                args: vec![
                    ArgSpec {
                        name: "lat".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Center latitude (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "lon".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Center longitude (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "radius_km".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Search radius in kilometers".into(),
                    },
                    ArgSpec {
                        name: "points".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "JSON array of objects with lat/lon fields".into(),
                    },
                ],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![],
                cleanup: None,
            },
            // 2: bounding_box
            Convention {
                id: 2,
                name: "bounding_box".into(),
                description: "Calculate lat/lon bounding box for a radius around a point".into(),
                call_pattern: "bounding_box(lat, lon, radius_km)".into(),
                args: vec![
                    ArgSpec {
                        name: "lat".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Center latitude (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "lon".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Center longitude (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "radius_km".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Radius in kilometers".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
            // 3: geocode
            Convention {
                id: 3,
                name: "geocode".into(),
                description: "Convert address to coordinates (stub — returns mock data for well-known places)".into(),
                call_pattern: "geocode(address)".into(),
                args: vec![ArgSpec {
                    name: "address".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Address or place name to geocode".into(),
                }],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 1000,
                side_effects: vec![],
                cleanup: None,
            },
            // 4: reverse_geocode
            Convention {
                id: 4,
                name: "reverse_geocode".into(),
                description: "Convert coordinates to address (stub — returns mock data for known areas)".into(),
                call_pattern: "reverse_geocode(lat, lon)".into(),
                args: vec![
                    ArgSpec {
                        name: "lat".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Latitude (decimal degrees)".into(),
                    },
                    ArgSpec {
                        name: "lon".into(),
                        arg_type: ArgType::Float,
                        required: true,
                        description: "Longitude (decimal degrees)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 1000,
                side_effects: vec![],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.distance(args),
            1 => self.within_radius(args),
            2 => self.bounding_box(args),
            3 => self.geocode(args),
            4 => self.reverse_geocode(args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention_id: {}",
                convention_id
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Convention implementations
// ---------------------------------------------------------------------------

impl GeoPlugin {
    /// Convention 0: Haversine distance between two lat/lon points (km).
    fn distance(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let lat1 = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lat1".into()))?
            .as_float()?;
        let lon1 = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lon1".into()))?
            .as_float()?;
        let lat2 = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lat2".into()))?
            .as_float()?;
        let lon2 = args
            .get(3)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lon2".into()))?
            .as_float()?;

        Ok(Value::Float(haversine(lat1, lon1, lat2, lon2)))
    }

    /// Convention 1: Filter a JSON array of points to those within `radius_km`
    /// of the given center.  Each input object must have `lat` and `lon` fields.
    /// Returns a JSON array with a `distance_km` field added to each match.
    fn within_radius(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let center_lat = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lat".into()))?
            .as_float()?;
        let center_lon = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lon".into()))?
            .as_float()?;
        let radius_km = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: radius_km".into()))?
            .as_float()?;
        let points_json = args
            .get(3)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: points".into()))?
            .as_str()?;

        let points: Vec<serde_json::Value> = serde_json::from_str(points_json)
            .map_err(|e| PluginError::InvalidArg(format!("invalid JSON points array: {}", e)))?;

        let mut results: Vec<serde_json::Value> = Vec::new();

        for point in &points {
            let lat = point
                .get("lat")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| {
                    PluginError::InvalidArg("each point must have a numeric 'lat' field".into())
                })?;
            let lon = point
                .get("lon")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| {
                    PluginError::InvalidArg("each point must have a numeric 'lon' field".into())
                })?;

            let dist = haversine(center_lat, center_lon, lat, lon);
            if dist <= radius_km {
                let mut entry = point.clone();
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert(
                        "distance_km".into(),
                        serde_json::Value::Number(
                            serde_json::Number::from_f64(
                                (dist * 1000.0).round() / 1000.0,
                            )
                            .unwrap_or_else(|| serde_json::Number::from(0)),
                        ),
                    );
                }
                results.push(entry);
            }
        }

        let json = serde_json::to_string(&results)
            .map_err(|e| PluginError::Failed(format!("JSON serialization failed: {}", e)))?;
        Ok(Value::String(json))
    }

    /// Convention 2: Compute a lat/lon bounding box for a radius around a point.
    /// Useful for pre-filtering database queries before applying exact Haversine.
    fn bounding_box(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let lat = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lat".into()))?
            .as_float()?;
        let lon = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lon".into()))?
            .as_float()?;
        let radius_km = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: radius_km".into()))?
            .as_float()?;

        // Angular distance in radians
        let angular = radius_km / EARTH_RADIUS_KM;

        let lat_rad = lat.to_radians();
        let lon_rad = lon.to_radians();

        let min_lat = (lat_rad - angular).to_degrees();
        let max_lat = (lat_rad + angular).to_degrees();

        // Longitude delta depends on latitude (narrower at poles)
        let delta_lon = (angular / lat_rad.cos()).asin();
        let min_lon = (lon_rad - delta_lon).to_degrees();
        let max_lon = (lon_rad + delta_lon).to_degrees();

        let mut map = HashMap::new();
        map.insert("min_lat".into(), Value::Float(min_lat));
        map.insert("max_lat".into(), Value::Float(max_lat));
        map.insert("min_lon".into(), Value::Float(min_lon));
        map.insert("max_lon".into(), Value::Float(max_lon));

        Ok(Value::Map(map))
    }

    /// Convention 3: Geocode an address to lat/lon (stub implementation).
    ///
    /// Returns mock coordinates for well-known places.  A production version
    /// would call Nominatim (OpenStreetMap) or a similar geocoding API.
    fn geocode(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let address = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: address".into()))?
            .as_str()?;

        let normalized = address.to_lowercase();

        // Well-known locations for mock/demo purposes
        let known: &[(&[&str], f64, f64, &str)] = &[
            (
                &["times square", "times sq"],
                40.7580,
                -73.9855,
                "Times Square, New York, NY, USA",
            ),
            (
                &["central park"],
                40.7829,
                -73.9654,
                "Central Park, New York, NY, USA",
            ),
            (
                &["eiffel tower", "tour eiffel"],
                48.8584,
                2.2945,
                "Eiffel Tower, Paris, France",
            ),
            (
                &["big ben", "westminster"],
                51.5007,
                -0.1246,
                "Big Ben, London, UK",
            ),
            (
                &["statue of liberty"],
                40.6892,
                -74.0445,
                "Statue of Liberty, New York, NY, USA",
            ),
            (
                &["golden gate bridge", "golden gate"],
                37.8199,
                -122.4783,
                "Golden Gate Bridge, San Francisco, CA, USA",
            ),
            (
                &["sydney opera house", "opera house sydney"],
                -33.8568,
                151.2153,
                "Sydney Opera House, Sydney, Australia",
            ),
            (
                &["tokyo tower"],
                35.6586,
                139.7454,
                "Tokyo Tower, Tokyo, Japan",
            ),
            (
                &["white house"],
                38.8977,
                -77.0365,
                "The White House, Washington, DC, USA",
            ),
            (
                &["123 main street", "123 main st"],
                40.7128,
                -74.0060,
                "123 Main Street, New York, NY, USA (approximate)",
            ),
        ];

        for (aliases, lat, lon, display_name) in known {
            for alias in *aliases {
                if normalized.contains(alias) {
                    let mut map = HashMap::new();
                    map.insert("lat".into(), Value::Float(*lat));
                    map.insert("lon".into(), Value::Float(*lon));
                    map.insert("display_name".into(), Value::String(display_name.to_string()));
                    map.insert(
                        "note".into(),
                        Value::String(
                            "stub result — real geocoding requires Nominatim or similar API"
                                .into(),
                        ),
                    );
                    return Ok(Value::Map(map));
                }
            }
        }

        // Unknown address
        Ok(Value::Null)
    }

    /// Convention 4: Reverse geocode lat/lon to an address (stub implementation).
    ///
    /// Returns mock addresses for coordinates near well-known cities.
    /// A production version would call Nominatim or a similar API.
    fn reverse_geocode(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let lat = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lat".into()))?
            .as_float()?;
        let lon = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: lon".into()))?
            .as_float()?;

        // Well-known city centers for stub matching (within ~50 km)
        let cities: &[(f64, f64, &str, &str, &str)] = &[
            (40.7128, -74.0060, "Manhattan, New York", "New York", "United States"),
            (48.8566, 2.3522, "Central Paris", "Paris", "France"),
            (51.5074, -0.1278, "City of London", "London", "United Kingdom"),
            (35.6762, 139.6503, "Shinjuku, Tokyo", "Tokyo", "Japan"),
            (-33.8688, 151.2093, "Sydney CBD", "Sydney", "Australia"),
            (37.7749, -122.4194, "Downtown San Francisco", "San Francisco", "United States"),
            (38.9072, -77.0369, "Downtown Washington", "Washington, DC", "United States"),
            (55.7558, 37.6173, "Central Moscow", "Moscow", "Russia"),
            (39.9042, 116.4074, "Dongcheng, Beijing", "Beijing", "China"),
            (-22.9068, -43.1729, "Centro, Rio de Janeiro", "Rio de Janeiro", "Brazil"),
        ];

        let threshold_km = 50.0;

        let mut best: Option<(f64, &str, &str, &str)> = None;

        for &(clat, clon, address, city, country) in cities {
            let dist = haversine(lat, lon, clat, clon);
            if dist <= threshold_km {
                if best.is_none() || dist < best.unwrap().0 {
                    best = Some((dist, address, city, country));
                }
            }
        }

        match best {
            Some((_dist, address, city, country)) => {
                let mut map = HashMap::new();
                map.insert("address".into(), Value::String(address.to_string()));
                map.insert("city".into(), Value::String(city.to_string()));
                map.insert("country".into(), Value::String(country.to_string()));
                map.insert(
                    "note".into(),
                    Value::String(
                        "stub result — real reverse geocoding requires Nominatim or similar API"
                            .into(),
                    ),
                );
                Ok(Value::Map(map))
            }
            None => Ok(Value::Null),
        }
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(GeoPlugin))
}
