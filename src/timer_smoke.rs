//! QEMU-only PIT delivery marker.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use x86_64::instructions::port::Port;

const DEBUG_PORT: u16 = 0xe9;
static TICKS_SEEN: AtomicU8 = AtomicU8::new(0);
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Emits `T\n` after ten timer IRQs have reached the kernel.
pub(crate) fn tick() {
    if TICKS_SEEN.fetch_add(1, Ordering::Relaxed) + 1 < 10 || REPORTED.swap(true, Ordering::Relaxed)
    {
        return;
    }

    // SAFETY: the smoke-test QEMU command line maps the debug console to 0xe9.
    unsafe {
        let mut port = Port::new(DEBUG_PORT);
        port.write(b'T');
        port.write(b'\n');
    }
}
