use core::sync::atomic::{AtomicBool, AtomicUsize};

use noto_sans_mono_bitmap::{FontWeight, RasterHeight};

pub(crate) const PROMPT: &str = "> ";

pub(crate) const FONT_WEIGHT: FontWeight = FontWeight::Regular;
pub(crate) const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;

pub static HEAP_START: AtomicUsize = AtomicUsize::new(0);
pub static HEAP_SIZE: AtomicUsize = AtomicUsize::new(0);

pub static BOOT_SERVICES_EXITED: AtomicBool = AtomicBool::new(false);

pub static CURSOR_W: usize = 20;
pub static CURSOR_H: usize = 20;
