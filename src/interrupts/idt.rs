//! Interrupt descriptor table construction and loading.

use spin::Once;
use x86_64::structures::idt::InterruptDescriptorTable;

use super::{handlers, pic};

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub(super) fn install() {
    IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(handlers::breakpoint);
        idt.double_fault.set_handler_fn(handlers::double_fault);
        idt[pic::TIMER_VECTOR].set_handler_fn(handlers::timer);
        idt[pic::KEYBOARD_VECTOR].set_handler_fn(handlers::keyboard);
        #[cfg(feature = "mouse")]
        idt[pic::MOUSE_VECTOR].set_handler_fn(handlers::mouse);
        idt
    })
    .load();
}
