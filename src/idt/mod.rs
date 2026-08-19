#[cfg(target_arch = "x86_64")]
use core::sync::atomic::Ordering;

#[cfg(target_arch = "x86_64")]
use spin::Once;
#[cfg(target_arch = "x86_64")]
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

#[cfg(target_arch = "x86_64")]
use crate::kprintln;
#[cfg(all(target_arch = "x86_64", feature = "mouse"))]
use crate::mouse::{MOUSE_QUEUE, initialize_controller};
#[cfg(target_arch = "x86_64")]
use crate::{
    TICKS,
    keyboard::{KEYBOARD_QUEUE, Scancode},
};

#[cfg(target_arch = "x86_64")]
static IDT: Once<InterruptDescriptorTable> = Once::new();
#[cfg(target_arch = "x86_64")]
static HARDWARE_INITIALIZED: Once<()> = Once::new();

/// Installs exception and IRQ handlers, then configures the legacy interrupt
/// controllers while interrupts are disabled.
#[cfg(target_arch = "x86_64")]
pub fn init() {
    crate::platform::without_interrupts(|| {
        install_idt();
        initialize_hardware();
    });
    x86_64::instructions::interrupts::enable();
}

/// Host builds do not install an IDT because they never execute kernel code.
#[cfg(not(target_arch = "x86_64"))]
pub fn init() {}

#[cfg(target_arch = "x86_64")]
const MASTER_PIC_OFFSET: u8 = 32;
#[cfg(target_arch = "x86_64")]
const SLAVE_PIC_OFFSET: u8 = MASTER_PIC_OFFSET + 8;
#[cfg(target_arch = "x86_64")]
const PIC1_COMMAND: u16 = 0x20;
#[cfg(target_arch = "x86_64")]
const PIC1_DATA: u16 = 0x21;
#[cfg(target_arch = "x86_64")]
const PIC2_COMMAND: u16 = 0xa0;
#[cfg(target_arch = "x86_64")]
const PIC2_DATA: u16 = 0xa1;
#[cfg(target_arch = "x86_64")]
const TIMER_INTERRUPT_VECTOR: u8 = MASTER_PIC_OFFSET;
#[cfg(target_arch = "x86_64")]
const KEYBOARD_INTERRUPT_VECTOR: u8 = MASTER_PIC_OFFSET + 1;
#[cfg(all(target_arch = "x86_64", feature = "mouse"))]
const MOUSE_INTERRUPT_VECTOR: u8 = SLAVE_PIC_OFFSET + 4;

#[cfg(target_arch = "x86_64")]
const PIT_FREQUENCY: u32 = 1_193_182;
#[cfg(target_arch = "x86_64")]
const TIMER_FREQUENCY: u32 = 1000;
#[cfg(target_arch = "x86_64")]
const MASTER_EOI: u8 = 0x20;

#[cfg(target_arch = "x86_64")]
fn install_idt() {
    IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.double_fault.set_handler_fn(double_fault_handler);
        idt[TIMER_INTERRUPT_VECTOR].set_handler_fn(timer_interrupt_handler);
        idt[KEYBOARD_INTERRUPT_VECTOR].set_handler_fn(keyboard_interrupt_handler);
        #[cfg(feature = "mouse")]
        idt[MOUSE_INTERRUPT_VECTOR].set_handler_fn(mouse_interrupt_handler);
        idt
    })
    .load();
}

#[cfg(target_arch = "x86_64")]
fn initialize_hardware() {
    HARDWARE_INITIALIZED.call_once(|| {
        // SAFETY: interrupts are disabled by `init` while the PIC and PIT are configured.
        unsafe {
            initialize_pic();
            initialize_pit();
        }
        #[cfg(feature = "mouse")]
        // SAFETY: PS/2 setup must not race the mouse IRQ handler.
        unsafe {
            let _ = initialize_controller();
        }
    });
}

#[cfg(target_arch = "x86_64")]
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
#[cfg(target_arch = "x86_64")]
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
        outb(MASTER_PIC_OFFSET, PIC1_DATA);
        outb(SLAVE_PIC_OFFSET, PIC2_DATA);

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

#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    kprintln!("{:#?}", stack_frame);
}

#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    acknowledge_master_pic();
}

#[cfg(target_arch = "x86_64")]
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
    #[cfg(target_arch = "x86_64")]
    {
        let mut port = Port::new(port);
        // SAFETY: callers supply the legacy PS/2 controller ports (0x64 or 0x60).
        unsafe { port.read() }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = port;
        0
    }
}

/// # Safety
///
/// `port` and `value` must be valid for an initialized I/O device's current
/// protocol.
pub(crate) unsafe fn outb(value: u8, port: u16) {
    #[cfg(target_arch = "x86_64")]
    {
        let mut port = Port::new(port);
        // SAFETY: callers supply a valid, initialized legacy device port and value.
        unsafe { port.write(value) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (value, port);
    }
}

#[cfg(target_arch = "x86_64")]
fn acknowledge_master_pic() {
    // SAFETY: every caller is an IRQ delivered by the initialized master PIC.
    unsafe { outb(MASTER_EOI, PIC1_COMMAND) }
}

#[cfg(all(target_arch = "x86_64", feature = "mouse"))]
fn acknowledge_slave_pic() {
    // SAFETY: every caller is an IRQ delivered through the initialized slave PIC.
    unsafe {
        outb(MASTER_EOI, PIC2_COMMAND);
        outb(MASTER_EOI, PIC1_COMMAND);
    }
}

pub const PS2_DATA: u16 = 0x60;
pub const PS2_COMMAND: u16 = 0x64; // same port, write = command, read = status
pub const PS2_STATUS: u16 = 0x64;

#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // SAFETY: IRQ1 is dispatched only after PIC setup, and `0x60` is the PS/2
    // data port associated with this interrupt.
    let code = unsafe { inb(PS2_DATA) };

    crate::platform::without_interrupts(|| {
        KEYBOARD_QUEUE.lock().push(Scancode::new(code));
    });

    acknowledge_master_pic();
}

#[cfg(all(target_arch = "x86_64", feature = "mouse"))]
extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // SAFETY: IRQ12 is dispatched only after PS/2 mouse setup, and `0x60` is
    // the controller data port carrying the mouse byte.
    let byte = unsafe { inb(PS2_DATA) };

    x86_64::instructions::interrupts::without_interrupts(|| {
        MOUSE_QUEUE.lock().push(byte);
    });

    acknowledge_slave_pic();
}
