//! AgnostOS kernel.
//!
//! This crate is the UEFI kernel: boot hand-off, physical memory, paging,
//! interrupts, the framebuffer console, and a cooperative scheduler. Host
//! tests compile the same modules without UEFI boot services; the `uefi-bin`
//! feature produces the EFI binary.
#![no_std]
#![feature(abi_x86_interrupt)]
extern crate alloc;

/// Prints to the kernel console without a trailing newline.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {
        $crate::console::kprint(format_args!($($arg)*))
    };
}

/// Prints to the kernel console with a trailing newline.
#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($($arg:tt)*) => {{
        $crate::kprint!($($arg)*);
        $crate::kprint!("\n");
    }};
}

/// Heap selection after `ExitBootServices`, plus the optional custom allocator.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/allocator/mod.rs")
)]
pub mod allocator;

/// Kernel-owned physical memory map and ownership classifications.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/memory.rs")
)]
pub mod memory;

/// Fixed-storage physical 4 KiB frame allocator.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/frame.rs")
)]
pub mod frame;

/// x86_64 virtual-address layout and page-table ownership.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/paging.rs")
)]
pub mod paging;

/// Ordered kernel startup sequence and UEFI hand-off.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/boot.rs")
)]
pub mod boot;

/// Shared tunables and boot-time globals (heap bounds, tick counter, layout).
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/globals.rs")
)]
pub(crate) mod globals;
pub(crate) use globals::*;
/// CPU operations with no-op / spin fallbacks so host tests can compile.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/platform/mod.rs")
)]
pub(crate) mod platform;

/// Returns whether the UEFI boot-services transition has completed.
#[must_use]
pub fn boot_services_exited() -> bool {
    BOOT_SERVICES_EXITED.load(core::sync::atomic::Ordering::Relaxed)
}

/// Interrupt controller setup, descriptor tables, and hardware IRQ handlers.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/interrupts/mod.rs")
)]
pub mod interrupts;

/// Direct framebuffer drawing after ExitBootServices (pixels, primitives, text).
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/graphics/mod.rs")
)]
pub mod graphics;

/// UEFI GOP mode selection used only while boot services are still available.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/uefi_graphics/mod.rs")
)]
pub mod uefi_graphics;

#[cfg(feature = "uefi-bin")]
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/uefi_compat.rs")
)]
mod uefi_compat;

/// Kernel text console: cursor, history, and `kprintln!` over the framebuffer.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/console/mod.rs")
)]
pub mod console;

/// RGB color values used by drawing and console output.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/color/mod.rs")
)]
pub mod color;

/// PS/2 keyboard scancode queue and decoded shell events.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/keyboard/mod.rs")
)]
pub mod keyboard;

#[cfg(feature = "input-smoke")]
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/input_smoke.rs")
)]
pub(crate) mod input_smoke;

#[cfg(feature = "fault-smoke")]
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/fault_smoke.rs")
)]
pub(crate) mod fault_smoke;

#[cfg(feature = "timer-smoke")]
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/timer_smoke.rs")
)]
pub(crate) mod timer_smoke;

#[cfg(feature = "profiling")]
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/profiling.rs")
)]
pub(crate) mod profiling;

/// PIT-backed tick counter and millisecond sleep helpers.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/timer/mod.rs")
)]
pub mod timer;

/// PS/2 mouse byte queue, packet decode, and software cursor.
#[cfg(feature = "mouse")]
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/mouse/mod.rs")
)]
pub mod mouse;

/// Interactive command loop driven by keyboard (and optional mouse) IRQs.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/shell/mod.rs")
)]
pub mod shell;

/// Shell command parse-and-dispatch table.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/commands/mod.rs")
)]
pub mod commands;

/// Allocation-free cooperative task scheduler.
#[cfg_attr(
    feature = "profiling",
    agnostos_profile_macro::profiled_module("src/task/mod.rs")
)]
pub mod task;
