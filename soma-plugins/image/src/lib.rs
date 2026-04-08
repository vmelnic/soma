//! SOMA Image Plugin -- image processing conventions for the SOMA runtime.
//!
//! Five conventions:
//!
//! | ID | Name              | Description                                          |
//! |----|-------------------|------------------------------------------------------|
//! | 0  | `thumbnail`       | Generate a thumbnail (fast, lower quality)           |
//! | 1  | `resize`          | Resize to exact dimensions (Lanczos3, high quality)  |
//! | 2  | `crop`            | Crop a rectangular region from an image              |
//! | 3  | `format_convert`  | Convert between PNG, JPEG, and WebP formats          |
//! | 4  | `exif_strip`      | Strip EXIF metadata by re-encoding the pixel data    |
//!
//! # Design choices
//!
//! - **`image` crate**: Pure Rust image decoding/encoding.  Supports PNG, JPEG,
//!   WebP, GIF, BMP, TIFF, and more.  No system dependencies (unlike ImageMagick
//!   or libvips bindings).
//!
//! - **Lanczos3 for resize**: High-quality downsampling filter that preserves
//!   sharp edges.  Slower than nearest-neighbor or bilinear but produces visibly
//!   superior results for photographic content.
//!
//! - **Thumbnail vs Resize**: `thumbnail()` uses a fast two-step algorithm
//!   (nearest-neighbor reduction then Gaussian smoothing) that is ~2-4x faster
//!   than `resize()` at the cost of some quality.  Use `thumbnail` for previews,
//!   `resize` for final output.
//!
//! - **EXIF stripping by re-encoding**: Rather than parsing and removing EXIF
//!   segments from the raw bytestream, we decode the image to pixels and
//!   re-encode it.  This is slightly slower but guaranteed to remove all metadata
//!   regardless of format quirks.
//!
//! - **Max dimension 16384**: Prevents accidental multi-gigabyte allocations from
//!   unreasonable resize/thumbnail requests.  16384px is 4x a 4K display edge.

#![allow(clippy::unnecessary_wraps)] // Convention methods must return Result per trait contract

use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use soma_plugin_sdk::prelude::*;

/// Maximum allowed dimension (width or height) for resize/thumbnail operations.
/// Prevents accidental multi-gigabyte RGBA allocations.
const MAX_DIMENSION: u32 = 16384;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The SOMA image processing plugin.
///
/// Stateless -- all image data is supplied per-call as `Value::Bytes`.
/// No buffers or caches are held between invocations.
pub struct ImagePlugin;

// ---------------------------------------------------------------------------
// SomaPlugin implementation
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_literal_bound)]
impl SomaPlugin for ImagePlugin {
    fn name(&self) -> &str {
        "image"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Image processing: thumbnail, resize, crop, format conversion, EXIF stripping"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::BuiltIn
    }

    #[allow(clippy::too_many_lines)]
    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: thumbnail
            Convention {
                id: 0,
                name: "thumbnail".into(),
                description: "Generate a thumbnail of an image (fast, lower quality than resize)"
                    .into(),
                call_pattern: "thumbnail(data, width, height)".into(),
                args: vec![
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Image data (any supported format)".into(),
                    },
                    ArgSpec {
                        name: "width".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Maximum thumbnail width in pixels".into(),
                    },
                    ArgSpec {
                        name: "height".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Maximum thumbnail height in pixels".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 10,
                max_latency_ms: 5000,
                side_effects: vec![],
                cleanup: None,
            },
            // 1: resize
            Convention {
                id: 1,
                name: "resize".into(),
                description:
                    "Resize an image to exact dimensions using Lanczos3 interpolation".into(),
                call_pattern: "resize(data, width, height)".into(),
                args: vec![
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Image data (any supported format)".into(),
                    },
                    ArgSpec {
                        name: "width".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Target width in pixels".into(),
                    },
                    ArgSpec {
                        name: "height".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Target height in pixels".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 50,
                max_latency_ms: 10000,
                side_effects: vec![],
                cleanup: None,
            },
            // 2: crop
            Convention {
                id: 2,
                name: "crop".into(),
                description: "Crop a rectangular region from an image".into(),
                call_pattern: "crop(data, x, y, w, h)".into(),
                args: vec![
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Image data (any supported format)".into(),
                    },
                    ArgSpec {
                        name: "x".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "X offset of crop region (pixels from left)".into(),
                    },
                    ArgSpec {
                        name: "y".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Y offset of crop region (pixels from top)".into(),
                    },
                    ArgSpec {
                        name: "w".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Width of crop region in pixels".into(),
                    },
                    ArgSpec {
                        name: "h".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Height of crop region in pixels".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 5,
                max_latency_ms: 5000,
                side_effects: vec![],
                cleanup: None,
            },
            // 3: format_convert
            Convention {
                id: 3,
                name: "format_convert".into(),
                description: "Convert image between formats (png, jpeg, webp)".into(),
                call_pattern: "format_convert(data, format)".into(),
                args: vec![
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Image data (any supported format)".into(),
                    },
                    ArgSpec {
                        name: "format".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Target format: \"png\", \"jpeg\", or \"webp\"".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 10,
                max_latency_ms: 5000,
                side_effects: vec![],
                cleanup: None,
            },
            // 4: exif_strip
            Convention {
                id: 4,
                name: "exif_strip".into(),
                description: "Strip EXIF metadata from an image by re-encoding pixel data".into(),
                call_pattern: "exif_strip(data)".into(),
                args: vec![ArgSpec {
                    name: "data".into(),
                    arg_type: ArgType::Bytes,
                    required: true,
                    description: "Image data (any supported format)".into(),
                }],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 10,
                max_latency_ms: 5000,
                side_effects: vec![],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => Self::thumbnail(&args),
            1 => Self::resize(&args),
            2 => Self::crop(&args),
            3 => Self::format_convert(&args),
            4 => Self::exif_strip(&args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention_id: {convention_id}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Convention implementations
// ---------------------------------------------------------------------------

impl ImagePlugin {
    /// Convention 0 -- Generate a thumbnail.
    ///
    /// Uses `DynamicImage::thumbnail()` which is faster than `resize()` but
    /// produces lower quality output (nearest-neighbor pre-shrink then smooth).
    /// The image is scaled to fit within the given bounds while preserving
    /// aspect ratio.  Output is always PNG.
    fn thumbnail(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;
        let width = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: width".into()))?
            .as_int()?;
        let height = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: height".into()))?
            .as_int()?;

        let (w, h) = validate_dimensions(width, height)?;

        let img = decode_image(data)?;
        let thumb = img.thumbnail(w, h);
        encode_png(&thumb)
    }

    /// Convention 1 -- Resize to exact dimensions with Lanczos3.
    ///
    /// Unlike `thumbnail`, this resizes to the *exact* width and height
    /// specified (may change aspect ratio).  Lanczos3 produces the highest
    /// quality output for photographic content.  Output is always PNG.
    fn resize(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;
        let width = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: width".into()))?
            .as_int()?;
        let height = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: height".into()))?
            .as_int()?;

        let (w, h) = validate_dimensions(width, height)?;

        let img = decode_image(data)?;
        let resized = img.resize_exact(w, h, FilterType::Lanczos3);
        encode_png(&resized)
    }

    /// Convention 2 -- Crop a rectangular region.
    ///
    /// Extracts the sub-image at (x, y) with dimensions (w, h).  Returns an
    /// error if the crop region extends beyond the image boundaries.
    /// Output is always PNG.
    fn crop(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;
        let x = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: x".into()))?
            .as_int()?;
        let y = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: y".into()))?
            .as_int()?;
        let w = args
            .get(3)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: w".into()))?
            .as_int()?;
        let h = args
            .get(4)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: h".into()))?
            .as_int()?;

        if x < 0 || y < 0 || w <= 0 || h <= 0 {
            return Err(PluginError::InvalidArg(
                "crop coordinates must be non-negative and dimensions must be positive".into(),
            ));
        }

        // Safety: validated to be non-negative above, so cast is lossless for
        // values within u32 range. Overflow beyond u32 is caught by the bounds
        // check below against the actual image dimensions.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (cx, cy, cw, ch) = (x as u32, y as u32, w as u32, h as u32);

        let img = decode_image(data)?;
        let (img_w, img_h) = (img.width(), img.height());

        // Check that the crop region fits within the image
        if cx.saturating_add(cw) > img_w || cy.saturating_add(ch) > img_h {
            return Err(PluginError::InvalidArg(format!(
                "crop region ({cx},{cy})+({cw}x{ch}) exceeds image bounds ({img_w}x{img_h})"
            )));
        }

        let cropped = img.crop_imm(cx, cy, cw, ch);
        encode_png(&cropped)
    }

    /// Convention 3 -- Convert image format.
    ///
    /// Decodes the input image (any supported format) and re-encodes it in
    /// the requested format.  Supported targets: `"png"`, `"jpeg"`, `"webp"`.
    fn format_convert(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;
        let format_str = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: format".into()))?
            .as_str()?;

        let format = parse_image_format(format_str)?;
        let img = decode_image(data)?;
        encode_format(&img, format)
    }

    /// Convention 4 -- Strip EXIF metadata.
    ///
    /// Decodes the image to raw pixels, then re-encodes it in the same format
    /// (detected from the input).  The re-encoding step discards all metadata
    /// chunks (EXIF, XMP, ICC profiles, etc.) because only pixel data is
    /// written back.
    ///
    /// Falls back to PNG if the input format cannot be determined.
    fn exif_strip(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;

        // Detect original format before decoding so we can re-encode to the
        // same format.  Falls back to PNG for unknown formats.
        let format = image::guess_format(data).unwrap_or(ImageFormat::Png);

        let img = decode_image(data)?;
        encode_format(&img, format)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decode image bytes into a `DynamicImage`.
fn decode_image(data: &[u8]) -> Result<DynamicImage, PluginError> {
    image::load_from_memory(data)
        .map_err(|e| PluginError::Failed(format!("failed to decode image: {e}")))
}

/// Validate and convert width/height from i64 to u32.
///
/// Both must be in the range `[1, MAX_DIMENSION]`.
fn validate_dimensions(width: i64, height: i64) -> Result<(u32, u32), PluginError> {
    if width <= 0 || height <= 0 {
        return Err(PluginError::InvalidArg(
            "width and height must be positive".into(),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (w, h) = (width as u32, height as u32);
    if w > MAX_DIMENSION || h > MAX_DIMENSION {
        return Err(PluginError::InvalidArg(format!(
            "dimensions exceed maximum ({MAX_DIMENSION}x{MAX_DIMENSION})"
        )));
    }
    Ok((w, h))
}

/// Parse a format string into an `ImageFormat`.
fn parse_image_format(s: &str) -> Result<ImageFormat, PluginError> {
    match s.to_lowercase().as_str() {
        "png" => Ok(ImageFormat::Png),
        "jpeg" | "jpg" => Ok(ImageFormat::Jpeg),
        "webp" => Ok(ImageFormat::WebP),
        other => Err(PluginError::InvalidArg(format!(
            "unsupported format \"{other}\"; supported: png, jpeg, webp"
        ))),
    }
}

/// Encode a `DynamicImage` as PNG and return as `Value::Bytes`.
fn encode_png(img: &DynamicImage) -> Result<Value, PluginError> {
    encode_format(img, ImageFormat::Png)
}

/// Encode a `DynamicImage` in the specified format and return as `Value::Bytes`.
fn encode_format(img: &DynamicImage, format: ImageFormat) -> Result<Value, PluginError> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, format)
        .map_err(|e| PluginError::Failed(format!("failed to encode image: {e}")))?;
    Ok(Value::Bytes(buf.into_inner()))
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

/// Create a heap-allocated `ImagePlugin` and return a raw pointer for dynamic loading.
///
/// Called by the SOMA runtime's `libloading`-based plugin loader.  The runtime
/// takes ownership of the pointer and drops it on unload.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(ImagePlugin))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    /// Create a test PNG image of the given dimensions filled with a solid color.
    fn make_test_png(width: u32, height: u32) -> Vec<u8> {
        let img = ImageBuffer::from_fn(width, height, |_x, _y| Rgba([255u8, 0, 0, 255]));
        let dyn_img = DynamicImage::ImageRgba8(img);
        let mut buf = Cursor::new(Vec::new());
        dyn_img.write_to(&mut buf, ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    /// Decode PNG bytes and return (width, height).
    fn png_dimensions(data: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(data).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn test_plugin_metadata() {
        let plugin = ImagePlugin;
        assert_eq!(plugin.name(), "image");
        assert_eq!(plugin.version(), "0.1.0");
        assert_eq!(plugin.conventions().len(), 5);
    }

    #[test]
    fn test_thumbnail() {
        let plugin = ImagePlugin;
        let png_100 = make_test_png(100, 100);

        let result = plugin
            .execute(
                0,
                vec![Value::Bytes(png_100), Value::Int(10), Value::Int(10)],
            )
            .unwrap();

        match result {
            Value::Bytes(data) => {
                let (w, h) = png_dimensions(&data);
                assert_eq!(w, 10);
                assert_eq!(h, 10);
            }
            other => panic!("expected Value::Bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_resize() {
        let plugin = ImagePlugin;
        let png_2x2 = make_test_png(2, 2);

        let result = plugin
            .execute(
                1,
                vec![Value::Bytes(png_2x2), Value::Int(4), Value::Int(4)],
            )
            .unwrap();

        match result {
            Value::Bytes(data) => {
                let (w, h) = png_dimensions(&data);
                assert_eq!(w, 4);
                assert_eq!(h, 4);
            }
            other => panic!("expected Value::Bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_crop() {
        let plugin = ImagePlugin;
        let png_10x10 = make_test_png(10, 10);

        // Crop a 5x5 region starting at (2, 3)
        let result = plugin
            .execute(
                2,
                vec![
                    Value::Bytes(png_10x10),
                    Value::Int(2),
                    Value::Int(3),
                    Value::Int(5),
                    Value::Int(5),
                ],
            )
            .unwrap();

        match result {
            Value::Bytes(data) => {
                let (w, h) = png_dimensions(&data);
                assert_eq!(w, 5);
                assert_eq!(h, 5);
            }
            other => panic!("expected Value::Bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_crop_out_of_bounds() {
        let plugin = ImagePlugin;
        let png_4x4 = make_test_png(4, 4);

        // Crop region exceeds image bounds
        let result = plugin.execute(
            2,
            vec![
                Value::Bytes(png_4x4),
                Value::Int(0),
                Value::Int(0),
                Value::Int(10),
                Value::Int(10),
            ],
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_format_convert_png_to_jpeg() {
        let plugin = ImagePlugin;
        let png_data = make_test_png(2, 2);

        let result = plugin
            .execute(
                3,
                vec![
                    Value::Bytes(png_data),
                    Value::String("jpeg".into()),
                ],
            )
            .unwrap();

        match result {
            Value::Bytes(data) => {
                // JPEG files start with FF D8 FF
                assert!(data.len() > 2);
                assert_eq!(data[0], 0xFF);
                assert_eq!(data[1], 0xD8);
            }
            other => panic!("expected Value::Bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_format_convert_unsupported() {
        let plugin = ImagePlugin;
        let png_data = make_test_png(2, 2);

        let result = plugin.execute(
            3,
            vec![
                Value::Bytes(png_data),
                Value::String("bmp".into()),
            ],
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_exif_strip() {
        let plugin = ImagePlugin;
        let png_data = make_test_png(2, 2);

        let result = plugin
            .execute(4, vec![Value::Bytes(png_data.clone())])
            .unwrap();

        match result {
            Value::Bytes(data) => {
                // Output should be a valid image
                let img = image::load_from_memory(&data);
                assert!(img.is_ok());
                let img = img.unwrap();
                assert_eq!(img.width(), 2);
                assert_eq!(img.height(), 2);
            }
            other => panic!("expected Value::Bytes, got {other:?}"),
        }
    }

    #[test]
    fn test_invalid_dimensions() {
        let plugin = ImagePlugin;
        let png_data = make_test_png(2, 2);

        // Zero width
        let result = plugin.execute(
            1,
            vec![Value::Bytes(png_data.clone()), Value::Int(0), Value::Int(10)],
        );
        assert!(result.is_err());

        // Negative height
        let result = plugin.execute(
            1,
            vec![Value::Bytes(png_data.clone()), Value::Int(10), Value::Int(-5)],
        );
        assert!(result.is_err());

        // Exceeds MAX_DIMENSION
        let result = plugin.execute(
            1,
            vec![Value::Bytes(png_data), Value::Int(20000), Value::Int(10)],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_convention() {
        let plugin = ImagePlugin;
        let result = plugin.execute(99, vec![]);
        assert!(result.is_err());
    }
}
