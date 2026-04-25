use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Frame, ImageBuffer, Rgba, RgbaImage};

const CELL_PX: u32 = 48;
const BORDER_PX: u32 = 2;

pub struct GridFrame {
    pub cells: serde_json::Value,
    pub label: String,
}

fn cell_color(ch: &str) -> [u8; 4] {
    match ch {
        "W" => [100, 100, 100, 255],
        "K" => [255, 215, 0, 255],
        "D" => [180, 40, 40, 255],
        "O" => [60, 180, 60, 255],
        "G" => [40, 200, 40, 255],
        "A" => [30, 120, 255, 255],
        "L" => [255, 100, 20, 255],
        "B" => [160, 80, 200, 255],
        "X" => [140, 100, 60, 255],
        _   => [230, 230, 230, 255],
    }
}

fn render_frame(frame: &GridFrame) -> Option<RgbaImage> {
    let rows = frame.cells.as_array()?;
    let h = rows.len() as u32;
    let w = rows.first()?.as_array()?.len() as u32;
    let label_h = 24u32;
    let img_w = w * CELL_PX;
    let img_h = h * CELL_PX + label_h;

    let mut img: RgbaImage = ImageBuffer::from_pixel(img_w, img_h, Rgba([20, 20, 30, 255]));

    for py in 0..label_h {
        for px in 0..img_w {
            img.put_pixel(px, py, Rgba([30, 30, 45, 255]));
        }
    }
    draw_text_5x7(&mut img, 4, 8, &frame.label, Rgba([200, 200, 220, 255]));

    for (gy, row) in rows.iter().enumerate() {
        let cols = row.as_array()?;
        for (gx, cell) in cols.iter().enumerate() {
            let ch = cell.as_str().unwrap_or(".");
            let color = cell_color(ch);
            let x0 = gx as u32 * CELL_PX;
            let y0 = gy as u32 * CELL_PX + label_h;

            for py in 0..CELL_PX {
                for px in 0..CELL_PX {
                    let on_border = px < BORDER_PX || py < BORDER_PX
                        || px >= CELL_PX - BORDER_PX || py >= CELL_PX - BORDER_PX;
                    let c = if on_border { [50, 50, 60, 255] } else { color };
                    img.put_pixel(x0 + px, y0 + py, Rgba(c));
                }
            }

            if ch == "A" { draw_triangle(&mut img, x0, y0, CELL_PX); }
            if ch == "K" { draw_key_icon(&mut img, x0, y0, CELL_PX); }
            if ch == "G" { draw_star(&mut img, x0, y0, CELL_PX); }
            if ch == "B" { draw_circle(&mut img, x0, y0, CELL_PX, Rgba([220, 180, 255, 255])); }
            if ch == "X" { draw_box_icon(&mut img, x0, y0, CELL_PX); }
            if ch == "L" { draw_lava(&mut img, x0, y0, CELL_PX); }
            if ch == "D" { draw_door_icon(&mut img, x0, y0, CELL_PX, true); }
            if ch == "O" { draw_door_icon(&mut img, x0, y0, CELL_PX, false); }
        }
    }

    Some(img)
}

fn draw_triangle(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let cx = size / 2;
    let margin = size / 4;
    let color = Rgba([255, 60, 60, 255]);
    for py in margin..size - margin {
        let progress = (py - margin) as f64 / (size - 2 * margin) as f64;
        let half_w = (progress * (size / 2 - margin) as f64) as u32;
        let left = cx.saturating_sub(half_w);
        let right = (cx + half_w).min(size - 1);
        for px in left..=right {
            img.put_pixel(x0 + px, y0 + py, color);
        }
    }
}

fn draw_key_icon(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let color = Rgba([120, 80, 0, 255]);
    let cx = size / 2;
    let cy = size / 3;
    let r = size / 6;
    for py in 0..size {
        for px in 0..size {
            let dx = px as i32 - cx as i32;
            let dy = py as i32 - cy as i32;
            let dist = ((dx * dx + dy * dy) as f64).sqrt();
            if dist <= r as f64 && dist >= (r as f64 - 3.0) {
                img.put_pixel(x0 + px, y0 + py, color);
            }
        }
    }
    let shaft_top = cy + r;
    let shaft_bottom = size - size / 5;
    for py in shaft_top..shaft_bottom {
        img.put_pixel(x0 + cx, y0 + py, color);
        img.put_pixel(x0 + cx + 1, y0 + py, color);
    }
    for offset in [0, size / 8] {
        let ty = shaft_bottom - 2 - offset;
        for px in cx + 2..cx + size / 6 {
            img.put_pixel(x0 + px, y0 + ty, color);
        }
    }
}

fn draw_star(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let color = Rgba([255, 255, 255, 255]);
    let cx = size as f64 / 2.0;
    let cy = size as f64 / 2.0;
    let r_outer = size as f64 * 0.35;
    let r_inner = r_outer * 0.4;
    let points = 5;

    let mut vertices = Vec::new();
    for i in 0..points * 2 {
        let angle = std::f64::consts::FRAC_PI_2 * -1.0
            + std::f64::consts::PI * i as f64 / points as f64;
        let r = if i % 2 == 0 { r_outer } else { r_inner };
        vertices.push((cx + r * angle.cos(), cy + r * angle.sin()));
    }

    for py in BORDER_PX..size - BORDER_PX {
        for px in BORDER_PX..size - BORDER_PX {
            if point_in_polygon(px as f64, py as f64, &vertices) {
                img.put_pixel(x0 + px, y0 + py, color);
            }
        }
    }
}

fn draw_circle(img: &mut RgbaImage, x0: u32, y0: u32, size: u32, color: Rgba<u8>) {
    let cx = size as f64 / 2.0;
    let cy = size as f64 / 2.0;
    let r = size as f64 * 0.3;
    for py in BORDER_PX..size - BORDER_PX {
        for px in BORDER_PX..size - BORDER_PX {
            let dx = px as f64 - cx;
            let dy = py as f64 - cy;
            if dx * dx + dy * dy <= r * r {
                img.put_pixel(x0 + px, y0 + py, color);
            }
        }
    }
}

fn draw_box_icon(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let color = Rgba([200, 160, 80, 255]);
    let m = size / 4;
    for py in m..size - m {
        for px in m..size - m {
            let on_edge = px == m || px == size - m - 1 || py == m || py == size - m - 1;
            if on_edge {
                img.put_pixel(x0 + px, y0 + py, color);
            }
        }
    }
    img.put_pixel(x0 + size / 2, y0 + size / 2, color);
}

fn draw_lava(img: &mut RgbaImage, x0: u32, y0: u32, size: u32) {
    let color = Rgba([255, 200, 0, 255]);
    let cx = size / 2;
    for px in (cx - 3)..=(cx + 3) {
        for py in (size / 3)..(size * 2 / 3) {
            img.put_pixel(x0 + px, y0 + py, color);
        }
    }
    for px in (cx - 1)..=(cx + 1) {
        img.put_pixel(x0 + px, y0 + size / 4, color);
    }
}

fn draw_door_icon(img: &mut RgbaImage, x0: u32, y0: u32, size: u32, locked: bool) {
    let color = if locked { Rgba([255, 80, 80, 255]) } else { Rgba([80, 255, 80, 255]) };
    let m = size / 4;
    for py in m..size - m {
        for px in m + 2..size - m - 2 {
            let on_edge = px <= m + 3 || px >= size - m - 4 || py == m || py == size - m - 1;
            if on_edge { img.put_pixel(x0 + px, y0 + py, color); }
        }
    }
    let knob_x = x0 + size * 3 / 4 - 2;
    let knob_y = y0 + size / 2;
    for dy in 0..3u32 {
        for dx in 0..3u32 {
            img.put_pixel(knob_x + dx, knob_y + dy, color);
        }
    }
}

fn point_in_polygon(x: f64, y: f64, verts: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = verts[i];
        let (xj, yj) = verts[j];
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn draw_text_5x7(img: &mut RgbaImage, x0: u32, y0: u32, text: &str, color: Rgba<u8>) {
    #[rustfmt::skip]
    const FONT: &[(char, [u8; 7])] = &[
        ('.', [0b00000,0b00000,0b00000,0b00000,0b00000,0b01100,0b01100]),
        ('_', [0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b11111]),
        ('a', [0b00000,0b00000,0b01110,0b00001,0b01111,0b10001,0b01111]),
        ('c', [0b00000,0b00000,0b01110,0b10000,0b10000,0b10001,0b01110]),
        ('d', [0b00001,0b00001,0b01101,0b10011,0b10001,0b10001,0b01111]),
        ('e', [0b00000,0b00000,0b01110,0b10001,0b11111,0b10000,0b01110]),
        ('g', [0b01111,0b10001,0b10001,0b01111,0b00001,0b10001,0b01110]),
        ('i', [0b00100,0b00000,0b01100,0b00100,0b00100,0b00100,0b01110]),
        ('k', [0b10000,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001]),
        ('l', [0b01100,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110]),
        ('n', [0b00000,0b00000,0b10110,0b11001,0b10001,0b10001,0b10001]),
        ('o', [0b00000,0b00000,0b01110,0b10001,0b10001,0b10001,0b01110]),
        ('p', [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000]),
        ('r', [0b00000,0b00000,0b10110,0b11001,0b10000,0b10000,0b10000]),
        ('s', [0b00000,0b00000,0b01110,0b10000,0b01110,0b00001,0b11110]),
        ('t', [0b00100,0b00100,0b01110,0b00100,0b00100,0b00100,0b00011]),
        ('u', [0b00000,0b00000,0b10001,0b10001,0b10001,0b10011,0b01101]),
        ('y', [0b00000,0b00000,0b10001,0b10001,0b01111,0b00001,0b01110]),
    ];

    let mut cx = x0;
    for ch in text.chars() {
        if let Some((_, rows)) = FONT.iter().find(|(c, _)| *c == ch) {
            for (row_i, bits) in rows.iter().enumerate() {
                for col in 0..5u32 {
                    if bits & (1 << (4 - col)) != 0 {
                        let px = cx + col;
                        let py = y0 + row_i as u32;
                        if px < img.width() && py < img.height() {
                            img.put_pixel(px, py, color);
                        }
                    }
                }
            }
        }
        cx += 6;
    }
}

pub fn write_gif(frames: &[GridFrame], path: &Path) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let writer = BufWriter::new(file);
    let mut encoder = GifEncoder::new_with_speed(writer, 10);
    encoder.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;

    for gf in frames {
        let img = render_frame(gf).ok_or("failed to render frame")?;
        let frame = Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(800, 1));
        encoder.encode_frame(frame).map_err(|e| e.to_string())?;
    }
    Ok(())
}
