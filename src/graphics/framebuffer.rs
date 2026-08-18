use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

use crate::color::Color;

use super::{PixelSize, Stride};

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
    pixel_format: PixelFormat,
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
    pub fn new(gop: &mut GraphicsOutput) -> Result<Self, FramebufferError> {
        let mode_info = gop.current_mode_info();
        let (width, height) = mode_info.resolution();
        let size = PixelSize::try_new(width, height).ok_or(FramebufferError::InvalidGeometry)?;
        let stride =
            Stride::try_new(mode_info.stride()).ok_or(FramebufferError::InvalidGeometry)?;
        let pixel_format = mode_info.pixel_format();
        let mut framebuffer = gop.frame_buffer();

        Self::from_raw_parts(
            framebuffer.as_mut_ptr(),
            FramebufferBytes::new(framebuffer.size()),
            size,
            stride,
            pixel_format,
        )
    }

    /// Builds a framebuffer backed by a caller-provided byte slice.
    ///
    /// # Safety
    ///
    /// The backing slice must remain allocated and exclusively available until
    /// the returned framebuffer is no longer used.
    #[doc(hidden)]
    pub unsafe fn from_mut_slice(
        bytes: &mut [u8],
        size: PixelSize,
        stride: Stride,
        pixel_format: PixelFormat,
    ) -> Result<Self, FramebufferError> {
        Self::from_raw_parts(
            bytes.as_mut_ptr(),
            FramebufferBytes::new(bytes.len()),
            size,
            stride,
            pixel_format,
        )
    }

    fn from_raw_parts(
        ptr: *mut u8,
        byte_len: FramebufferBytes,
        size: PixelSize,
        stride: Stride,
        pixel_format: PixelFormat,
    ) -> Result<Self, FramebufferError> {
        if !matches!(pixel_format, PixelFormat::Rgb | PixelFormat::Bgr) {
            return Err(FramebufferError::UnsupportedPixelFormat(pixel_format));
        }

        let required =
            Self::required_byte_len(size, stride).ok_or(FramebufferError::InvalidGeometry)?;
        if byte_len < required {
            return Err(FramebufferError::FramebufferTooSmall {
                required,
                actual: byte_len,
            });
        }

        Ok(Self {
            ptr,
            size,
            stride,
            pixel_format,
            byte_len,
        })
    }

    #[must_use]
    pub const fn size(&self) -> PixelSize {
        self.size
    }

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

    #[must_use]
    pub fn is_drawable(&self) -> bool {
        !self.ptr.is_null()
            && matches!(self.pixel_format, PixelFormat::Rgb | PixelFormat::Bgr)
            && Self::required_byte_len(self.size, self.stride)
                .is_some_and(|required| required <= self.byte_len)
    }

    #[inline]
    #[cfg(feature = "mouse")]
    pub(crate) fn read_pixel(&self, x: usize, y: usize) -> Color {
        if !self.is_drawable() || x >= self.width() || y >= self.height() {
            return crate::color::BLACK;
        }
        let Some(offset) = y
            .checked_mul(self.stride())
            .and_then(|row| row.checked_add(x))
            .and_then(|pixel| pixel.checked_mul(4))
        else {
            return crate::color::BLACK;
        };
        if offset
            .checked_add(4)
            .is_none_or(|end| end > self.byte_len.get())
        {
            return crate::color::BLACK;
        }

        // SAFETY: coordinate and byte-range validation above guarantee a full pixel.
        let p = unsafe { self.ptr.add(offset) };
        // SAFETY: `p..p+3` lies within the validated pixel.
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
            _ => crate::color::BLACK,
        }
    }

    #[inline]
    pub(crate) fn write_pixel(&self, pixel_index: usize, color: &Color) -> bool {
        let rgb = match self.pixel_format {
            PixelFormat::Rgb => [color.r, color.g, color.b],
            PixelFormat::Bgr => [color.b, color.g, color.r],
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
        // SAFETY: `offset` identifies a complete pixel in the validated mapping.
        let p = unsafe { self.ptr.add(offset) };
        // SAFETY: the first three bytes of the complete pixel are writable.
        unsafe {
            p.write_volatile(rgb[0]);
            p.add(1).write_volatile(rgb[1]);
            p.add(2).write_volatile(rgb[2]);
        }
        true
    }

    pub(crate) fn scroll_rows(&self, rows: usize) {
        if !self.is_drawable() || rows == 0 {
            return;
        }
        let row_bytes = self.stride() * 4;
        if rows >= self.height() {
            // SAFETY: `is_drawable` validates the full framebuffer span.
            unsafe { core::ptr::write_bytes(self.ptr, 0, self.stride() * self.height() * 4) };
            return;
        }
        let moved_bytes = (self.height() - rows) * row_bytes;
        let cleared_bytes = rows * row_bytes;
        // SAFETY: the source and destination ranges are contained in the mapping.
        unsafe {
            core::ptr::copy(self.ptr.add(rows * row_bytes), self.ptr, moved_bytes);
            core::ptr::write_bytes(self.ptr.add(moved_bytes), 0, cleared_bytes);
        }
    }
}
