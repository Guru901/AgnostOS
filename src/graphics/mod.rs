//! Framebuffer access, primitive drawing, and bitmap text rendering.

mod drawing;
mod framebuffer;
pub mod pixel;
mod text;

pub use drawing::{clear_background, draw_circle, draw_line, draw_rec, scroll_up};
pub use framebuffer::{Framebuffer, FramebufferBytes, FramebufferError};
pub use pixel::{PixelCoord, PixelRadius, PixelRows, PixelSize, Stride};
pub use text::draw_text;

#[cfg(test)]
mod tests {
    use alloc::vec;
    use uefi::proto::console::gop::PixelFormat;

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
            crate::color::Color::new(255, 0, 0),
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
