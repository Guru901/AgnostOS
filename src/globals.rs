use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};

use noto_sans_mono_bitmap::{FontWeight, RasterHeight};

/// Terminal prompt
pub(crate) const PROMPT: &str = "> ";

/// Maximum number of editable characters accepted on one shell command line.
pub(crate) const MAX_INPUT_CHARS: usize = 512;

pub(crate) const FONT_WEIGHT: FontWeight = FontWeight::Regular;
pub(crate) const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;

pub(crate) static HEAP_START: AtomicUsize = AtomicUsize::new(0);
pub(crate) static HEAP_SIZE: AtomicUsize = AtomicUsize::new(0);

pub(crate) static BOOT_SERVICES_EXITED: AtomicBool = AtomicBool::new(false);

pub(crate) static TICKS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "mouse")]
pub(crate) static CURSOR_W: usize = 20;
#[cfg(feature = "mouse")]
pub(crate) static CURSOR_H: usize = 20;

pub(crate) mod command {
    pub(crate) const COMMAND_NAMES: &[&str] = &[
        "about", "clear", "echo", "font", "help", "history", "meminfo", "shutdown", "uptime",
    ];
}

pub(crate) mod frame {
    pub(crate) const MAX_MEMORY_RANGES: usize = 256;
    pub(crate) const PAGE_SIZE: u64 = 4096;
}

pub(crate) mod paging {
    pub(crate) const KERNEL_BASE: u64 = 0xffff_8000_0000_0000;
    pub(crate) const HEAP_BASE: u64 = 0xffff_9000_0000_0000;
    pub(crate) const FRAMEBUFFER_BASE: u64 = 0xffff_a000_0000_0000;
    pub(crate) const MMIO_BASE: u64 = 0xffff_b000_0000_0000;
    pub(crate) const STACK_BASE: u64 = 0xffff_c000_0000_0000;

    pub(crate) const LOW_CANONICAL_MAX: u64 = 0x0000_7fff_ffff_ffff;
    pub(crate) const HIGH_CANONICAL_MIN: u64 = 0xffff_8000_0000_0000;
    pub(crate) const MAX_PAGE_TABLE_FRAMES: usize = 512;
    pub(crate) const RECURSIVE_INDEX: u16 = 510;
}

pub(crate) mod gdt {
    pub(crate) const NMI_IST_INDEX: u16 = 0;
    pub(crate) const DOUBLE_FAULT_IST_INDEX: u16 = 1;
    pub(crate) const MACHINE_CHECK_IST_INDEX: u16 = 2;

    pub(crate) const IST_STACK_SIZE: usize = 32 * 1024;
}

pub(crate) mod task {
    pub(crate) const TASK_STACK_SIZE: usize = 4 * 1024;
    pub(crate) const STACK_FRAME_WORDS: usize = 7;
    pub(crate) const MAX_TASKS: usize = 64;
}

pub(crate) mod timer {
    pub(crate) const PIT_FREQUENCY: u64 = 1_193_182;
    pub(crate) const TIMER_FREQUENCY: u64 = 1_000;
    pub(crate) const PIT_DIVISOR: u64 = PIT_FREQUENCY / TIMER_FREQUENCY;
}
