use spin::Once;
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

use crate::keyboard::KEYBOARD_QUEUE;
use crate::kprintln;

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init() {
    IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();

        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.double_fault.set_handler_fn(double_fault_handler);

        // IRQ1 after PIC remapping
        idt[33].set_handler_fn(keyboard_interrupt_handler);

        idt
    })
    .load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    kprintln!("{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    panic!("{:#?} {}", stack_frame, error_code);
}

unsafe fn inb(port: u16) -> u8 {
    let mut port = Port::new(port);
    // SAFETY: callers use the legacy PS/2 controller ports (0x64 and 0x60),
    // which are available while the kernel is running on the target hardware.
    unsafe { port.read() }
}

unsafe fn outb(value: u8, port: u16) {
    let mut port = Port::new(port);
    // SAFETY: callers use the legacy PS/2 controller ports (0x64 and 0x60),
    // which are available while the kernel is running on the target hardware.
    unsafe { port.write(value) }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let code = unsafe { inb(0x60) };

    x86_64::instructions::interrupts::without_interrupts(|| {
        KEYBOARD_QUEUE.lock().push(code);
    });

    unsafe {
        outb(0x20, 0x20); // EOI
    }
}
