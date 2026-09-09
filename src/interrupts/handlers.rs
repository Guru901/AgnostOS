//! CPU exception and hardware IRQ handlers.

use core::sync::atomic::Ordering;

use x86_64::structures::idt::InterruptStackFrame;

use crate::{TICKS, keyboard::{KeyboardScancode, push_keyboard_scancode}, kprintln};
#[cfg(feature = "mouse")]
use crate::mouse::{MouseByte, push_mouse_byte};

use super::{PS2_DATA, inb, pic};

pub(super) extern "x86-interrupt" fn breakpoint(stack_frame: InterruptStackFrame) {
    kprintln!("{stack_frame:#?}");
}

pub(super) extern "x86-interrupt" fn double_fault(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

pub(super) extern "x86-interrupt" fn timer(_stack_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    pic::acknowledge_master();
}

pub(super) extern "x86-interrupt" fn keyboard(_stack_frame: InterruptStackFrame) {
    // SAFETY: IRQ1 owns the PS/2 data byte that triggered it.
    let code = unsafe { inb(PS2_DATA) };
    push_keyboard_scancode(KeyboardScancode::new(code));
    pic::acknowledge_master();
}

#[cfg(feature = "mouse")]
pub(super) extern "x86-interrupt" fn mouse(_stack_frame: InterruptStackFrame) {
    // SAFETY: IRQ12 owns the PS/2 data byte that triggered it.
    let byte = unsafe { inb(PS2_DATA) };
    push_mouse_byte(MouseByte::new(byte));
    pic::acknowledge_slave();
}
