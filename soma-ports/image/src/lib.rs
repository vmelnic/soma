//! SOMA Image Port -- image processing capabilities.
//!
//! Five capabilities:
//!
//! | ID | Name              | Description                                          |
//! |----|-------------------|------------------------------------------------------|
//! | 0  | `thumbnail`       | Generate a thumbnail (fast, lower quality)           |
//! | 1  | `resize`          | Resize to exact dimensions (Lanczos3, high quality)  |
//! | 2  | `crop`            | Crop a rectangular region from an image              |
//! | 3  | `format_convert`  | Convert between PNG, JPEG, and WebP formats          |
//! | 4  | `exif_strip`      | Strip EXIF metadata by re-encoding the pixel data    |
//!
//! Uses the `image` crate (pure Rust, no system dependencies).
//! Lanczos3 for resize, fast two-step algorithm for thumbnails.
//! Max dimension 16384 prevents multi-gigabyte allocations.

use std::io::Cursor;
use std::time::Instant;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use soma_port_sdk::prelude::*;

const MAX_DIMENSION: u32 = 16384;
const PORT_ID: &str = "soma.image";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct ImagePort {
    spec: PortSpec,
}

impl ImagePort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }
}

impl Default for ImagePort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for ImagePort {
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
            "thumbnail" => self.thumbnail(&input),
            "resize" => self.resize(&input),
            "crop" => self.crop(&input),
            "format_convert" => self.format_convert(&input),
            "exif_strip" => self.exif_strip(&input),
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )))
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => Ok(PortCallRecord::success(PORT_ID, capability_id, value, latency_ms)),
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
            "thumbnail" | "resize" => {
                require_field(input, "data")?;
                require_field(input, "width")?;
                require_field(input, "height")?;
            }
            "crop" => {
                for field in &["data", "x", "y", "w", "h"] {
                    require_field(input, field)?;
                }
            }
            "format_convert" => {
                require_field(input, "data")?;
                require_field(input, "format")?;
            }
            "exif_strip" => {
                require_field(input, "data")?;
            }
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )))
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

impl ImagePort {
    fn thumbnail(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let data = decode_base64_field(input, "data")?;
        let width = get_u32(input, "width")?;
        let height = get_u32(input, "height")?;
        validate_dimensions(width, height)?;

        let img = decode_image(&data)?;
        let thumb = img.thumbnail(width, height);
        let bytes = encode_png(&thumb)?;
        Ok(serde_json::json!({
            "data": base64_encode(&bytes),
            "width": thumb.width(),
            "height": thumb.height(),
            "format": "png",
        }))
    }

    fn resize(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let data = decode_base64_field(input, "data")?;
        let width = get_u32(input, "width")?;
        let height = get_u32(input, "height")?;
        validate_dimensions(width, height)?;

        let img = decode_image(&data)?;
        let resized = img.resize_exact(width, height, FilterType::Lanczos3);
        let bytes = encode_png(&resized)?;
        Ok(serde_json::json!({
            "data": base64_encode(&bytes),
            "width": resized.width(),
            "height": resized.height(),
            "format": "png",
        }))
    }

    fn crop(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let data = decode_base64_field(input, "data")?;
        let x = get_u32(input, "x")?;
        let y = get_u32(input, "y")?;
        let w = get_u32(input, "w")?;
        let h = get_u32(input, "h")?;

        if w == 0 || h == 0 {
            return Err(PortError::Validation(
                "crop width and height must be positive".into(),
            ));
        }

        let img = decode_image(&data)?;
        let (img_w, img_h) = (img.width(), img.height());

        if x.saturating_add(w) > img_w || y.saturating_add(h) > img_h {
            return Err(PortError::Validation(format!(
                "crop region ({x},{y})+({w}x{h}) exceeds image bounds ({img_w}x{img_h})"
            )));
        }

        let cropped = img.crop_imm(x, y, w, h);
        let bytes = encode_png(&cropped)?;
        Ok(serde_json::json!({
            "data": base64_encode(&bytes),
            "width": cropped.width(),
            "height": cropped.height(),
            "format": "png",
        }))
    }

    fn format_convert(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let data = decode_base64_field(input, "data")?;
        let format_str = input["format"]
            .as_str()
            .ok_or_else(|| PortError::Validation("format must be a string".into()))?;

        let format = parse_image_format(format_str)?;
        let img = decode_image(&data)?;
        let bytes = encode_format(&img, format)?;
        Ok(serde_json::json!({
            "data": base64_encode(&bytes),
            "width": img.width(),
            "height": img.height(),
            "format": format_str.to_lowercase(),
        }))
    }

    fn exif_strip(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let data = decode_base64_field(input, "data")?;
        let format = image::guess_format(&data).unwrap_or(ImageFormat::Png);
        let img = decode_image(&data)?;
        let bytes = encode_format(&img, format)?;
        Ok(serde_json::json!({
            "data": base64_encode(&bytes),
            "width": img.width(),
            "height": img.height(),
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

fn decode_base64_field(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<Vec<u8>> {
    let s = input[field]
        .as_str()
        .ok_or_else(|| PortError::Validation(format!("{field} must be a base64 string")))?;
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| PortError::Validation(format!("invalid base64 in {field}: {e}")))
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn get_u32(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<u32> {
    let n = input[field]
        .as_u64()
        .ok_or_else(|| PortError::Validation(format!("{field} must be a positive integer")))?;
    u32::try_from(n)
        .map_err(|_| PortError::Validation(format!("{field} too large for u32")))
}

fn validate_dimensions(width: u32, height: u32) -> soma_port_sdk::Result<()> {
    if width == 0 || height == 0 {
        return Err(PortError::Validation(
            "width and height must be positive".into(),
        ));
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(PortError::Validation(format!(
            "dimensions exceed maximum ({MAX_DIMENSION}x{MAX_DIMENSION})"
        )));
    }
    Ok(())
}

fn decode_image(data: &[u8]) -> soma_port_sdk::Result<DynamicImage> {
    image::load_from_memory(data)
        .map_err(|e| PortError::ExternalError(format!("failed to decode image: {e}")))
}

fn parse_image_format(s: &str) -> soma_port_sdk::Result<ImageFormat> {
    match s.to_lowercase().as_str() {
        "png" => Ok(ImageFormat::Png),
        "jpeg" | "jpg" => Ok(ImageFormat::Jpeg),
        "webp" => Ok(ImageFormat::WebP),
        other => Err(PortError::Validation(format!(
            "unsupported format \"{other}\"; supported: png, jpeg, webp"
        ))),
    }
}

fn encode_png(img: &DynamicImage) -> soma_port_sdk::Result<Vec<u8>> {
    encode_format(img, ImageFormat::Png)
}

fn encode_format(img: &DynamicImage, format: ImageFormat) -> soma_port_sdk::Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, format)
        .map_err(|e| PortError::ExternalError(format!("failed to encode image: {e}")))?;
    Ok(buf.into_inner())
}

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "image".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Image processing: thumbnail, resize, crop, format conversion, EXIF stripping"
            .into(),
        namespace: "soma.image".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "thumbnail".into(),
                name: "thumbnail".into(),
                purpose: "Generate a thumbnail of an image (fast, lower quality than resize)".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string", "description": "Base64-encoded image data"},
                    "width": {"type": "integer", "description": "Maximum thumbnail width"},
                    "height": {"type": "integer", "description": "Maximum thumbnail height"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "width": {"type": "integer"},
                    "height": {"type": "integer"}, "format": {"type": "string"},
                })),
                effect_class: SideEffectClass::None,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 10, p95_latency_ms: 100, max_latency_ms: 5000 },
                cost_profile: CostProfile { cpu_cost_class: CostClass::Low, ..CostProfile::default() },
                remote_exposable: true,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "resize".into(),
                name: "resize".into(),
                purpose: "Resize an image to exact dimensions using Lanczos3 interpolation".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "width": {"type": "integer"},
                    "height": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "width": {"type": "integer"},
                    "height": {"type": "integer"}, "format": {"type": "string"},
                })),
                effect_class: SideEffectClass::None,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 50, p95_latency_ms: 500, max_latency_ms: 10000 },
                cost_profile: CostProfile { cpu_cost_class: CostClass::Medium, ..CostProfile::default() },
                remote_exposable: true,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "crop".into(),
                name: "crop".into(),
                purpose: "Crop a rectangular region from an image".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "x": {"type": "integer"},
                    "y": {"type": "integer"}, "w": {"type": "integer"},
                    "h": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "width": {"type": "integer"},
                    "height": {"type": "integer"}, "format": {"type": "string"},
                })),
                effect_class: SideEffectClass::None,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 5, p95_latency_ms: 50, max_latency_ms: 5000 },
                cost_profile: CostProfile::default(),
                remote_exposable: true,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "format_convert".into(),
                name: "format_convert".into(),
                purpose: "Convert image between formats (png, jpeg, webp)".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "format": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "width": {"type": "integer"},
                    "height": {"type": "integer"}, "format": {"type": "string"},
                })),
                effect_class: SideEffectClass::None,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 10, p95_latency_ms: 100, max_latency_ms: 5000 },
                cost_profile: CostProfile::default(),
                remote_exposable: true,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "exif_strip".into(),
                name: "exif_strip".into(),
                purpose: "Strip EXIF metadata from an image by re-encoding pixel data".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "width": {"type": "integer"},
                    "height": {"type": "integer"},
                })),
                effect_class: SideEffectClass::None,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 10, p95_latency_ms: 100, max_latency_ms: 5000 },
                cost_profile: CostProfile::default(),
                remote_exposable: true,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: SideEffectClass::None,
        latency_profile: LatencyProfile { expected_latency_ms: 10, p95_latency_ms: 500, max_latency_ms: 10000 },
        cost_profile: CostProfile { cpu_cost_class: CostClass::Medium, ..CostProfile::default() },
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["width".into(), "height".into(), "format".into()],
        validation_rules: vec![],
        remote_exposure: true,
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(ImagePort::new()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = ImagePort::new();
        assert_eq!(port.spec().port_id, "soma.image");
        assert_eq!(port.spec().capabilities.len(), 5);
    }

    #[test]
    fn test_lifecycle() {
        let port = ImagePort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn test_validate_thumbnail_missing_field() {
        let port = ImagePort::new();
        let input = serde_json::json!({"data": "abc"});
        assert!(port.validate_input("thumbnail", &input).is_err());
    }

    #[test]
    fn test_unknown_capability() {
        let port = ImagePort::new();
        let result = port.invoke("nonexistent", serde_json::json!({}));
        assert!(result.is_err());
    }
}
