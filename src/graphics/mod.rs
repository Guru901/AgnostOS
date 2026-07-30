use noto_sans_mono_bitmap::{RasterHeight, RasterizedChar, get_raster};
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

use crate::{FONT_HEIGHT, FONT_WEIGHT, color::Color};

#[derive(Debug, Clone)]
pub struct Framebuffer {
    pub ptr: *mut u8,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub is_bgr: bool, // true if pixel format is BGR, false if RGB
}

impl Framebuffer {
    pub fn new(gop: &mut GraphicsOutput) -> Self {
        let mode_info = gop.current_mode_info();
        let (width, height) = mode_info.resolution();
        let stride = mode_info.stride();
        let is_bgr = matches!(mode_info.pixel_format(), PixelFormat::Bgr);
        let ptr = gop.frame_buffer().as_mut_ptr();

        Self {
            ptr,
            width,
            height,
            stride,
            is_bgr,
        }
    }
}

impl Framebuffer {
    #[inline]
    unsafe fn write_pixel(&self, pixel_index: usize, color: &Color) {
        let rgb = if self.is_bgr {
            [color.b, color.g, color.r]
        } else {
            [color.r, color.g, color.b]
        };
        // SAFETY: callers calculate `pixel_index` from coordinates within the
        // framebuffer dimensions; the GOP framebuffer has four bytes per pixel.
        let p = unsafe { self.ptr.add(4 * pixel_index) };
        // SAFETY: `p` points to the four-byte pixel selected above, so its first
        // three bytes can be written with the selected channel ordering.
        unsafe {
            p.write(rgb[0]);
            p.add(1).write(rgb[1]);
            p.add(2).write(rgb[2]);
        }
    }
}

/// Clears the background with the given color
///
/// **Example**
///
/// ```rust
/// use agnostos::Color;
///
/// agnostos::graphics::clear_background(&fb, Color { r: 255, g: 255, b: 255 });
/// ```
pub fn clear_background(fb: &Framebuffer, color: &Color) {
    for row in 0..fb.height {
        for col in 0..fb.width {
            let pixel_index = row * fb.stride + col;
            // SAFETY: `row` and `col` are bounded by the framebuffer dimensions.
            unsafe { fb.write_pixel(pixel_index, &color) };
        }
    }
}

/// Renders a rectangle on the screen, at the provided coordinates with the provided color and
/// dimensions.
///
/// **Example**
///
/// ```rust
/// use agnostos::Color;
///
/// agnostos::graphics::draw_rec(&fb, (100, 100), (100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_rec(fb: &Framebuffer, (x, y): (usize, usize), (w, h): (usize, usize), color: Color) {
    let x2 = x + w;
    let y2 = y + h;
    assert!(x2 <= fb.width, "Bad X coordinate");
    assert!(y2 <= fb.height, "Bad Y coordinate");

    for row in y..y2 {
        for col in x..x2 {
            let pixel_index = row * fb.stride + col;
            // SAFETY: the coordinate assertions above keep this pixel in bounds.
            unsafe { fb.write_pixel(pixel_index, &color) };
        }
    }
}

/// Renders a circle on the screen, at the provided coordinates with the provided color and radius.
///
/// **Example**
///
/// ```rust
/// use agnostos::Color;
///
/// agnostos::graphics::draw_circle(&fb, 20, (100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_circle(fb: &Framebuffer, radius: usize, (cx, cy): (usize, usize), color: Color) {
    let r = radius as isize;
    let cx = cx as isize;
    let cy = cy as isize;
    let r_sq = r * r;

    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r_sq {
                let px = cx + dx;
                let py = cy + dy;

                if px >= 0 && py >= 0 && px < fb.width as isize && py < fb.height as isize {
                    let pixel_index = py as usize * fb.stride + px as usize;
                    // SAFETY: the preceding bounds check keeps `(px, py)` in the framebuffer.
                    unsafe {
                        fb.write_pixel(pixel_index, &color);
                    }
                }
            }
        }
    }
}

/// Renders a line on the screen, at the provided coordinates with the provided color.
///
/// **Example**
///
/// ```rust
/// use agnostos::Color;
///
/// agnostos::graphics::draw_line(&fb, (100, 100), (100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_line(fb: &Framebuffer, (x1, y1): (i64, i64), (x2, y2): (i64, i64), color: Color) {
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    let sx = if x2 >= x1 { 1 } else { -1 };
    let sy = if y2 >= y1 { 1 } else { -1 };
    let mut err = dx - dy;

    let (mut x, mut y) = (x1, y1);

    loop {
        if x >= 0 && y >= 0 && x < fb.width as i64 && y < fb.height as i64 {
            let pixel_index = y as usize * fb.stride + x as usize;
            // SAFETY: the bounds check above keeps `(x, y)` in the framebuffer.
            unsafe {
                fb.write_pixel(pixel_index, &color);
            }
        }

        if x == x2 && y == y2 {
            break;
        }

        let e2 = 2 * err;

        if e2 > -dy {
            err -= dy;
            x += sx;
        }

        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
}

/// Scrolls the framebuffer content up by `rows` pixel rows, clearing the freed strip at the bottom.
pub fn scroll_up(fb: &Framebuffer, rows: usize) {
    let bpp = 4usize; // bytes per pixel
    for row in rows..fb.height {
        for col in 0..fb.width {
            let src = (row * fb.stride + col) * bpp;
            let dst = ((row - rows) * fb.stride + col) * bpp;
            // SAFETY: source and destination offsets are derived from valid rows
            // and columns, and each pixel occupies `bpp` bytes in the framebuffer.
            unsafe {
                for b in 0..bpp {
                    let val = fb.ptr.add(src + b).read();
                    fb.ptr.add(dst + b).write(val);
                }
            }
        }
    }
    // clear the freed bottom strip
    for row in (fb.height - rows)..fb.height {
        for col in 0..fb.width {
            let idx = (row * fb.stride + col) * bpp;
            // SAFETY: these offsets address the valid bottom rows being cleared.
            unsafe {
                fb.ptr.add(idx).write(0);
                fb.ptr.add(idx + 1).write(0);
                fb.ptr.add(idx + 2).write(0);
                fb.ptr.add(idx + 3).write(0);
            }
        }
    }
}

/// Renders the provided text on the screen, at the provided coordinates with the provided color and font.
///
/// **Example**
///
/// ```rust
/// use agnostos::Color;
///
/// agnostos::graphics::draw_text(&fb, "Random text to render", (100, 200), Color { r: 0, g: 0, b: 0 }, None);
/// ```
pub fn draw_text(
    fb: &Framebuffer,
    text: &str,
    (x, y): (usize, usize),
    color: Color,
    font_height: Option<RasterHeight>,
) {
    let mut cursor_x = x;

    for ch in text.chars() {
        if ch == '\n' {
            break;
        }

        let font_height = font_height.unwrap_or(FONT_HEIGHT);
        let char_raster = match get_raster(ch, FONT_WEIGHT, font_height) {
            Some(r) => r,
            None => match get_raster('?', FONT_WEIGHT, FONT_HEIGHT) {
                Some(r) => r,
                None => continue,
            },
        };

        draw_glyph(fb, &char_raster, cursor_x, y, &color);
        cursor_x += char_raster.width();
    }
}

fn draw_glyph(fb: &Framebuffer, raster: &RasterizedChar, x: usize, y: usize, color: &Color) {
    for (row, row_data) in raster.raster().iter().enumerate() {
        for (col, &intensity) in row_data.iter().enumerate() {
            if intensity == 0 {
                continue; // fully transparent, skip
            }

            let px = x + col;
            let py = y + row;

            if px >= fb.width || py >= fb.height {
                continue;
            }

            // blend intensity with color
            let r = (color.r as u32 * intensity as u32 / 255) as u8;
            let g = (color.g as u32 * intensity as u32 / 255) as u8;
            let b = (color.b as u32 * intensity as u32 / 255) as u8;

            let pixel_index = py * fb.stride + px;
            // SAFETY: out-of-bounds glyph pixels are skipped immediately above.
            unsafe { fb.write_pixel(pixel_index, &Color::new(r, g, b)) };
        }
    }
}
