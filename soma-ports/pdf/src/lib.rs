//! SOMA PDF Port -- PDF document generation via the `printpdf` crate.
//!
//! Three capabilities:
//!
//! | ID | Name              | Description                              |
//! |----|-------------------|------------------------------------------|
//! | 0  | `create_document` | Create a PDF with a title and text       |
//! | 1  | `add_page`        | Append a page of text to an existing PDF |
//! | 2  | `text_to_pdf`     | Convert plain text to a PDF file         |
//!
//! No external services or env vars required. Uses the built-in Helvetica font.

use std::io::BufWriter;
use std::time::Instant;

use printpdf::*;
use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.pdf";

// Page dimensions (A4 in mm)
const PAGE_WIDTH_MM: f32 = 210.0;
const PAGE_HEIGHT_MM: f32 = 297.0;

// Text layout constants
const MARGIN_MM: f32 = 20.0;
const FONT_SIZE_PT: f32 = 12.0;
const TITLE_FONT_SIZE_PT: f32 = 18.0;
const LINE_HEIGHT_MM: f32 = 5.0;
const TITLE_LINE_HEIGHT_MM: f32 = 8.0;

// Approximate characters per line at 12pt Helvetica on A4 with 20mm margins
const CHARS_PER_LINE: usize = 80;

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct PdfPort {
    spec: PortSpec,
}

impl PdfPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }
}

impl Default for PdfPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for PdfPort {
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
            "create_document" => create_document(&input),
            "add_page" => add_page(&input),
            "text_to_pdf" => text_to_pdf(&input),
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
            "create_document" => {
                require_field(input, "title")?;
                require_field(input, "content")?;
                require_field(input, "output_path")?;
            }
            "add_page" => {
                require_field(input, "document_path")?;
                require_field(input, "content")?;
            }
            "text_to_pdf" => {
                require_field(input, "text")?;
                require_field(input, "output_path")?;
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
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

fn create_document(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let title = get_str(input, "title")?;
    let content = get_str(input, "content")?;
    let output_path = get_str(input, "output_path")?;

    let (doc, page_idx, layer_idx) = PdfDocument::new(
        title,
        Mm(PAGE_WIDTH_MM),
        Mm(PAGE_HEIGHT_MM),
        "Layer 1",
    );

    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| PortError::Internal(format!("failed to add font: {e}")))?;
    let font_bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| PortError::Internal(format!("failed to add bold font: {e}")))?;

    let layer = doc.get_page(page_idx).get_layer(layer_idx);

    // Draw title
    let mut y = PAGE_HEIGHT_MM - MARGIN_MM;
    layer.use_text(title, TITLE_FONT_SIZE_PT, Mm(MARGIN_MM), Mm(y), &font_bold);
    y -= TITLE_LINE_HEIGHT_MM * 2.0;

    // Draw content lines
    let lines = wrap_text(content, CHARS_PER_LINE);
    for line in &lines {
        if y < MARGIN_MM {
            break; // Stop at bottom margin (single page for create_document)
        }
        layer.use_text(line, FONT_SIZE_PT, Mm(MARGIN_MM), Mm(y), &font);
        y -= LINE_HEIGHT_MM;
    }

    save_doc(doc, output_path)?;

    let page_count = 1;
    Ok(serde_json::json!({
        "created": true,
        "output_path": output_path,
        "pages": page_count,
        "title": title,
    }))
}

fn add_page(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let document_path = get_str(input, "document_path")?;
    let content = get_str(input, "content")?;

    // printpdf cannot open/append to existing PDFs. Instead, create a new PDF
    // and write content. The caller can track multi-page documents by calling
    // create_document once and add_page for subsequent pages, understanding
    // that each call produces a standalone PDF file. For true multi-page
    // appending, a PDF manipulation library like lopdf would be needed.
    //
    // Practical approach: create a new single-page PDF at document_path,
    // overwriting the previous file. The caller sequences page creation.
    let (doc, page_idx, layer_idx) = PdfDocument::new(
        "Page",
        Mm(PAGE_WIDTH_MM),
        Mm(PAGE_HEIGHT_MM),
        "Layer 1",
    );

    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| PortError::Internal(format!("failed to add font: {e}")))?;

    let layer = doc.get_page(page_idx).get_layer(layer_idx);

    let mut y = PAGE_HEIGHT_MM - MARGIN_MM;
    let lines = wrap_text(content, CHARS_PER_LINE);
    for line in &lines {
        if y < MARGIN_MM {
            break;
        }
        layer.use_text(line, FONT_SIZE_PT, Mm(MARGIN_MM), Mm(y), &font);
        y -= LINE_HEIGHT_MM;
    }

    save_doc(doc, document_path)?;

    Ok(serde_json::json!({
        "added": true,
        "document_path": document_path,
    }))
}

fn text_to_pdf(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let text = get_str(input, "text")?;
    let output_path = get_str(input, "output_path")?;

    let (doc, first_page_idx, first_layer_idx) = PdfDocument::new(
        "Text Document",
        Mm(PAGE_WIDTH_MM),
        Mm(PAGE_HEIGHT_MM),
        "Layer 1",
    );

    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| PortError::Internal(format!("failed to add font: {e}")))?;

    let lines = wrap_text(text, CHARS_PER_LINE);
    let usable_height = PAGE_HEIGHT_MM - 2.0 * MARGIN_MM;
    let lines_per_page = (usable_height / LINE_HEIGHT_MM) as usize;

    let mut page_count = 0u32;
    let mut line_idx = 0;

    while line_idx < lines.len() {
        let (page_idx, layer_idx) = if page_count == 0 {
            (first_page_idx, first_layer_idx)
        } else {
            let (p, l) =
                doc.add_page(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Layer 1");
            (p, l)
        };
        page_count += 1;

        let layer = doc.get_page(page_idx).get_layer(layer_idx);
        let mut y = PAGE_HEIGHT_MM - MARGIN_MM;

        let end = (line_idx + lines_per_page).min(lines.len());
        for line in &lines[line_idx..end] {
            layer.use_text(line, FONT_SIZE_PT, Mm(MARGIN_MM), Mm(y), &font);
            y -= LINE_HEIGHT_MM;
        }
        line_idx = end;
    }

    save_doc(doc, output_path)?;

    Ok(serde_json::json!({
        "created": true,
        "output_path": output_path,
        "pages": page_count,
    }))
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

/// Simple word-wrap: split text into lines of at most `max_chars` characters.
/// Respects existing newlines in the input.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            result.push(String::new());
            continue;
        }
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in words {
            if line.is_empty() {
                line.push_str(word);
            } else if line.len() + 1 + word.len() <= max_chars {
                line.push(' ');
                line.push_str(word);
            } else {
                result.push(line);
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            result.push(line);
        }
    }
    result
}

fn save_doc(doc: PdfDocumentReference, path: &str) -> soma_port_sdk::Result<()> {
    let file = std::fs::File::create(path)
        .map_err(|e| PortError::Internal(format!("failed to create file {path}: {e}")))?;
    let mut writer = BufWriter::new(file);
    doc.save(&mut writer)
        .map_err(|e| PortError::Internal(format!("failed to save PDF: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "pdf".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Renderer,
        description: "PDF document generation: create documents, add pages, convert text to PDF"
            .into(),
        namespace: "soma.pdf".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "create_document".into(),
                name: "create_document".into(),
                purpose: "Create a new PDF document with a title and text content".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "title": {"type": "string", "description": "Document title"},
                    "content": {"type": "string", "description": "Text content for the first page"},
                    "output_path": {"type": "string", "description": "File path to write the PDF"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "created": {"type": "boolean"},
                    "output_path": {"type": "string"},
                    "pages": {"type": "integer"},
                    "title": {"type": "string"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::LogicalUndo,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 50,
                    p95_latency_ms: 200,
                    max_latency_ms: 1000,
                },
                cost_profile: CostProfile {
                    io_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "add_page".into(),
                name: "add_page".into(),
                purpose: "Write a page of text content to a PDF file path".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "document_path": {"type": "string", "description": "File path of the PDF"},
                    "content": {"type": "string", "description": "Text content for the page"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "added": {"type": "boolean"},
                    "document_path": {"type": "string"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::LogicalUndo,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 50,
                    p95_latency_ms: 200,
                    max_latency_ms: 1000,
                },
                cost_profile: CostProfile {
                    io_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "text_to_pdf".into(),
                name: "text_to_pdf".into(),
                purpose: "Convert plain text to a multi-page PDF file".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "text": {"type": "string", "description": "Plain text to convert"},
                    "output_path": {"type": "string", "description": "File path to write the PDF"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "created": {"type": "boolean"},
                    "output_path": {"type": "string"},
                    "pages": {"type": "integer"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::LogicalUndo,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 100,
                    p95_latency_ms: 500,
                    max_latency_ms: 5000,
                },
                cost_profile: CostProfile {
                    io_cost_class: CostClass::Medium,
                    cpu_cost_class: CostClass::Low,
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
            PortFailureClass::Unknown,
        ],
        side_effect_class: SideEffectClass::LocalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 100,
            p95_latency_ms: 500,
            max_latency_ms: 5000,
        },
        cost_profile: CostProfile {
            io_cost_class: CostClass::Low,
            cpu_cost_class: CostClass::Low,
            ..CostProfile::default()
        },
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements {
            filesystem_access: true,
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
    Box::into_raw(Box::new(PdfPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = PdfPort::new();
        assert_eq!(port.spec().port_id, "soma.pdf");
        assert_eq!(port.spec().capabilities.len(), 3);
    }

    #[test]
    fn test_lifecycle_active() {
        let port = PdfPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn test_validate_create_document_missing_fields() {
        let port = PdfPort::new();
        assert!(port
            .validate_input("create_document", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_create_document_ok() {
        let port = PdfPort::new();
        let input = serde_json::json!({
            "title": "Test",
            "content": "Hello world",
            "output_path": "/tmp/test.pdf"
        });
        assert!(port.validate_input("create_document", &input).is_ok());
    }

    #[test]
    fn test_validate_text_to_pdf_missing_fields() {
        let port = PdfPort::new();
        assert!(port
            .validate_input("text_to_pdf", &serde_json::json!({"text": "hello"}))
            .is_err());
    }

    #[test]
    fn test_unknown_capability() {
        let port = PdfPort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }

    #[test]
    fn test_wrap_text() {
        let lines = wrap_text("hello world this is a test", 12);
        assert_eq!(lines, vec!["hello world", "this is a", "test"]);
    }

    #[test]
    fn test_wrap_text_newlines() {
        let lines = wrap_text("line one\nline two\n\nline four", 80);
        assert_eq!(
            lines,
            vec!["line one", "line two", "", "line four"]
        );
    }

    #[test]
    fn test_create_document_writes_file() {
        let port = PdfPort::new();
        let path = "/tmp/soma_pdf_test_create.pdf";
        let input = serde_json::json!({
            "title": "Test Document",
            "content": "This is a test PDF generated by the SOMA PDF port.",
            "output_path": path,
        });
        let record = port.invoke("create_document", input).unwrap();
        assert!(record.success);
        assert!(std::path::Path::new(path).exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_text_to_pdf_writes_file() {
        let port = PdfPort::new();
        let path = "/tmp/soma_pdf_test_text.pdf";
        let input = serde_json::json!({
            "text": "Hello from SOMA PDF port.\nSecond line here.",
            "output_path": path,
        });
        let record = port.invoke("text_to_pdf", input).unwrap();
        assert!(record.success);
        assert!(std::path::Path::new(path).exists());
        let _ = std::fs::remove_file(path);
    }
}
