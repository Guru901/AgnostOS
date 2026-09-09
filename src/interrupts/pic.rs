//! Legacy 8259 programmable interrupt controller configuration.

use super::outb;

const MASTER_OFFSET: u8 = 32;
const SLAVE_OFFSET: u8 = MASTER_OFFSET + 8;
const MASTER_COMMAND: u16 = 0x20;
const MASTER_DATA: u16 = 0x21;
const SLAVE_COMMAND: u16 = 0xa0;
const SLAVE_DATA: u16 = 0xa1;
const EOI: u8 = 0x20;

pub(super) const TIMER_VECTOR: u8 = MASTER_OFFSET;
pub(super) const KEYBOARD_VECTOR: u8 = MASTER_OFFSET + 1;
#[cfg(feature = "mouse")]
pub(super) const MOUSE_VECTOR: u8 = SLAVE_OFFSET + 4;

/// Remaps the PICs and unmasks only IRQs with installed handlers.
///
/// # Safety
///
/// CPU interrupts must be disabled and the target must provide legacy PICs.
pub(super) unsafe fn initialize() {
    const ICW1_INIT: u8 = 0x10;
    const ICW1_ICW4: u8 = 0x01;
    const ICW4_8086: u8 = 0x01;

    #[cfg(feature = "mouse")]
    const MASTER_IRQ_MASK: u8 = 0b1111_1000;
    #[cfg(not(feature = "mouse"))]
    const MASTER_IRQ_MASK: u8 = 0b1111_1100;
    #[cfg(feature = "mouse")]
    const SLAVE_IRQ_MASK: u8 = 0b1110_1111;
    #[cfg(not(feature = "mouse"))]
    const SLAVE_IRQ_MASK: u8 = 0b1111_1111;

    // SAFETY: upheld by `initialize`'s caller.
    unsafe {
        outb(ICW1_INIT | ICW1_ICW4, MASTER_COMMAND);
        outb(ICW1_INIT | ICW1_ICW4, SLAVE_COMMAND);
        outb(MASTER_OFFSET, MASTER_DATA);
        outb(SLAVE_OFFSET, SLAVE_DATA);
        outb(0x04, MASTER_DATA);
        outb(0x02, SLAVE_DATA);
        outb(ICW4_8086, MASTER_DATA);
        outb(ICW4_8086, SLAVE_DATA);
        outb(MASTER_IRQ_MASK, MASTER_DATA);
        outb(SLAVE_IRQ_MASK, SLAVE_DATA);
    }
}

pub(super) fn acknowledge_master() {
    // SAFETY: called only by an IRQ from the initialized master PIC.
    unsafe { outb(EOI, MASTER_COMMAND) }
}

#[cfg(feature = "mouse")]
pub(super) fn acknowledge_slave() {
    // SAFETY: called only by an IRQ delivered through the slave PIC.
    unsafe {
        outb(EOI, SLAVE_COMMAND);
        outb(EOI, MASTER_COMMAND);
    }
}
