//! Minimal, opt-in trace point for the QEMU input smoke test.
//!
//! QEMU's `isa-debugcon` device receives bytes written to port `0xe9`.  Keep
//! this outside the framebuffer console: the test needs to observe input even
//! while the shell is redrawing a partially edited line.

use core::sync::atomic::{AtomicBool, Ordering};

use x86_64::instructions::port::Port;

const DEBUG_PORT: u16 = 0xe9;
static OVERFLOW_TRIGGERED: AtomicBool = AtomicBool::new(false);

// The Apple-hosted UEFI target used by the smoke test currently leaves this C
// runtime symbol unresolved in `uefi`.  Supplying it only in this opt-in test
// build keeps the production image's link surface unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(mut string: *const u16) -> usize {
    let mut length = 0;
    // SAFETY: as with C's `wcslen`, the caller supplies a valid,
    // NUL-terminated UTF-16 string.
    unsafe {
        while *string != 0 {
            length += 1;
            string = string.add(1);
        }
    }
    length
}

fn write_byte(byte: u8) {
    // SAFETY: `scripts/qemu-input-smoke.py` creates an `isa-debugcon` device
    // at this port. This module is only compiled for that test build.
    unsafe { Port::new(DEBUG_PORT).write(byte) };
}

fn write_hex(byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    write_byte(HEX[(byte >> 4) as usize]);
    write_byte(HEX[(byte & 0x0f) as usize]);
}

fn trace(prefix: u8, byte: u8) {
    write_byte(prefix);
    write_hex(byte);
    write_byte(b'\n');
}

pub(crate) fn ready() {
    write_byte(b'R');
    write_byte(b'\n');
}

pub(crate) fn keyboard_byte(byte: u8) {
    trace(b'K', byte);
}

pub(crate) fn keyboard_overflow() {
    write_byte(b'D');
    write_byte(b'\n');
}

/// Turns the first real `B` make code into a deterministic queue-overflow
/// probe. QEMU serializes PS/2 input through a one-byte controller buffer, so
/// even a large QMP event burst cannot naturally fill the guest's 256-byte
/// software ring before the shell drains it.
pub(crate) fn force_keyboard_overflow(byte: u8) -> bool {
    byte == 0x30 && !OVERFLOW_TRIGGERED.swap(true, Ordering::Relaxed)
}

#[cfg(feature = "mouse")]
pub(crate) fn mouse_byte(byte: u8) {
    trace(b'M', byte);
}
