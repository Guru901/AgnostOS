use noto_sans_mono_bitmap::{RasterHeight, RasterizedChar, get_raster};

use crate::{FONT_HEIGHT, FONT_WEIGHT, color::Color};

use super::{Framebuffer, PixelCoord};

pub fn draw_text(
    fb: &Framebuffer,
    text: &str,
    origin: PixelCoord,
    color: Color,
    font_height: Option<RasterHeight>,
) {
    if !fb.is_drawable() {
        return;
    }
    let mut cursor_x = origin.x();
    let font_height = font_height.unwrap_or(FONT_HEIGHT);
    for ch in text.chars() {
        if ch == '\n' {
            break;
        }
        let Some(raster) = get_raster(ch, FONT_WEIGHT, font_height)
            .or_else(|| get_raster('?', FONT_WEIGHT, FONT_HEIGHT))
        else {
            continue;
        };
        draw_glyph(fb, &raster, cursor_x, origin.y(), color);
        let Some(next_x) = cursor_x.checked_add(raster.width()) else {
            return;
        };
        cursor_x = next_x;
    }
}

fn draw_glyph(
    fb: &Framebuffer,
    raster: &RasterizedChar,
    origin_x: usize,
    origin_y: usize,
    color: Color,
) {
    for (row, row_data) in raster.raster().iter().enumerate() {
        for (col, &intensity) in row_data.iter().enumerate() {
            if intensity == 0 {
                continue;
            }
            let (Some(x), Some(y)) = (origin_x.checked_add(col), origin_y.checked_add(row)) else {
                continue;
            };
            if x >= fb.width() || y >= fb.height() {
                continue;
            }
            let color = Color::new(
                blend(color.r, intensity),
                blend(color.g, intensity),
                blend(color.b, intensity),
            );
            fb.write_pixel(y * fb.stride() + x, &color);
        }
    }
}

fn blend(channel: u8, intensity: u8) -> u8 {
    (u16::from(channel) * u16::from(intensity) / 255) as u8
}
