use spin::Once;
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

use crate::keyboard::{KEYBOARD_QUEUE, Scancode};
use crate::kprintln;
#[cfg(feature = "mouse")]
use crate::mouse::{MOUSE_QUEUE, ps2_write_data};
#[cfg(feature = "mouse")]
use crate::mouse::{ps2_read_data, ps2_write_command};

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
            #[cfg(feature = "mouse")]
            idt[MOUSE_INTERRUPT_VECTOR].set_handler_fn(mouse_interrupt_handler);

            idt
        })
        .load();

        PIC_INITIALIZED.call_once(|| {
            // SAFETY: interrupts are disabled while the legacy PIC is reprogrammed.
            unsafe { initialize_pic() };
            #[cfg(feature = "mouse")]
            // SAFETY: interrupts remain disabled, so PS/2 controller setup cannot
            // race its IRQ handlers.
            unsafe {
                initialize_mouse()
            };
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
#[cfg(feature = "mouse")]
const MOUSE_INTERRUPT_VECTOR: u8 = PIC2_OFFSET + 4;
#[cfg(feature = "mouse")]
// Controller commands
const CMD_ENABLE_AUX: u8 = 0xa8; // enable the second PS/2 port (mouse)
#[cfg(feature = "mouse")]
const CMD_READ_CONFIG: u8 = 0x20;
#[cfg(feature = "mouse")]
const CMD_WRITE_CONFIG: u8 = 0x60;
#[cfg(feature = "mouse")]
const CMD_WRITE_TO_AUX: u8 = 0xd4; // "next byte on 0x60 goes to the mouse, not keyboard"

#[cfg(feature = "mouse")]
const MOUSE_ENABLE_PACKETS: u8 = 0xf4;
#[cfg(feature = "mouse")]
const MOUSE_ACK: u8 = 0xfa;

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
    const MASTER_IRQ_MASK: u8 = 0b1111_1001; // IRQ1 (keyboard) + IRQ2 (cascade) unmasked
    #[cfg(not(feature = "mouse"))]
    const MASTER_IRQ_MASK: u8 = 0b1111_1101; // IRQ1 (keyboard) unmasked
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

#[cfg(feature = "mouse")]
unsafe fn initialize_mouse() {
    // SAFETY: interrupts are disabled and this is the only PS/2 controller setup
    // path, so the command/data sequence cannot be interleaved.
    unsafe {
        // 1. Tell the controller to enable the auxiliary (mouse) port.
        ps2_write_command(CMD_ENABLE_AUX);

        // 2. Read the controller's config byte, set the bit that enables
        //    IRQ12 generation on mouse activity, write it back.
        ps2_write_command(CMD_READ_CONFIG);
        let mut config = ps2_read_data();
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
