//! Browser / WebAssembly entry point for soma-next.
//!
//! This module compiles only on `target_arch = "wasm32"`. It exposes
//! JavaScript-visible functions through wasm-bindgen, installs a panic
//! hook that forwards Rust panics to the browser console, and provides
//! the in-tab port catalog (DOM today; audio/voice/keyboard in
//! subsequent phase-1 steps).
//!
//! The mental model: soma-next is the brain/body bridge running inside
//! the browser tab. JavaScript is the shell that loads the wasm, wires
//! user events to `invoke_*` entry points, and receives effects through
//! ports that call back out into `web_sys` / `js_sys`.

use wasm_bindgen::prelude::*;

pub mod dom_port;

use dom_port::DomPort;

use crate::runtime::port::Port;
use crate::types::port::InvocationContext;

/// Called from JavaScript exactly once, before any other soma-next function.
/// Installs the panic hook so Rust panics end up as `console.error` entries
/// with full stack traces, and logs a boot banner so the page can confirm
/// the wasm module actually loaded.
#[wasm_bindgen(start)]
pub fn soma_start() {
    console_error_panic_hook::set_once();
    web_sys::console::log_1(&JsValue::from_str(
        "[soma-next wasm] boot — soma body loaded in the browser",
    ));
}

/// Minimal phase-1a proof entry point. Creates an in-tab DomPort, invokes
/// its `append_heading` capability with the supplied text, and returns a
/// JSON string of the resulting PortCallRecord so the JS side can see
/// exactly what the runtime produced.
///
/// This is NOT the permanent API. It exists so the very first browser
/// proof has something short and concrete to call. Once the pack-manifest
/// bootstrap lands, this goes away in favor of `invoke_port(port_id,
/// capability_id, input_json)`.
#[wasm_bindgen]
pub fn soma_demo_render_heading(text: &str) -> Result<JsValue, JsValue> {
    let port = DomPort::new();
    let input = serde_json::json!({ "text": text });

    // Validate first (mirrors the native DefaultPortRuntime pipeline).
    port.validate_input("append_heading", &input)
        .map_err(|e| JsValue::from_str(&format!("validate_input: {e}")))?;

    let _ctx = InvocationContext::local();
    let record = port
        .invoke("append_heading", input)
        .map_err(|e| JsValue::from_str(&format!("invoke: {e}")))?;

    let json = serde_json::to_string(&record)
        .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))?;
    Ok(JsValue::from_str(&json))
}
