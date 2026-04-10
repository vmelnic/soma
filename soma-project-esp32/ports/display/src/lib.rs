// soma-esp32-port-display — text + pixel primitives for a monochrome
// OLED panel (SSD1306 in the reference firmware, but the port crate is
// driver-agnostic).
//
// Primitives:
//   display.info        {}                                 -> {width, height, driver, i2c_addr}
//   display.clear       {}                                 -> {cleared: true}
//   display.draw_text   {text, line?, column?, invert?}    -> {rendered: true}
//   display.draw_text_xy{text, x, y, invert?}              -> {rendered: true}
//   display.fill_rect   {x, y, width, height, on}          -> {filled: true}
//   display.set_contrast{value}                            -> {contrast: u8}
//   display.flush       {}                                 -> {flushed: true}
//
// The port takes six injected closures at construction time. Each
// closure captures the real Ssd1306 driver (plus whatever shared-bus
// wrapper the firmware uses) and performs the actual hardware work.
//
// Why closures instead of a trait + dyn object: the Ssd1306 driver's
// type depends on the I²C bus type, the DisplaySize const generic, and
// the DisplayMode. Keeping all that behind `Box<dyn FnMut>` erases the
// generics at the port boundary and keeps this crate free of the
// ssd1306 / embedded-graphics / embedded-hal-bus deps.
//
// Draw calls are expected to be synchronous and to flush immediately —
// the firmware-side closures call `display.flush()` at the end of each
// draw so a single skill invocation produces a visible update. That
// way a brain-side "every 5 seconds, draw temperature" loop needs one
// MCP call per tick, not two.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

/// Static information about the attached panel. Returned by
/// `display.info` so brains can size their drawing calls correctly.
#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub width: u16,
    pub height: u16,
    /// Driver identifier, e.g. "ssd1306".
    pub driver: &'static str,
    /// 7-bit I²C slave address of the panel, for diagnostic reporting.
    pub i2c_addr: u8,
}

/// Boxed closures the firmware injects at port construction.
///
/// These are `Box<dyn FnMut ...>` without a `Send` bound because the
/// shared-bus wiring in the firmware uses `&'static RefCell<...>` to
/// give multiple closures access to the same Ssd1306 driver — and
/// `RefCell` isn't `Sync`, so `&RefCell` isn't `Send`. That's fine:
/// the leaf's dispatch loop is single-threaded, and the leaf's
/// `Box<dyn SomaEspPort>` doesn't require `Send` either.
pub type InfoFn = Box<dyn FnMut() -> DisplayInfo>;
pub type ClearFn = Box<dyn FnMut() -> Result<(), String>>;
/// Draw text starting at (column, line) where line is a text row
/// (0..N where N = height/font_height) and column is the starting
/// character column. If `invert` is true, pixels are drawn with the
/// background on.
pub type DrawTextLineFn = Box<dyn FnMut(&str, u8, u8, bool) -> Result<(), String>>;
/// Draw text at absolute pixel coordinates. Same semantics as
/// `DrawTextLineFn` otherwise.
pub type DrawTextXyFn = Box<dyn FnMut(&str, u16, u16, bool) -> Result<(), String>>;
pub type FillRectFn = Box<dyn FnMut(u16, u16, u16, u16, bool) -> Result<(), String>>;
pub type SetContrastFn = Box<dyn FnMut(u8) -> Result<(), String>>;
pub type FlushFn = Box<dyn FnMut() -> Result<(), String>>;

pub struct DisplayPort {
    info_fn: InfoFn,
    clear_fn: ClearFn,
    draw_text_line_fn: DrawTextLineFn,
    draw_text_xy_fn: DrawTextXyFn,
    fill_rect_fn: FillRectFn,
    set_contrast_fn: SetContrastFn,
    flush_fn: FlushFn,
}

impl DisplayPort {
    pub fn new(
        info_fn: InfoFn,
        clear_fn: ClearFn,
        draw_text_line_fn: DrawTextLineFn,
        draw_text_xy_fn: DrawTextXyFn,
        fill_rect_fn: FillRectFn,
        set_contrast_fn: SetContrastFn,
        flush_fn: FlushFn,
    ) -> Self {
        Self {
            info_fn,
            clear_fn,
            draw_text_line_fn,
            draw_text_xy_fn,
            fill_rect_fn,
            set_contrast_fn,
            flush_fn,
        }
    }
}

impl SomaEspPort for DisplayPort {
    fn port_id(&self) -> &'static str {
        "display"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "display.info".to_string(),
                description: "Report the panel size, driver name, and I2C address".to_string(),
                input_schema: "{}".to_string(),
                output_schema:
                    r#"{"width":"u16","height":"u16","driver":"str","i2c_addr":"u8"}"#
                        .to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "display.clear".to_string(),
                description: "Clear the panel framebuffer and flush".to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"cleared":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "display.draw_text".to_string(),
                description:
                    "Draw text on a text row. Clears that row first so existing content on the same line is replaced. line is 0..N-1 where N = height/font_height (8 rows for a 128x64 panel with 8-pixel font). column is the starting character column (optional, defaults to 0). invert toggles foreground/background (optional, default false). Flushes to the panel."
                        .to_string(),
                input_schema:
                    r#"{"text":"str","line":"u8?","column":"u8?","invert":"bool?"}"#
                        .to_string(),
                output_schema: r#"{"rendered":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "display.draw_text_xy".to_string(),
                description:
                    "Draw text at absolute pixel coordinates (x, y). Does NOT clear existing pixels — use display.clear or display.fill_rect first if you need a clean region. Flushes to the panel."
                        .to_string(),
                input_schema:
                    r#"{"text":"str","x":"u16","y":"u16","invert":"bool?"}"#.to_string(),
                output_schema: r#"{"rendered":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "display.fill_rect".to_string(),
                description:
                    "Fill a rectangle at (x, y) of (width, height) pixels. on=true lights pixels; on=false clears them. Flushes to the panel."
                        .to_string(),
                input_schema:
                    r#"{"x":"u16","y":"u16","width":"u16","height":"u16","on":"bool"}"#
                        .to_string(),
                output_schema: r#"{"filled":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "display.set_contrast".to_string(),
                description: "Set the panel contrast (0-255). 0 is dimmest, 255 is brightest."
                    .to_string(),
                input_schema: r#"{"value":"u8"}"#.to_string(),
                output_schema: r#"{"contrast":"u8"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "display.flush".to_string(),
                description:
                    "Force a framebuffer flush to the panel. Usually unnecessary because draw_text and fill_rect flush implicitly — only useful if a brain-composed routine needs to batch multiple writes."
                        .to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"flushed":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "display.info" => {
                let info = (self.info_fn)();
                Ok(json!({
                    "width": info.width,
                    "height": info.height,
                    "driver": info.driver,
                    "i2c_addr": info.i2c_addr,
                }))
            }
            "display.clear" => {
                (self.clear_fn)()?;
                Ok(json!({ "cleared": true }))
            }
            "display.draw_text" => {
                let text = input["text"]
                    .as_str()
                    .ok_or_else(|| "missing 'text'".to_string())?;
                let line = input["line"].as_u64().unwrap_or(0) as u8;
                let column = input["column"].as_u64().unwrap_or(0) as u8;
                let invert = input["invert"].as_bool().unwrap_or(false);
                (self.draw_text_line_fn)(text, line, column, invert)?;
                Ok(json!({ "rendered": true }))
            }
            "display.draw_text_xy" => {
                let text = input["text"]
                    .as_str()
                    .ok_or_else(|| "missing 'text'".to_string())?;
                let x = input["x"]
                    .as_u64()
                    .ok_or_else(|| "missing 'x'".to_string())?
                    as u16;
                let y = input["y"]
                    .as_u64()
                    .ok_or_else(|| "missing 'y'".to_string())?
                    as u16;
                let invert = input["invert"].as_bool().unwrap_or(false);
                (self.draw_text_xy_fn)(text, x, y, invert)?;
                Ok(json!({ "rendered": true }))
            }
            "display.fill_rect" => {
                let x = input["x"]
                    .as_u64()
                    .ok_or_else(|| "missing 'x'".to_string())?
                    as u16;
                let y = input["y"]
                    .as_u64()
                    .ok_or_else(|| "missing 'y'".to_string())?
                    as u16;
                let width = input["width"]
                    .as_u64()
                    .ok_or_else(|| "missing 'width'".to_string())?
                    as u16;
                let height = input["height"]
                    .as_u64()
                    .ok_or_else(|| "missing 'height'".to_string())?
                    as u16;
                let on = input["on"]
                    .as_bool()
                    .ok_or_else(|| "missing 'on' (bool)".to_string())?;
                (self.fill_rect_fn)(x, y, width, height, on)?;
                Ok(json!({ "filled": true }))
            }
            "display.set_contrast" => {
                let value = input["value"]
                    .as_u64()
                    .ok_or_else(|| "missing 'value'".to_string())?;
                if value > 255 {
                    return Err(alloc::format!("contrast out of range: {}", value));
                }
                let v = value as u8;
                (self.set_contrast_fn)(v)?;
                Ok(json!({ "contrast": v }))
            }
            "display.flush" => {
                (self.flush_fn)()?;
                Ok(json!({ "flushed": true }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
