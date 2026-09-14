#![no_std]
#![feature(abi_x86_interrupt)]
extern crate alloc;

/// Module that contains the code for our custom allocator.
pub mod allocator;

/// Kernel-owned physical memory map and ownership classifications.
pub mod memory;

/// Fixed-storage physical 4 KiB frame allocator.
pub mod frame;

/// Ordered kernel startup sequence and UEFI hand-off.
pub mod boot;

pub(crate) mod constants;
pub(crate) use constants::*;
pub(crate) mod platform;

/// Returns whether the UEFI boot-services transition has completed.
#[must_use]
pub fn boot_services_exited() -> bool {
    BOOT_SERVICES_EXITED.load(core::sync::atomic::Ordering::Relaxed)
}

/// Interrupt controller setup, descriptor tables, and hardware IRQ handlers.
pub mod interrupts;

/// Module that contains the code for rendering things to the screen after exiting uefi boot
/// services. It usses framebuffer to write the bytes directly
pub mod graphics;

/// Module that contains the code for rendering things to the screen when in uefi. It usses gop.
pub mod uefi_graphics;

#[cfg(feature = "uefi-bin")]
mod uefi_compat;

/// Module that contains the code for printing text to the screen same way println! does
pub mod console;

/// Module that contains the code for using Colors
pub mod color;

/// Module that contains the code for handling keyboard.
pub mod keyboard;

#[cfg(feature = "input-smoke")]
pub(crate) mod input_smoke;

#[cfg(feature = "fault-smoke")]
pub(crate) mod fault_smoke;

pub mod timer;

#[cfg(feature = "mouse")]
pub mod mouse;

/// Module that contains the code for shell.
pub mod shell;

pub mod commands;
