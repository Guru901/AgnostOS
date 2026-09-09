//! CPU exception and hardware IRQ handlers.

use core::sync::atomic::Ordering;

use x86_64::{
    registers::control::Cr2,
    structures::idt::{InterruptStackFrame, PageFaultErrorCode},
};

use crate::{TICKS, keyboard::{KeyboardScancode, push_keyboard_scancode}, kprintln};
#[cfg(feature = "mouse")]
use crate::mouse::{MouseByte, push_mouse_byte};

use super::{PS2_DATA, inb, pic};

fn halt() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

fn fatal_exception(name: &str, stack_frame: InterruptStackFrame) -> ! {
    kprintln!("\nEXCEPTION: {name}\n{stack_frame:#?}");
    halt()
}

fn fatal_exception_with_code(name: &str, stack_frame: InterruptStackFrame, error_code: u64) -> ! {
    kprintln!("\nEXCEPTION: {name} (error code: {error_code:#x})\n{stack_frame:#?}");
    halt()
}

macro_rules! exception_handler {
    ($handler:ident, $name:literal) => {
        pub(super) extern "x86-interrupt" fn $handler(stack_frame: InterruptStackFrame) {
            fatal_exception($name, stack_frame);
        }
    };
}

macro_rules! exception_handler_with_code {
    ($handler:ident, $name:literal) => {
        pub(super) extern "x86-interrupt" fn $handler(
            stack_frame: InterruptStackFrame,
            error_code: u64,
        ) {
            fatal_exception_with_code($name, stack_frame, error_code);
        }
    };
}

exception_handler!(divide_error, "divide error");
exception_handler!(debug, "debug");
exception_handler!(non_maskable_interrupt, "non-maskable interrupt");
exception_handler!(overflow, "overflow");
exception_handler!(bound_range_exceeded, "bound range exceeded");
exception_handler!(invalid_opcode, "invalid opcode");
exception_handler!(device_not_available, "device not available");
exception_handler!(x87_floating_point, "x87 floating point");
exception_handler!(simd_floating_point, "SIMD floating point");
exception_handler!(virtualization, "virtualization");
exception_handler!(hv_injection_exception, "hypervisor injection");

exception_handler_with_code!(invalid_tss, "invalid TSS");
exception_handler_with_code!(segment_not_present, "segment not present");
exception_handler_with_code!(stack_segment_fault, "stack segment fault");
exception_handler_with_code!(general_protection_fault, "general protection fault");
exception_handler_with_code!(alignment_check, "alignment check");
exception_handler_with_code!(cp_protection_exception, "control protection exception");
exception_handler_with_code!(vmm_communication_exception, "VMM communication exception");
exception_handler_with_code!(security_exception, "security exception");

pub(super) extern "x86-interrupt" fn breakpoint(stack_frame: InterruptStackFrame) {
    kprintln!("{stack_frame:#?}");
}

pub(super) extern "x86-interrupt" fn double_fault(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    // Logging or allocating here could immediately trigger a triple fault.
    halt()
}

pub(super) extern "x86-interrupt" fn page_fault(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let address = Cr2::read_raw();
    kprintln!(
        "\nEXCEPTION: page fault at {address:#x} ({error_code:?})\n{stack_frame:#?}"
    );
    halt()
}

pub(super) extern "x86-interrupt" fn machine_check(_stack_frame: InterruptStackFrame) -> ! {
    // Machine-check state may be unreliable, so avoid console access.
    halt()
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
