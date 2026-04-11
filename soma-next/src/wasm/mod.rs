//! Browser / WebAssembly entry point for soma-next.
//!
//! This module compiles only on `target_arch = "wasm32"`. It exposes
//! JavaScript-visible functions through wasm-bindgen, installs a panic
//! hook that forwards Rust panics to the browser console, registers the
//! built-in in-tab ports (DOM + audio in phase 1b, voice + keyboard
//! coming in phase 1c), and dispatches JS-initiated `invoke_port` calls
//! through the same `Port::invoke` path that every other SOMA body uses.
//!
//! The mental model: soma-next is the brain/body bridge running inside
//! the browser tab. JavaScript is the shell that loads the wasm, wires
//! user events to `soma_invoke_port`, and receives effects through ports
//! that call back out into `web_sys` / `js_sys`.

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::prelude::*;

pub mod audio_port;
pub mod dom_port;
pub mod voice_port;

use audio_port::AudioPort;
use dom_port::DomPort;
use voice_port::VoicePort;

use crate::runtime::port::Port;

// In-tab port registry. Built lazily on first `soma_invoke_port` call.
//
// `thread_local!` + `RefCell` is the idiomatic single-threaded wasm
// pattern: wasm32-unknown-unknown has no real threads, the runtime lives
// entirely inside the main JavaScript event loop, and `RefCell` gives
// interior mutability without the overhead of `Mutex`.
thread_local! {
    static PORTS: RefCell<Option<HashMap<String, Box<dyn Port>>>> = const { RefCell::new(None) };
}

fn ensure_ports_initialized() {
    PORTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let mut map: HashMap<String, Box<dyn Port>> = HashMap::new();
            map.insert("dom".to_string(), Box::new(DomPort::new()));
            map.insert("audio".to_string(), Box::new(AudioPort::new()));
            map.insert("voice".to_string(), Box::new(VoicePort::new()));
            *slot = Some(map);
            web_sys::console::log_1(&JsValue::from_str(
                "[soma-next wasm] port registry initialized: dom, audio, voice",
            ));
        }
    });
}

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

/// Return a JSON array of all registered port IDs. Useful for debugging
/// from the browser dev console.
#[wasm_bindgen]
pub fn soma_list_ports() -> String {
    ensure_ports_initialized();
    PORTS.with(|cell| {
        let slot = cell.borrow();
        let map = slot.as_ref().expect("ports initialized above");
        let ids: Vec<&String> = map.keys().collect();
        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string())
    })
}

/// Invoke a capability on one of the in-tab ports.
///
/// Parameters:
///   * `port_id` — `"dom"` or `"audio"` in phase 1b
///   * `capability_id` — e.g. `"append_heading"`, `"say_text"`
///   * `input_json` — JSON object as a string, matching the capability's
///     declared `input_schema`
///
/// Returns the resulting `PortCallRecord` serialized as a JSON string.
/// Throws a JS `Error` containing the failure message when port lookup,
/// validation, or invocation itself fail at the Rust level. Capability
/// errors (`success: false`) are returned as a populated record, not
/// a thrown error — same as every other SOMA call path.
#[wasm_bindgen]
pub fn soma_invoke_port(
    port_id: &str,
    capability_id: &str,
    input_json: &str,
) -> Result<String, JsValue> {
    ensure_ports_initialized();

    let input: serde_json::Value = serde_json::from_str(input_json).map_err(|e| {
        JsValue::from_str(&format!("input_json is not valid JSON: {e}"))
    })?;

    PORTS.with(|cell| {
        let slot = cell.borrow();
        let map = slot.as_ref().expect("ports initialized above");
        let port = map
            .get(port_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown port '{port_id}'")))?;

        port.validate_input(capability_id, &input)
            .map_err(|e| JsValue::from_str(&format!("validate_input: {e}")))?;

        let record = port
            .invoke(capability_id, input)
            .map_err(|e| JsValue::from_str(&format!("invoke: {e}")))?;

        serde_json::to_string(&record)
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })
}

/// Phase 1a compatibility shim. The new generic path is
/// `soma_invoke_port("dom", "append_heading", "{\"text\":\"...\"}")` —
/// this function wraps that so the original proof harness keeps working.
#[wasm_bindgen]
pub fn soma_demo_render_heading(text: &str) -> Result<JsValue, JsValue> {
    let input = serde_json::json!({ "text": text }).to_string();
    let json = soma_invoke_port("dom", "append_heading", &input)?;
    Ok(JsValue::from_str(&json))
}
