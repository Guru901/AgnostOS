//! Programmable interval timer configuration.

use super::outb;

pub(crate) const PIT_FREQUENCY: u32 = 1_193_182;
pub(crate) const TIMER_FREQUENCY: u32 = 1_000;
const COMMAND_PORT: u16 = 0x43;
const CHANNEL_0_PORT: u16 = 0x40;
const CHANNEL_0_MODE_3: u8 = 0x36;

/// Programs PIT channel 0 for the kernel's millisecond timer tick.
///
/// # Safety
///
/// CPU interrupts must be disabled while the timer is reprogrammed.
pub(super) unsafe fn initialize() {
    let divisor = divisor();

    // SAFETY: upheld by `initialize`'s caller.
    unsafe {
        outb(CHANNEL_0_MODE_3, COMMAND_PORT);
        outb((divisor & 0xff) as u8, CHANNEL_0_PORT);
        outb((divisor >> 8) as u8, CHANNEL_0_PORT);
    }
}

/// Returns the programmed PIT divisor used by the kernel timer.
#[must_use]
pub(crate) const fn divisor() -> u16 {
    (PIT_FREQUENCY / TIMER_FREQUENCY) as u16
}
