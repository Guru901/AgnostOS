use noto_sans_mono_bitmap::{RasterHeight, RasterizedChar, get_raster};
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

use crate::{FONT_HEIGHT, FONT_WEIGHT, color::Color};

pub mod pixel;
pub use pixel::{PixelCoord, PixelRadius, PixelRows, PixelSize, Stride};

/// A count of bytes in a framebuffer mapping.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct FramebufferBytes(usize);

impl FramebufferBytes {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

#[derive(Debug)]
pub struct Framebuffer {
    ptr: *mut u8,
    size: PixelSize,
    stride: Stride,
    /// Pixel layout selected by GOP. Direct rendering supports only `Rgb` and
    /// `Bgr`, both of which are 32-bit formats.
    pixel_format: PixelFormat,
    /// Number of bytes reported by GOP for the framebuffer mapping.
    byte_len: FramebufferBytes,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FramebufferError {
    UnsupportedPixelFormat(PixelFormat),
    InvalidGeometry,
    FramebufferTooSmall {
        required: FramebufferBytes,
        actual: FramebufferBytes,
    },
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
        let size = PixelSize::try_new(width, height).ok_or(FramebufferError::InvalidGeometry)?;
        let stride =
            Stride::try_new(mode_info.stride()).ok_or(FramebufferError::InvalidGeometry)?;
        let pixel_format = mode_info.pixel_format();

        if !matches!(pixel_format, PixelFormat::Rgb | PixelFormat::Bgr) {
            return Err(FramebufferError::UnsupportedPixelFormat(pixel_format));
        }

        let required =
            Self::required_byte_len(size, stride).ok_or(FramebufferError::InvalidGeometry)?;
        let mut framebuffer = gop.frame_buffer();
        let byte_len = FramebufferBytes::new(framebuffer.size());
        if byte_len < required {
            return Err(FramebufferError::FramebufferTooSmall {
                required,
                actual: byte_len,
            });
        }

        Ok(Self {
            ptr: framebuffer.as_mut_ptr(),
            size,
            stride,
            pixel_format,
            byte_len,
        })
    }

    /// Builds a framebuffer backed by a caller-provided byte slice.
    ///
    /// This is primarily useful for host-side rendering tests. The geometry and
    /// backing length are checked before a framebuffer is created. The caller
    /// must keep `bytes` alive and must not access it while the framebuffer is
    /// being drawn to.
    ///
    /// # Safety
    ///
    /// The backing slice must remain allocated and exclusively available for
    /// framebuffer access until the returned `Framebuffer` is no longer used.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported pixel formats, invalid geometry, or a
    /// backing slice that is too small.
    #[doc(hidden)]
    pub unsafe fn from_mut_slice(
        bytes: &mut [u8],
        size: PixelSize,
        stride: Stride,
        pixel_format: PixelFormat,
    ) -> Result<Self, FramebufferError> {
        if !matches!(pixel_format, PixelFormat::Rgb | PixelFormat::Bgr) {
            return Err(FramebufferError::UnsupportedPixelFormat(pixel_format));
        }
        let required =
            Self::required_byte_len(size, stride).ok_or(FramebufferError::InvalidGeometry)?;
        let byte_len = FramebufferBytes::new(bytes.len());
        if byte_len < required {
            return Err(FramebufferError::FramebufferTooSmall {
                required,
                actual: byte_len,
            });
        }

        Ok(Self {
            ptr: bytes.as_mut_ptr(),
            size,
            stride,
            pixel_format,
            byte_len,
        })
    }

    /// Width of the visible framebuffer, in pixels.
    #[must_use]
    pub const fn size(&self) -> PixelSize {
        self.size
    }

    /// Height of the visible framebuffer, in pixels.
    #[must_use]
    pub const fn row_stride(&self) -> Stride {
        self.stride
    }

    pub(crate) const fn width(&self) -> usize {
        self.size.width()
    }
    pub(crate) const fn height(&self) -> usize {
        self.size.height()
    }
    pub(crate) const fn stride(&self) -> usize {
        self.stride.get()
    }

    fn required_byte_len(size: PixelSize, stride: Stride) -> Option<FramebufferBytes> {
        if size.width() > stride.get() {
            return None;
        }

        Some(FramebufferBytes::new(
            stride.get().checked_mul(size.height())?.checked_mul(4)?,
        ))
    }

    /// Returns whether this framebuffer is suitable for direct 32-bit drawing.
    /// Drawing functions treat an invalid framebuffer as a no-op.
    #[must_use]
    pub fn is_drawable(&self) -> bool {
        !self.ptr.is_null()
            && matches!(self.pixel_format, PixelFormat::Rgb | PixelFormat::Bgr)
            && Self::required_byte_len(self.size, self.stride)
                .is_some_and(|required| required <= self.byte_len)
    }

    #[inline]
    #[cfg(feature = "mouse")]
    /// # Safety
    ///
    /// `self` must retain a valid framebuffer mapping for the duration of this
    /// call. Callers must also prevent concurrent non-atomic access to it.
    pub(crate) unsafe fn read_pixel(&self, x: usize, y: usize) -> Color {
        use crate::color;
        if !self.is_drawable() || x >= self.size.width() || y >= self.size.height() {
            return color::BLACK;
        }

        let Some(pixel_index) = y
            .checked_mul(self.stride.get())
            .and_then(|row| row.checked_add(x))
        else {
            return color::BLACK;
        };
        let Some(offset) = pixel_index.checked_mul(4) else {
            return color::BLACK;
        };
        if offset
            .checked_add(4)
            .is_none_or(|end| end > self.byte_len.get())
        {
            return color::BLACK;
        }

        // SAFETY: the framebuffer invariant and checks above make `offset..offset
        // + 4` a valid range within the framebuffer allocation.
        let p = unsafe { self.ptr.add(offset) };
        // SAFETY: `p` and its next two bytes lie in the validated pixel range.
        let (first, second, third) = unsafe {
            (
                p.read_volatile(),
                p.add(1).read_volatile(),
                p.add(2).read_volatile(),
            )
        };

        match self.pixel_format {
            PixelFormat::Rgb => Color::new(first, second, third),
            PixelFormat::Bgr => Color::new(third, second, first),
            _ => color::BLACK,
        }
    }

    #[inline]
    /// # Safety
    ///
    /// `self` must retain a valid framebuffer mapping for the duration of this
    /// call. Callers must also prevent concurrent non-atomic access to it.
    pub(crate) unsafe fn write_pixel(&self, pixel_index: usize, color: &Color) -> bool {
        let rgb = match self.pixel_format {
            PixelFormat::Bgr => [color.b, color.g, color.r],
            PixelFormat::Rgb => [color.r, color.g, color.b],
            _ => return false,
        };
        let Some(offset) = pixel_index.checked_mul(4) else {
            return false;
        };
        if offset
            .checked_add(4)
            .is_none_or(|end| end > self.byte_len.get())
        {
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
/// ```ignore
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// agnostos::graphics::clear_background(&fb, &Color { r: 255, g: 255, b: 255 });
/// ```
pub fn clear_background(fb: &Framebuffer, color: &Color) {
    if !fb.is_drawable() {
        return;
    }

    for row in 0..fb.height() {
        for col in 0..fb.width() {
            let pixel_index = row * fb.stride() + col;
            // SAFETY: `is_drawable` validates the backing range, and `row` and
            // `col` are bounded by the framebuffer dimensions.
            unsafe { fb.write_pixel(pixel_index, color) };
        }
    }
}

/// Renders a rectangle on the screen, at the provided coordinates with the provided color and
/// dimensions.
///
/// # Panics
///
/// Panics when a non-overflowing rectangle extends past the framebuffer's
/// right or bottom edge.
///
/// Returns without drawing when an extent calculation overflows.
///
/// **Example**
///
/// ```ignore
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// agnostos::graphics::draw_rec(&fb, PixelCoord::new(100, 100), PixelSize::new(100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_rec(fb: &Framebuffer, origin: PixelCoord, size: PixelSize, color: Color) {
    let (x, y) = (origin.x(), origin.y());
    let (w, h) = (size.width(), size.height());
    if !fb.is_drawable() {
        return;
    }
    let Some(x2) = x.checked_add(w) else {
        return;
    };
    let Some(y2) = y.checked_add(h) else {
        return;
    };
    assert!(x2 <= fb.width(), "Bad X coordinate");
    assert!(y2 <= fb.height(), "Bad Y coordinate");

    for row in y..y2 {
        for col in x..x2 {
            let pixel_index = row * fb.stride() + col;
            // SAFETY: `is_drawable` validates the backing range, and the coordinate
            // assertions above keep this pixel in bounds.
            unsafe { fb.write_pixel(pixel_index, &color) };
        }
    }
}

/// Renders a circle on the screen, at the provided coordinates with the provided color and radius.
///
/// **Example**
///
/// ```ignore
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// agnostos::graphics::draw_circle(&fb, PixelRadius::new(20), PixelCoord::new(100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_circle(fb: &Framebuffer, radius: PixelRadius, center: PixelCoord, color: Color) {
    let (cx, cy) = (center.x(), center.y());
    if !fb.is_drawable() {
        return;
    }

    let (Ok(radius), Ok(center_x), Ok(center_y), Ok(width), Ok(height)) = (
        isize::try_from(radius.get()),
        isize::try_from(cx),
        isize::try_from(cy),
        isize::try_from(fb.width()),
        isize::try_from(fb.height()),
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
                    let pixel_index =
                        pixel_y.cast_unsigned() * fb.stride() + pixel_x.cast_unsigned();
                    // SAFETY: `is_drawable` validates the backing range, and the
                    // preceding bounds check keeps the pixel in the framebuffer.
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
/// ```ignore
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// agnostos::graphics::draw_line(&fb, PixelCoord::new(100, 100), PixelCoord::new(100, 100), Color { r: 0, g: 0, b: 0 });
/// ```
pub fn draw_line(fb: &Framebuffer, start: PixelCoord, end: PixelCoord, color: Color) {
    if !fb.is_drawable() {
        return;
    }

    let (Ok(width), Ok(height), Ok(x1), Ok(y1), Ok(x2), Ok(y2)) = (
        i64::try_from(fb.width()),
        i64::try_from(fb.height()),
        i64::try_from(start.x()),
        i64::try_from(start.y()),
        i64::try_from(end.x()),
        i64::try_from(end.y()),
    ) else {
        return;
    };
    let Some(delta_x) = x2.checked_sub(x1).or_else(|| x1.checked_sub(x2)) else {
        return;
    };
    let Some(delta_y) = y2.checked_sub(y1).or_else(|| y1.checked_sub(y2)) else {
        return;
    };
    let sx = if x2 >= x1 { 1 } else { -1 };
    let sy = if y2 >= y1 { 1 } else { -1 };
    let mut err = delta_x - delta_y;

    let (mut x, mut y) = (x1, y1);

    loop {
        if x >= 0 && y >= 0 && x < width && y < height {
            let (Ok(row), Ok(column)) = (usize::try_from(y), usize::try_from(x)) else {
                continue;
            };
            let pixel_index = row * fb.stride() + column;
            // SAFETY: `is_drawable` validates the backing range, and the bounds
            // check above keeps `(x, y)` in the framebuffer.
            unsafe {
                fb.write_pixel(pixel_index, &color);
            }
        }

        if x == x2 && y == y2 {
            break;
        }

        let Some(e2) = err.checked_mul(2) else {
            return;
        };

        if e2 > -delta_y {
            let Some(next_err) = err.checked_sub(delta_y) else {
                return;
            };
            let Some(next_x) = x.checked_add(sx) else {
                return;
            };
            err = next_err;
            x = next_x;
        }

        if e2 < delta_x {
            let Some(next_err) = err.checked_add(delta_x) else {
                return;
            };
            let Some(next_y) = y.checked_add(sy) else {
                return;
            };
            err = next_err;
            y = next_y;
        }
    }
}

/// Scrolls the framebuffer content up by `rows` pixel rows, clearing the freed strip at the bottom.
pub fn scroll_up(fb: &Framebuffer, rows: PixelRows) {
    let rows = rows.get();
    if !fb.is_drawable() {
        return;
    }
    if rows >= fb.height() {
        // SAFETY: `is_drawable` verifies the entire framebuffer span is valid.
        unsafe {
            core::ptr::write_bytes(fb.ptr, 0, fb.stride() * fb.height() * 4);
        }
        return;
    }
    if rows == 0 {
        return;
    }

    let row_bytes = fb.stride() * 4;
    let moved_bytes = (fb.height() - rows) * row_bytes;
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
/// ```ignore
/// use agnostos::color::Color;
/// use agnostos::graphics::Framebuffer;
/// agnostos::graphics::draw_text(&fb, "Random text to render", PixelCoord::new(100, 200), Color { r: 0, g: 0, b: 0 }, None);
/// ```
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
    let y = origin.y();

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

            if pixel_x >= fb.width() || pixel_y >= fb.height() {
                continue;
            }

            // blend intensity with color
            let red = blend_channel(color.r, intensity);
            let green = blend_channel(color.g, intensity);
            let blue = blend_channel(color.b, intensity);

            let pixel_index = pixel_y * fb.stride() + pixel_x;
            // SAFETY: `is_drawable` validates the backing range, and out-of-bounds
            // glyph pixels are skipped immediately above.
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
        // SAFETY: `pixels` remains alive and is not otherwise accessed while drawing.
        let fb = unsafe {
            Framebuffer::from_mut_slice(
                &mut pixels,
                PixelSize::new(5, 3),
                Stride::new(5),
                PixelFormat::Rgb,
            )
        }
        .unwrap();

        draw_line(
            &fb,
            PixelCoord::new(1, 1),
            PixelCoord::new(3, 1),
            Color::new(255, 0, 0),
        );

        for x in 1..=3 {
            let offset = (fb.stride() + x) * 4;
            assert_eq!(&pixels[offset..offset + 3], &[255, 0, 0]);
        }
    }

    #[test]
    fn too_small_framebuffer_is_rejected() {
        let mut pixels = vec![0x55u8; 3];
        // SAFETY: the constructor only inspects `pixels` before rejecting it.
        let fb = unsafe {
            Framebuffer::from_mut_slice(
                &mut pixels,
                PixelSize::new(1, 1),
                Stride::new(1),
                PixelFormat::Rgb,
            )
        };

        assert!(matches!(
            fb,
            Err(FramebufferError::FramebufferTooSmall { .. })
        ));
        assert_eq!(&pixels, &[0x55, 0x55, 0x55]);
    }

    #[test]
    fn bitmask_framebuffer_is_not_drawable() {
        let mut pixels = vec![0u8; 4];
        // SAFETY: the constructor only inspects `pixels` before rejecting it.
        let fb = unsafe {
            Framebuffer::from_mut_slice(
                &mut pixels,
                PixelSize::new(1, 1),
                Stride::new(1),
                PixelFormat::Bitmask,
            )
        };

        assert!(matches!(
            fb,
            Err(FramebufferError::UnsupportedPixelFormat(
                PixelFormat::Bitmask
            ))
        ));
    }
}
