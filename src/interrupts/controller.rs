//! Interrupt-controller boundary.
//!
//! The legacy PIC is the first implementation.  IDT setup and IRQ handlers use
//! this module rather than PIC ports directly, so APIC/IOAPIC selection can be
//! added without changing device handlers.

use super::pic::LegacyPic;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Irq {
    Timer,
    Keyboard,
    #[cfg(feature = "mouse")]
    Mouse,
}

pub(super) trait InterruptController {
    /// Programs the controller while CPU interrupts are disabled.
    unsafe fn initialize(&self);
    fn vector_for(&self, irq: Irq) -> u8;
    fn acknowledge(&self, irq: Irq);
    fn enable_runtime(&self);
    #[cfg(feature = "mouse")]
    fn enable_mouse(&self);
}

static CONTROLLER: LegacyPic = LegacyPic;

/// Initializes the controller selected for this platform.
///
/// Platform discovery is deliberately kept here.  It currently selects the
/// legacy PIC because the kernel has not yet parsed ACPI MADT data.
pub(super) unsafe fn initialize() {
    // SAFETY: required by the caller contract of this wrapper.
    unsafe { CONTROLLER.initialize() };
}

pub(super) fn vector_for(irq: Irq) -> u8 {
    CONTROLLER.vector_for(irq)
}

pub(super) fn acknowledge(irq: Irq) {
    CONTROLLER.acknowledge(irq);
}

pub(super) fn enable_runtime() {
    CONTROLLER.enable_runtime();
}

#[cfg(feature = "mouse")]
pub(super) fn enable_mouse() {
    CONTROLLER.enable_mouse();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_vectors_are_stable_for_installed_irqs() {
        assert_eq!(vector_for(Irq::Timer), 32);
        assert_eq!(vector_for(Irq::Keyboard), 33);
        #[cfg(feature = "mouse")]
        assert_eq!(vector_for(Irq::Mouse), 44);
    }
}
