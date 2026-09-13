//! Opt-in invalid-opcode smoke probe for the real exception path.
//!
//! This is intentionally unavailable in normal builds: it exists only to let
//! QEMU assert that the IDT/GDT/TSS path reaches the fatal-exception handler.

use x86_64::instructions::port::Port;

const DEBUG_PORT: u16 = 0xe9;

pub(crate) fn trigger() -> ! {
    // SAFETY: `ud2` is architecturally guaranteed to raise invalid opcode.
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}

pub(crate) fn exception_entered() {
    // SAFETY: the fault smoke runner provides this QEMU debug console.  The
    // write is allocation-free and independent of the framebuffer console.
    unsafe {
        let mut port = Port::new(DEBUG_PORT);
        port.write(b'F');
        port.write(b'\n');
    }
}
