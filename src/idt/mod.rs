use spin::Once;
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

use crate::{
    keyboard::KEYBOARD_QUEUE,
    mouse::{MOUSE_QUEUE, ps2_write_data},
};
use crate::{
    kprintln,
    mouse::{ps2_read_data, ps2_write_command},
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
            idt[MOUSE_INTERRUPT_VECTOR].set_handler_fn(mouse_interrupt_handler);

            idt
        })
        .load();

        PIC_INITIALIZED.call_once(|| {
            // SAFETY: interrupts are disabled while the legacy PIC is reprogrammed.
            unsafe { initialize_pic() };
            unsafe { initialize_mouse() };
        });
    });
}

const PIC1_OFFSET: u8 = 32;
const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;
const PIC2_OFFSET: u8 = 40;
const KEYBOARD_INTERRUPT_VECTOR: u8 = PIC1_OFFSET + 1;
const MOUSE_INTERRUPT_VECTOR: u8 = PIC2_OFFSET + 4;
// Controller commands
const CMD_ENABLE_AUX: u8 = 0xa8; // enable the second PS/2 port (mouse)
const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_WRITE_TO_AUX: u8 = 0xd4; // "next byte on 0x60 goes to the mouse, not keyboard"

const MOUSE_ENABLE_PACKETS: u8 = 0xf4;
const MOUSE_ACK: u8 = 0xfa;

/// Maps hardware IRQs away from CPU exception vectors and unmasks IRQ1
/// (keyboard), IRQ2 (slave cascade), and IRQ12 (PS/2 mouse).
///
/// All other IRQs remain masked because this kernel has no handlers for them.
unsafe fn initialize_pic() {
    const ICW1_INIT: u8 = 0x10;
    const ICW1_ICW4: u8 = 0x01;
    const ICW4_8086: u8 = 0x01;
    const MASTER_KEYBOARD_MOUSE: u8 = 0b1111_1001; // IRQ1 (keyboard) + IRQ2 (cascade) unmasked
    const SLAVE_MOUSE_ONLY: u8 = 0b1110_1111; // IRQ12 (bit 4 of slave) unmasked

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

        outb(MASTER_KEYBOARD_MOUSE, PIC1_DATA);
        outb(SLAVE_MOUSE_ONLY, PIC2_DATA);
    }
}

unsafe fn initialize_mouse() {
    unsafe {
        // 1. Tell the controller to enable the auxiliary (mouse) port.
        ps2_write_command(CMD_ENABLE_AUX);

        // 2. Read the controller's config byte, set the bit that enables
        //    IRQ12 generation on mouse activity, write it back.
        ps2_write_command(CMD_READ_CONFIG);
        let mut config = ps2_read_data();
        // **I DONT KNOW WHAT THIS DOES**
        config |= 0b0000_0010; // bit 1 = enable IRQ12 (aux interrupt)
        config &= !0b0010_0000; // bit 5 = disable aux clock masking
        ps2_write_command(CMD_WRITE_CONFIG);
        ps2_write_data(config);

        // 3. Tell the mouse itself to start sending movement packets.
        ps2_write_command(CMD_WRITE_TO_AUX);
        ps2_write_data(MOUSE_ENABLE_PACKETS);
        let ack = ps2_read_data();
        debug_assert_eq!(ack, MOUSE_ACK, "mouse did not ack enable-packets command");
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

pub(crate) unsafe fn inb(port: u16) -> u8 {
    let mut port = Port::new(port);
    // SAFETY: callers use the legacy PS/2 controller ports (0x64 and 0x60),
    // which are available while the kernel is running on the target hardware.
    unsafe { port.read() }
}

pub(crate) unsafe fn outb(value: u8, port: u16) {
    let mut port = Port::new(port);
    // SAFETY: callers use the legacy PS/2 controller ports (0x64 and 0x60),
    // which are available while the kernel is running on the target hardware.
    unsafe { port.write(value) }
}

pub const PS2_DATA: u16 = 0x60;
pub const PS2_COMMAND: u16 = 0x64; // same port, write = command, read = status
pub const PS2_STATUS: u16 = 0x64;

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let code = unsafe { inb(PS2_DATA) };

    x86_64::instructions::interrupts::without_interrupts(|| {
        KEYBOARD_QUEUE.lock().push(code);
    });

    unsafe {
        outb(0x20, PIC1_COMMAND); // EOI
    }
}

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let byte = unsafe { inb(PS2_DATA) };

    x86_64::instructions::interrupts::without_interrupts(|| {
        MOUSE_QUEUE.lock().push(byte);
    });

    unsafe {
        outb(0x20, PIC2_COMMAND); // EOI to slave first
        outb(0x20, PIC1_COMMAND); // then EOI to master
    }
}
