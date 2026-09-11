//! Interrupt descriptor table construction and loading.

use spin::Once;
use x86_64::structures::idt::InterruptDescriptorTable;

use super::{
    controller::{self, Irq},
    gdt, handlers,
};

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub(super) fn install() {
    IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.divide_error.set_handler_fn(handlers::divide_error);
        idt.debug.set_handler_fn(handlers::debug);
        // SAFETY: `gdt::install` created this IST stack before IDT load.
        unsafe {
            idt.non_maskable_interrupt
                .set_handler_fn(handlers::non_maskable_interrupt)
                .set_stack_index(gdt::NMI_IST_INDEX);
        }
        idt.breakpoint.set_handler_fn(handlers::breakpoint);
        idt.overflow.set_handler_fn(handlers::overflow);
        idt.bound_range_exceeded
            .set_handler_fn(handlers::bound_range_exceeded);
        idt.invalid_opcode.set_handler_fn(handlers::invalid_opcode);
        idt.device_not_available
            .set_handler_fn(handlers::device_not_available);
        // SAFETY: `gdt::install` created this IST stack before IDT load.
        unsafe {
            idt.double_fault
                .set_handler_fn(handlers::double_fault)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.invalid_tss.set_handler_fn(handlers::invalid_tss);
        idt.segment_not_present
            .set_handler_fn(handlers::segment_not_present);
        idt.stack_segment_fault
            .set_handler_fn(handlers::stack_segment_fault);
        idt.general_protection_fault
            .set_handler_fn(handlers::general_protection_fault);
        idt.page_fault.set_handler_fn(handlers::page_fault);
        idt.x87_floating_point
            .set_handler_fn(handlers::x87_floating_point);
        idt.alignment_check
            .set_handler_fn(handlers::alignment_check);
        // SAFETY: `gdt::install` created this IST stack before IDT load.
        unsafe {
            idt.machine_check
                .set_handler_fn(handlers::machine_check)
                .set_stack_index(gdt::MACHINE_CHECK_IST_INDEX);
        }
        idt.simd_floating_point
            .set_handler_fn(handlers::simd_floating_point);
        idt.virtualization.set_handler_fn(handlers::virtualization);
        idt.cp_protection_exception
            .set_handler_fn(handlers::cp_protection_exception);
        idt.hv_injection_exception
            .set_handler_fn(handlers::hv_injection_exception);
        idt.vmm_communication_exception
            .set_handler_fn(handlers::vmm_communication_exception);
        idt.security_exception
            .set_handler_fn(handlers::security_exception);
        idt[controller::vector_for(Irq::Timer)].set_handler_fn(handlers::timer);
        idt[controller::vector_for(Irq::Keyboard)].set_handler_fn(handlers::keyboard);
        #[cfg(feature = "mouse")]
        idt[controller::vector_for(Irq::Mouse)].set_handler_fn(handlers::mouse);
        idt
    })
    .load();
}
