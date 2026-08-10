use core::sync::atomic::{AtomicBool, AtomicUsize};

use noto_sans_mono_bitmap::{FontWeight, RasterHeight};

pub(crate) const PROMPT: &str = "> ";

pub(crate) const FONT_WEIGHT: FontWeight = FontWeight::Regular;
pub(crate) const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;

pub(crate) static HEAP_START: AtomicUsize = AtomicUsize::new(0);
pub(crate) static HEAP_SIZE: AtomicUsize = AtomicUsize::new(0);

pub(crate) static BOOT_SERVICES_EXITED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "mouse")]
pub(crate) static CURSOR_W: usize = 20;
#[cfg(feature = "mouse")]
pub(crate) static CURSOR_H: usize = 20;
