//! Kernel-wide tunables and a few boot-time atomics.
//!
//! Subsystems import these instead of repeating magic numbers. Changing a
//! layout constant here is the intended way to keep paging, frames, and the
//! timer in agreement.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};

use noto_sans_mono_bitmap::{FontWeight, RasterHeight};

/// Shell prompt drawn at the start of each input line.
pub(crate) const PROMPT: &str = "> ";

/// Maximum number of editable characters accepted on one shell command line.
pub(crate) const MAX_INPUT_CHARS: usize = 512;

pub(crate) const FONT_WEIGHT: FontWeight = FontWeight::Regular;
pub(crate) const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;

/// Physical start of the owned heap, recorded for `meminfo`.
pub(crate) static HEAP_START: AtomicUsize = AtomicUsize::new(0);
/// Heap size in bytes, recorded for `meminfo`.
pub(crate) static HEAP_SIZE: AtomicUsize = AtomicUsize::new(0);

/// Set once `ExitBootServices` succeeds; panic output switches consoles on this.
pub(crate) static BOOT_SERVICES_EXITED: AtomicBool = AtomicBool::new(false);

/// PIT IRQ count. Converted to milliseconds with [`self::timer::PIT_DIVISOR`].
pub(crate) static TICKS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "mouse")]
pub(crate) static CURSOR_W: usize = 20;
#[cfg(feature = "mouse")]
pub(crate) static CURSOR_H: usize = 20;

/// Names Tab-complete is allowed to match. Keep in sync with [`crate::commands`].
pub(crate) mod command {
    pub(crate) const COMMAND_NAMES: &[&str] = &[
        "about", "clear", "echo", "font", "help", "history", "meminfo", "shutdown", "uptime",
    ];
}

pub(crate) mod frame {
    /// Upper bound on UEFI map entries copied into kernel storage.
    pub(crate) const MAX_MEMORY_RANGES: usize = 256;
    /// x86_64 4 KiB page; every allocated frame is this size.
    pub(crate) const PAGE_SIZE: u64 = 4096;
}

/// Higher-half virtual windows used by bootstrap paging.
///
/// Each region is a 1 TiB slice so identity-mapped physical ranges can be
/// relocated later without colliding. Addresses sit in the canonical high half.
pub(crate) mod paging {
    pub(crate) const KERNEL_BASE: u64 = 0xffff_8000_0000_0000;
    pub(crate) const HEAP_BASE: u64 = 0xffff_9000_0000_0000;
    pub(crate) const FRAMEBUFFER_BASE: u64 = 0xffff_a000_0000_0000;
    pub(crate) const MMIO_BASE: u64 = 0xffff_b000_0000_0000;
    pub(crate) const STACK_BASE: u64 = 0xffff_c000_0000_0000;

    /// Largest valid low-half canonical address (bits 48..=63 must match bit 47).
    pub(crate) const LOW_CANONICAL_MAX: u64 = 0x0000_7fff_ffff_ffff;
    /// Smallest valid high-half canonical address.
    pub(crate) const HIGH_CANONICAL_MIN: u64 = 0xffff_8000_0000_0000;
    /// Bootstrap page-table frames kept inside the loaded image.
    pub(crate) const MAX_PAGE_TABLE_FRAMES: usize = 512;
    /// P4 slot used for recursive mapping after CR3 is loaded (510, not 511).
    pub(crate) const RECURSIVE_INDEX: u16 = 510;
}

/// Interrupt Stack Table slots in the TSS. Indices must match the IDT entries.
pub(crate) mod gdt {
    pub(crate) const NMI_IST_INDEX: u16 = 0;
    pub(crate) const DOUBLE_FAULT_IST_INDEX: u16 = 1;
    pub(crate) const MACHINE_CHECK_IST_INDEX: u16 = 2;

    pub(crate) const IST_STACK_SIZE: usize = 32 * 1024;
}

pub(crate) mod task {
    /// Bootstrap per-task stack; later work should allocate real frames.
    pub(crate) const TASK_STACK_SIZE: usize = 4 * 1024;
    /// Callee-saved registers plus the trampoline return address.
    pub(crate) const STACK_FRAME_WORDS: usize = 7;
    pub(crate) const MAX_TASKS: usize = 64;
}

/// PIT channel 0 is programmed for ~1 kHz; conversion uses the integer divisor.
pub(crate) mod timer {
    /// Oscillator rate of the IBM-compatible PIT, in Hz.
    pub(crate) const PIT_FREQUENCY: u64 = 1_193_182;
    /// Desired IRQ rate after programming channel 0.
    pub(crate) const TIMER_FREQUENCY: u64 = 1_000;
    pub(crate) const PIT_DIVISOR: u64 = PIT_FREQUENCY / TIMER_FREQUENCY;
}
