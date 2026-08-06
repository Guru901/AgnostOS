#![no_std]
#![feature(abi_x86_interrupt)]
extern crate alloc;

/// Module that contains the code for our custom allocator.
pub mod allocator;

pub mod constants;
pub use constants::*;

/// Module that contains the code for interrupts
pub mod idt;

/// Module that contains the code for rendering things to the screen after exiting uefi boot
/// services. It usses framebuffer to write the bytes directly
pub mod graphics;

/// Module that contains the code for rendering things to the screen when in uefi. It usses gop.
pub mod uefi_graphics;

/// Module that contains the code for printing text to the screen same way println! does
pub mod console;

/// Module that contains the code for using Colors
pub mod color;

/// Module that contains the code for handling keyboard.
pub mod keyboard;

pub mod mouse;

/// Module that contains the code for shell.
pub mod shell;

pub mod commands;

use alloc::string::String;
use alloc::vec::Vec;

use crate::color::Color;

pub struct ScreenCell {
    pub ch: char,
    pub color: Color,
}

pub struct TextGrid {
    cols: usize,
    rows: usize,
    cells: Vec<ScreenCell>,
}

impl TextGrid {
    pub fn get(&self, col: usize, row: usize) -> Option<&ScreenCell> {
        self.cells.get(row * self.cols + col)
    }

    pub fn set(&mut self, col: usize, row: usize, ch: char, color: Color) {
        if let Some(cell) = self.cells.get_mut(row * self.cols + col) {
            cell.ch = ch;
            cell.color = color;
        }
    }
}
