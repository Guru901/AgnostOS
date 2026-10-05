//! AgnostOS kernel.
//!
//! This crate is the UEFI kernel: boot hand-off, physical memory, paging,
//! interrupts, the framebuffer console, and a cooperative scheduler. Host
//! tests compile the same modules without UEFI boot services; the `uefi-bin`
//! feature produces the EFI binary.
#![no_std]
#![feature(abi_x86_interrupt)]
extern crate alloc;

#[cfg(test)]
extern crate std;

/// Heap selection after `ExitBootServices`, plus the optional custom allocator.
pub mod allocator;

/// Kernel-owned physical memory map and ownership classifications.
pub mod memory;

/// Fixed-storage physical 4 KiB frame allocator.
pub mod frame;

/// x86_64 virtual-address layout and page-table ownership.
pub mod paging;

/// Ordered kernel startup sequence and UEFI hand-off.
pub mod boot;

/// Shared tunables and boot-time globals (heap bounds, tick counter, layout).
pub(crate) mod globals;
pub(crate) use globals::*;
/// CPU operations with no-op / spin fallbacks so host tests can compile.
pub(crate) mod platform;

/// Returns whether the UEFI boot-services transition has completed.
#[must_use]
pub fn boot_services_exited() -> bool {
    BOOT_SERVICES_EXITED.load(core::sync::atomic::Ordering::Relaxed)
}

/// Interrupt controller setup, descriptor tables, and hardware IRQ handlers.
pub mod interrupts;

/// Direct framebuffer drawing after ExitBootServices (pixels, primitives, text).
pub mod graphics;

/// UEFI GOP mode selection used only while boot services are still available.
pub mod uefi_graphics;

#[cfg(feature = "uefi-bin")]
mod uefi_compat;

/// Kernel text console: cursor, history, and `kprintln!` over the framebuffer.
pub mod console;

/// RGB color values used by drawing and console output.
pub mod color;

/// PS/2 keyboard scancode queue and decoded shell events.
pub mod keyboard;

#[cfg(feature = "input-smoke")]
pub(crate) mod input_smoke;

#[cfg(feature = "fault-smoke")]
pub(crate) mod fault_smoke;

#[cfg(feature = "timer-smoke")]
pub(crate) mod timer_smoke;

/// PIT-backed tick counter and millisecond sleep helpers.
pub mod timer;

/// PS/2 mouse byte queue, packet decode, and software cursor.
#[cfg(feature = "mouse")]
pub mod mouse;

/// Interactive command loop driven by keyboard (and optional mouse) IRQs.
pub mod shell;

/// Shell command parse-and-dispatch table.
pub mod commands;

/// Allocation-free cooperative task scheduler.
pub mod task;
