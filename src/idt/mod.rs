use spin::Once;
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

use crate::keyboard::KEYBOARD_QUEUE;
use crate::kprintln;

static IDT: Once<InterruptDescriptorTable> = Once::new();
static PIC_INITIALIZED: Once<()> = Once::new();

pub fn init() {
    x86_64::instructions::interrupts::without_interrupts(|| {
        IDT.call_once(|| {
            let mut idt = InterruptDescriptorTable::new();

            idt.breakpoint.set_handler_fn(breakpoint_handler);
            idt.double_fault.set_handler_fn(double_fault_handler);

            // IRQ1 after PIC remapping.
            idt[KEYBOARD_INTERRUPT_VECTOR].set_handler_fn(keyboard_interrupt_handler);

            idt
        })
        .load();

        PIC_INITIALIZED.call_once(|| {
            // SAFETY: interrupts are disabled while the legacy PIC is reprogrammed.
            unsafe { initialize_pic() };
        });
    });
}

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;
const PIC1_OFFSET: u8 = 32;
const PIC2_OFFSET: u8 = 40;
const KEYBOARD_INTERRUPT_VECTOR: u8 = PIC1_OFFSET + 1;

/// Maps hardware IRQs away from CPU exception vectors and enables only IRQ1.
///
/// All other IRQs remain masked because this kernel has no handlers for them.
unsafe fn initialize_pic() {
    const ICW1_INIT: u8 = 0x10;
    const ICW1_ICW4: u8 = 0x01;
    const ICW4_8086: u8 = 0x01;
    const MASTER_KEYBOARD_ONLY: u8 = 0b1111_1101;
    const MASK_ALL: u8 = 0xff;

    // Start initialization in cascade mode.
    unsafe {
        outb(ICW1_INIT | ICW1_ICW4, PIC1_COMMAND);
        outb(ICW1_INIT | ICW1_ICW4, PIC2_COMMAND);

        // Set vector offsets.
        outb(PIC1_OFFSET, PIC1_DATA);
        outb(PIC2_OFFSET, PIC2_DATA);

        // Tell the master and slave about their cascade wiring.
        outb(0x04, PIC1_DATA);
        outb(0x02, PIC2_DATA);

        // Use 8086 mode.
        outb(ICW4_8086, PIC1_DATA);
        outb(ICW4_8086, PIC2_DATA);

        // This kernel only handles the keyboard on master IRQ1.
        outb(MASTER_KEYBOARD_ONLY, PIC1_DATA);
        outb(MASK_ALL, PIC2_DATA);
    }
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
        outb(0x20, PIC1_COMMAND); // EOI
    }
}
