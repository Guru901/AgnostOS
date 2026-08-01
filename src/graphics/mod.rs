use alloc::boxed::Box;
use alloc::vec;

use noto_sans_mono_bitmap::{RasterHeight, RasterizedChar, get_raster};
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

use crate::{FONT_HEIGHT, FONT_WEIGHT, color::Color};

#[derive(Debug, Clone)]
pub struct Framebuffer {
    pub ptr: *mut u8,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    /// Pixel layout selected by GOP. Direct rendering supports only `Rgb` and
    /// `Bgr`, both of which are 32-bit formats.
    pub pixel_format: PixelFormat,
    /// Number of bytes reported by GOP for the framebuffer mapping.
    pub byte_len: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FramebufferError {
    UnsupportedPixelFormat(PixelFormat),
    InvalidGeometry,
    FramebufferTooSmall { required: usize, actual: usize },
}

impl Framebuffer {
    /// Creates a framebuffer wrapper around the current GOP mode.
    ///
    /// # Errors
    ///
    /// Returns an error when the GOP pixel format is unsupported, its geometry
    /// overflows, or the backing framebuffer is smaller than its geometry requires.
    pub fn new(gop: &mut GraphicsOutput) -> Result<Self, FramebufferError> {
        let mode_info = gop.current_mode_info();
        let (width, height) = mode_info.resolution();
        let stride = mode_info.stride();
        let pixel_format = mode_info.pixel_format();

        if !matches!(pixel_format, PixelFormat::Rgb | PixelFormat::Bgr) {
            return Err(FramebufferError::UnsupportedPixelFormat(pixel_format));
        }

        let required = Self::required_byte_len(width, height, stride)
            .ok_or(FramebufferError::InvalidGeometry)?;
        let mut framebuffer = gop.frame_buffer();
        let byte_len = framebuffer.size();
        if byte_len < required {
            return Err(FramebufferError::FramebufferTooSmall {
                required,
                actual: byte_len,
            });
        }

        Ok(Self {
            ptr: framebuffer.as_mut_ptr(),
            width,
            height,
            stride,
            pixel_format,
            byte_len,
        })
    }

    #[doc(hidden)]
    #[must_use]
    pub fn for_doc_test() -> Self {
        const WIDTH: usize = 300;
        const HEIGHT: usize = 300;
        // The examples render through a raw framebuffer pointer, so retain a
        // heap allocation for the duration of the doctest rather than using a
        // null pointer.  It is intentionally leaked: `Framebuffer` does not
        // own pointers returned by `new`, and therefore cannot safely free it.
        let pixels = Box::leak(vec![0_u8; WIDTH * HEIGHT * 4].into_boxed_slice());

        Self {
            ptr: pixels.as_mut_ptr(),
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH,
            pixel_format: PixelFormat::Bgr,
            byte_len: WIDTH * HEIGHT * 4,
        }
    }

    fn required_byte_len(width: usize, height: usize, stride: usize) -> Option<usize> {
        if width > stride {
            return None;
        }

        stride.checked_mul(height)?.checked_mul(4)
    }

    /// Returns whether this framebuffer is suitable for direct 32-bit drawing.
    /// Drawing functions treat an invalid framebuffer as a no-op.
    #[must_use]
    pub fn is_drawable(&self) -> bool {
        !self.ptr.is_null()
            && matches!(self.pixel_format, PixelFormat::Rgb | PixelFormat::Bgr)
            && Self::required_byte_len(self.width, self.height, self.stride)
                .is_some_and(|required| required <= self.byte_len)
    }

    #[inline]
    unsafe fn write_pixel(&self, pixel_index: usize, color: &Color) -> bool {
        let rgb = match self.pixel_format {
            PixelFormat::Bgr => [color.b, color.g, color.r],
            PixelFormat::Rgb => [color.r, color.g, color.b],
            _ => return false,
        };
        let Some(offset) = pixel_index.checked_mul(4) else {
            return false;
        };
        if offset.checked_add(4).is_none_or(|end| end > self.byte_len) {
            return false;
        }
        // SAFETY: the checked offset identifies a complete 32-bit pixel within
        // the framebuffer byte range.
        let p = unsafe { self.ptr.add(offset) };
        // SAFETY: `p` points to the four-byte pixel selected above, so its first
        // three bytes can be written with the selected channel ordering.
        unsafe {
            p.write_volatile(rgb[0]);
            p.add(1).write_volatile(rgb[1]);
            p.add(2).write_volatile(rgb[2]);
        }
        true
    }
}

/// Clears the background with the given color
///
/// **Example**
///
/// ```rust
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// # let fb = Framebuffer::for_doc_test();
/// agnostos::graphics::clear_background(&fb, &Color { r: 255, g: 255, b: 255 });
/// ```
pub fn clear_background(fb: &Framebuffer, color: &Color) {
    if !fb.is_drawable() {
        return;
    }

    for row in 0..fb.height {
        for col in 0..fb.width {
            let pixel_index = row * fb.stride + col;
            // SAFETY: `row` and `col` are bounded by the framebuffer dimensions.
            unsafe { fb.write_pixel(pixel_index, color) };
        }
    }
}

/// Renders a rectangle on the screen, at the provided coordinates with the provided color and
/// dimensions.
///
/// # Panics
///
/// Panics when the rectangle extends past the framebuffer's right or bottom edge.
///
/// **Example**
///
/// ```rust
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// # let fb = Framebuffer::for_doc_test();
/// agnostos::graphics::draw_rec(&fb, (100, 100), (100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_rec(fb: &Framebuffer, (x, y): (usize, usize), (w, h): (usize, usize), color: Color) {
    if !fb.is_drawable() {
        return;
    }
    let Some(x2) = x.checked_add(w) else {
        return;
    };
    let Some(y2) = y.checked_add(h) else {
        return;
    };
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
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// # let fb = Framebuffer::for_doc_test();
/// agnostos::graphics::draw_circle(&fb, 20, (100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_circle(fb: &Framebuffer, radius: usize, (cx, cy): (usize, usize), color: Color) {
    if !fb.is_drawable() {
        return;
    }

    let (Ok(radius), Ok(center_x), Ok(center_y), Ok(width), Ok(height)) = (
        isize::try_from(radius),
        isize::try_from(cx),
        isize::try_from(cy),
        isize::try_from(fb.width),
        isize::try_from(fb.height),
    ) else {
        return;
    };
    let Some(radius_squared) = radius.checked_mul(radius) else {
        return;
    };

    for delta_y in -radius..=radius {
        for delta_x in -radius..=radius {
            let Some(distance_squared) = delta_x
                .checked_mul(delta_x)
                .and_then(|x_squared| delta_y.checked_mul(delta_y)?.checked_add(x_squared))
            else {
                continue;
            };

            if distance_squared <= radius_squared {
                let (Some(pixel_x), Some(pixel_y)) =
                    (center_x.checked_add(delta_x), center_y.checked_add(delta_y))
                else {
                    continue;
                };

                if pixel_x >= 0 && pixel_y >= 0 && pixel_x < width && pixel_y < height {
                    let pixel_index = pixel_y.cast_unsigned() * fb.stride + pixel_x.cast_unsigned();
                    // SAFETY: the preceding bounds check keeps the pixel in the framebuffer.
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
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// # let fb = Framebuffer::for_doc_test();
/// agnostos::graphics::draw_line(&fb, (100, 100), (100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_line(fb: &Framebuffer, (x1, y1): (i64, i64), (x2, y2): (i64, i64), color: Color) {
    if !fb.is_drawable() {
        return;
    }

    let (Ok(width), Ok(height)) = (i64::try_from(fb.width), i64::try_from(fb.height)) else {
        return;
    };
    let delta_x = (x2 - x1).abs();
    let delta_y = (y2 - y1).abs();
    let sx = if x2 >= x1 { 1 } else { -1 };
    let sy = if y2 >= y1 { 1 } else { -1 };
    let mut err = delta_x - delta_y;

    let (mut x, mut y) = (x1, y1);

    loop {
        if x >= 0 && y >= 0 && x < width && y < height {
            let (Ok(row), Ok(column)) = (usize::try_from(y), usize::try_from(x)) else {
                continue;
            };
            let pixel_index = row * fb.stride + column;
            // SAFETY: the bounds check above keeps `(x, y)` in the framebuffer.
            unsafe {
                fb.write_pixel(pixel_index, &color);
            }
        }

        if x == x2 && y == y2 {
            break;
        }

        let e2 = 2 * err;

        if e2 > -delta_y {
            err -= delta_y;
            x += sx;
        }

        if e2 < delta_x {
            err += delta_x;
            y += sy;
        }
    }
}

/// Scrolls the framebuffer content up by `rows` pixel rows, clearing the freed strip at the bottom.
pub fn scroll_up(fb: &Framebuffer, rows: usize) {
    if !fb.is_drawable() {
        return;
    }
    if rows >= fb.height {
        // SAFETY: `is_drawable` verifies the entire framebuffer span is valid.
        unsafe {
            core::ptr::write_bytes(fb.ptr, 0, fb.stride * fb.height * 4);
        }
        return;
    }
    if rows == 0 {
        return;
    }

    let row_bytes = fb.stride * 4;
    let moved_bytes = (fb.height - rows) * row_bytes;
    let cleared_bytes = rows * row_bytes;

    // SAFETY: `is_drawable` verifies the whole `stride * height * 4` byte
    // region is valid. `copy` has memmove semantics, which safely handles the
    // overlapping source and destination ranges.
    unsafe {
        core::ptr::copy(fb.ptr.add(rows * row_bytes), fb.ptr, moved_bytes);
        core::ptr::write_bytes(fb.ptr.add(moved_bytes), 0, cleared_bytes);
    }
}

/// Renders the provided text on the screen, at the provided coordinates with the provided color and font.
///
/// **Example**
///
/// ```
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// # let fb = Framebuffer::for_doc_test();
/// agnostos::graphics::draw_text(&fb, "Random text to render", (100, 200), Color { r: 0, g: 0, b: 0 }, None);
/// ```
pub fn draw_text(
    fb: &Framebuffer,
    text: &str,
    (x, y): (usize, usize),
    color: Color,
    font_height: Option<RasterHeight>,
) {
    if !fb.is_drawable() {
        return;
    }

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

        draw_glyph(fb, &char_raster, cursor_x, y, color);
        let Some(next_cursor_x) = cursor_x.checked_add(char_raster.width()) else {
            return;
        };
        cursor_x = next_cursor_x;
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
                continue; // fully transparent, skip
            }

            let Some(pixel_x) = origin_x.checked_add(col) else {
                continue;
            };
            let Some(pixel_y) = origin_y.checked_add(row) else {
                continue;
            };

            if pixel_x >= fb.width || pixel_y >= fb.height {
                continue;
            }

            // blend intensity with color
            let red = blend_channel(color.r, intensity);
            let green = blend_channel(color.g, intensity);
            let blue = blend_channel(color.b, intensity);

            let pixel_index = pixel_y * fb.stride + pixel_x;
            // SAFETY: out-of-bounds glyph pixels are skipped immediately above.
            unsafe { fb.write_pixel(pixel_index, &Color::new(red, green, blue)) };
        }
    }
}

fn blend_channel(channel: u8, intensity: u8) -> u8 {
    let blended = u16::from(channel) * u16::from(intensity) / 255;
    u8::try_from(blended).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn draw_line_colors_every_pixel_on_a_horizontal_line() {
        let mut pixels = vec![0u8; 5 * 3 * 4];
        let fb = Framebuffer {
            ptr: pixels.as_mut_ptr(),
            width: 5,
            height: 3,
            stride: 5,
            pixel_format: PixelFormat::Rgb,
            byte_len: pixels.len(),
        };

        draw_line(&fb, (1, 1), (3, 1), Color::new(255, 0, 0));

        for x in 1..=3 {
            let offset = (fb.stride + x) * 4;
            assert_eq!(&pixels[offset..offset + 3], &[255, 0, 0]);
        }
    }

    #[test]
    fn malformed_framebuffer_is_not_drawable_or_written() {
        let mut pixels = vec![0x55u8; 3];
        let fb = Framebuffer {
            ptr: pixels.as_mut_ptr(),
            width: 1,
            height: 1,
            stride: 1,
            pixel_format: PixelFormat::Rgb,
            byte_len: pixels.len(),
        };

        assert!(!fb.is_drawable());
        clear_background(&fb, &Color::new(1, 2, 3));
        assert_eq!(&pixels, &[0x55, 0x55, 0x55]);
    }

    #[test]
    fn bitmask_framebuffer_is_not_drawable() {
        let mut pixels = vec![0u8; 4];
        let fb = Framebuffer {
            ptr: pixels.as_mut_ptr(),
            width: 1,
            height: 1,
            stride: 1,
            pixel_format: PixelFormat::Bitmask,
            byte_len: pixels.len(),
        };

        assert!(!fb.is_drawable());
    }
}
