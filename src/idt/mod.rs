use core::sync::atomic::Ordering;

use spin::Once;
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

#[cfg(feature = "mouse")]
use crate::mouse::{MOUSE_QUEUE, initialize_controller};
use crate::{IDT_INITIALISED, kprintln};
use crate::{
    TICKS,
    keyboard::{KEYBOARD_QUEUE, Scancode},
};

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
            idt[TIMER_INTERRUPT_VECTOR].set_handler_fn(timer_interrupt_handler);
            #[cfg(feature = "mouse")]
            idt[MOUSE_INTERRUPT_VECTOR].set_handler_fn(mouse_interrupt_handler);

            idt
        })
        .load();

        PIC_INITIALIZED.call_once(|| {
            // SAFETY: interrupts are disabled while the legacy PIC is reprogrammed.
            unsafe {
                initialize_pic();
                initialize_pit();
            };
            #[cfg(feature = "mouse")]
            // SAFETY: interrupts remain disabled, so PS/2 controller setup cannot
            // race its IRQ handlers.
            unsafe {
                let _ = initialize_controller();
            };
        });
    });

    IDT_INITIALISED.store(true, Ordering::Relaxed);
}

const PIC1_OFFSET: u8 = 32;
const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;
const PIC2_OFFSET: u8 = 40;
const KEYBOARD_INTERRUPT_VECTOR: u8 = PIC1_OFFSET + 1;
const TIMER_INTERRUPT_VECTOR: u8 = PIC1_OFFSET;
#[cfg(feature = "mouse")]
const MOUSE_INTERRUPT_VECTOR: u8 = PIC2_OFFSET + 4;

const PIT_FREQUENCY: u32 = 1_193_182;
const TIMER_FREQUENCY: u32 = 1000;

unsafe fn initialize_pit() {
    let divisor = (PIT_FREQUENCY / TIMER_FREQUENCY) as u16;

    unsafe {
        outb(0x36, 0x43); // channel 0, lo/hi byte, mode 3

        outb((divisor & 0xff) as u8, 0x40);
        outb((divisor >> 8) as u8, 0x40);
    }
}

/// Maps hardware IRQs away from CPU exception vectors and unmasks IRQ1
/// (keyboard). With the `mouse` feature, it also unmasks IRQ2 (slave cascade)
/// and IRQ12 (PS/2 mouse).
///
/// All other IRQs remain masked because this kernel has no handlers for them.
unsafe fn initialize_pic() {
    const ICW1_INIT: u8 = 0x10;
    const ICW1_ICW4: u8 = 0x01;
    const ICW4_8086: u8 = 0x01;

    #[cfg(feature = "mouse")]
    const MASTER_IRQ_MASK: u8 = 0b1111_1000;

    #[cfg(not(feature = "mouse"))]
    const MASTER_IRQ_MASK: u8 = 0b1111_1100;

    #[cfg(feature = "mouse")]
    const SLAVE_IRQ_MASK: u8 = 0b1110_1111; // IRQ12 (bit 4 of slave) unmasked

    #[cfg(not(feature = "mouse"))]
    const SLAVE_IRQ_MASK: u8 = 0b1111_1111; // all slave IRQs masked

    // SAFETY: the caller has disabled interrupts, and these are the legacy PIC
    // command/data ports on the target x86 hardware.
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

        outb(MASTER_IRQ_MASK, PIC1_DATA);
        outb(SLAVE_IRQ_MASK, PIC2_DATA);
    }
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    kprintln!("{:#?}", stack_frame);
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);

    unsafe {
        outb(0x20, PIC1_COMMAND);
    }
}

extern "x86-interrupt" fn double_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    // A double fault may indicate that the stack, allocator, or console is no
    // longer usable. Avoid invoking the normal panic path and halt directly.
    loop {
        x86_64::instructions::hlt();
    }
}

/// # Safety
///
/// `port` must name a readable, initialized I/O device, and access must obey
/// that device's protocol.
pub(crate) unsafe fn inb(port: u16) -> u8 {
    let mut port = Port::new(port);
    // SAFETY: callers supply the legacy PS/2 controller ports (0x64 or 0x60),
    // which are available while this kernel runs on the target hardware.
    unsafe { port.read() }
}

/// # Safety
///
/// `port` and `value` must be valid for an initialized I/O device's current
/// protocol.
pub(crate) unsafe fn outb(value: u8, port: u16) {
    let mut port = Port::new(port);
    // SAFETY: callers supply a valid, initialized legacy device port and a value
    // valid for that device's current protocol.
    unsafe { port.write(value) }
}

pub const PS2_DATA: u16 = 0x60;
pub const PS2_COMMAND: u16 = 0x64; // same port, write = command, read = status
pub const PS2_STATUS: u16 = 0x64;

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // SAFETY: IRQ1 is dispatched only after PIC setup, and `0x60` is the PS/2
    // data port associated with this interrupt.
    let code = unsafe { inb(PS2_DATA) };

    x86_64::instructions::interrupts::without_interrupts(|| {
        KEYBOARD_QUEUE.lock().push(Scancode::new(code));
    });

    // SAFETY: PIC1 was initialized during `init`; `0x20` is its EOI command.
    unsafe {
        outb(0x20, PIC1_COMMAND); // EOI
    }
}

#[cfg(feature = "mouse")]
extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // SAFETY: IRQ12 is dispatched only after PS/2 mouse setup, and `0x60` is
    // the controller data port carrying the mouse byte.
    let byte = unsafe { inb(PS2_DATA) };

    x86_64::instructions::interrupts::without_interrupts(|| {
        MOUSE_QUEUE.lock().push(byte);
    });

    // SAFETY: both PICs were initialized during `init`; these are their EOI
    // commands, issued slave first as required for a cascaded IRQ.
    unsafe {
        outb(0x20, PIC2_COMMAND); // EOI to slave first
        outb(0x20, PIC1_COMMAND); // then EOI to master
    }
}
