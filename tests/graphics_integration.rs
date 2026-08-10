#![no_std]
extern crate alloc;

use alloc::vec;

use agnostos::{
    color::Color,
    graphics::{self, Framebuffer},
};
use uefi::proto::console::gop::PixelFormat;

fn framebuffer(bytes: &mut [u8], width: usize, height: usize) -> Framebuffer {
    // SAFETY: every test keeps `bytes` alive and does not access it while drawing.
    unsafe { Framebuffer::from_mut_slice(bytes, width, height, width, PixelFormat::Rgb) }.unwrap()
}

#[test]
fn clears_screen_then_draws_rectangle() {
    let mut bytes = vec![0u8; 4 * 4 * 4];
    let fb = framebuffer(&mut bytes, 4, 4);

    graphics::clear_background(&fb, &Color::new(0, 0, 0));
    graphics::draw_rec(&fb, (1, 1), (2, 2), Color::new(255, 0, 0));

    let red_pixel = (fb.stride() + 1) * 4;
    assert_eq!(&bytes[red_pixel..red_pixel + 3], &[255, 0, 0]);
    assert_eq!(&bytes[0..3], &[0, 0, 0]);
}

#[test]
fn scrolling_moves_pixels_up_and_clears_bottom_row() {
    let mut bytes = vec![0u8; 2 * 3 * 4];
    let fb = framebuffer(&mut bytes, 2, 3);

    graphics::draw_rec(&fb, (0, 0), (2, 1), Color::new(255, 0, 0));
    graphics::draw_rec(&fb, (0, 1), (2, 1), Color::new(0, 255, 0));
    graphics::draw_rec(&fb, (0, 2), (2, 1), Color::new(0, 0, 255));

    graphics::scroll_up(&fb, 1);

    assert_eq!(&bytes[0..3], &[0, 255, 0]);
    let bottom_row = 2 * fb.stride() * 4;
    assert_eq!(&bytes[bottom_row..bottom_row + 3], &[0, 0, 0]);
}

#[test]
fn scrolling_past_the_framebuffer_clears_it() {
    let mut bytes = vec![0xAA; 2 * 3 * 4];
    let fb = framebuffer(&mut bytes, 2, 3);

    graphics::scroll_up(&fb, 4);

    assert!(bytes.iter().all(|&byte| byte == 0));
}
