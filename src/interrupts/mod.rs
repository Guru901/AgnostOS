//! x86 interrupt subsystem.
//!
//! This module owns the interrupt startup sequence. Architecture-specific
//! descriptor-table setup, legacy PIC/PIT programming, and IRQ handlers live
//! in their own modules so device drivers only depend on the small port-I/O
//! interface exposed here.

mod controller;
#[cfg(target_arch = "x86_64")]
mod gdt;
#[cfg(target_arch = "x86_64")]
mod handlers;
#[cfg(target_arch = "x86_64")]
mod idt;
mod pic;
#[cfg(target_arch = "x86_64")]
mod pit;

#[cfg(target_arch = "x86_64")]
use spin::Once;
#[cfg(target_arch = "x86_64")]
use x86_64::instructions::port::Port;

#[cfg(target_arch = "x86_64")]
static HARDWARE_INITIALIZED: Once<()> = Once::new();

/// Installs exception and IRQ handlers, then configures hardware while CPU
/// interrupts are disabled.
#[cfg(target_arch = "x86_64")]
pub fn init() {
    crate::platform::without_interrupts(|| {
        gdt::install();
        idt::install();
        HARDWARE_INITIALIZED.call_once(|| {
            // SAFETY: `without_interrupts` prevents IRQ handlers from racing
            // PIC, PIT, and PS/2 controller initialization.
            unsafe {
                controller::initialize();
                pit::initialize();
            }
            #[cfg(feature = "mouse")]
            // SAFETY: the mouse IRQ is still disabled during controller setup.
            unsafe {
                let _ = crate::mouse::initialize_controller();
            }
        });
        crate::keyboard::init_keyboard();
        #[cfg(feature = "mouse")]
        crate::mouse::init_mouse();

        // Discard controller responses left over from firmware/device setup
        // before enabling the keyboard IRQ. Otherwise QEMU can deliver a
        // stale 0xfa acknowledgement as the first keyboard byte.
        unsafe { drain_ps2_output() };
    });
    x86_64::instructions::interrupts::enable();
}

/// Drains stale bytes from the shared PS/2 output buffer during boot.
///
/// # Safety
///
/// Interrupts are disabled and the controller is not being accessed by a
/// concurrent device handler.
unsafe fn drain_ps2_output() {
    for _ in 0..32 {
        if unsafe { inb(PS2_STATUS) } & 0b01 == 0 {
            break;
        }
        unsafe { inb(PS2_DATA) };
    }
}

/// Enables runtime device IRQs after the post-UEFI input path is ready.
pub fn enable_runtime() {
    controller::enable_runtime();
}

/// Enables the mouse IRQ after the shell and its input queues are ready.
#[cfg(feature = "mouse")]
pub fn enable_mouse() {
    controller::enable_mouse();
}

/// Host builds never execute kernel code and therefore have no IDT to install.
#[cfg(not(target_arch = "x86_64"))]
pub fn init() {}

/// PS/2 controller data port.
pub(crate) const PS2_DATA: u16 = 0x60;
/// PS/2 controller command port (also its read-only status port).
pub(crate) const PS2_COMMAND: u16 = 0x64;
pub(crate) const PS2_STATUS: u16 = PS2_COMMAND;

/// Reads a byte from a legacy I/O port.
///
/// # Safety
///
/// `port` must be a readable initialized device port, used according to that
/// device's protocol.
pub(crate) unsafe fn inb(port: u16) -> u8 {
    #[cfg(target_arch = "x86_64")]
    {
        let mut port = Port::new(port);
        // SAFETY: upheld by this function's caller.
        unsafe { port.read() }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = port;
        0
    }
}

/// Writes a byte to a legacy I/O port.
///
/// # Safety
///
/// `port` and `value` must be valid for the device's current protocol.
pub(crate) unsafe fn outb(value: u8, port: u16) {
    #[cfg(target_arch = "x86_64")]
    {
        let mut port = Port::new(port);
        // SAFETY: upheld by this function's caller.
        unsafe { port.write(value) }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (value, port);
    }
}
