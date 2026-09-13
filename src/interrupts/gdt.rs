//! GDT and task-state-segment setup for exception-safe kernel stacks.
//!
//! The first kernel runs on one CPU.  NMI, double-fault, and machine-check
//! entries use separate Interrupt Stack Table (IST) stacks so a damaged normal
//! stack cannot turn a recoverable diagnostic into a triple fault.

use spin::Once;
use x86_64::{
    VirtAddr,
    instructions::segmentation::{CS, DS, ES, FS, GS, SS, Segment},
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
};

pub(super) const NMI_IST_INDEX: u16 = 0;
pub(super) const DOUBLE_FAULT_IST_INDEX: u16 = 1;
pub(super) const MACHINE_CHECK_IST_INDEX: u16 = 2;

const IST_STACK_SIZE: usize = 32 * 1024;

#[repr(C, align(16))]
struct InterruptStack([u8; IST_STACK_SIZE]);

// These stacks are never exposed as references.  They are installed once into
// the TSS before interrupts are enabled, then used exclusively by the CPU.
static mut INTERRUPT_STACKS: [InterruptStack; 3] = [
    const { InterruptStack([0; IST_STACK_SIZE]) },
    const { InterruptStack([0; IST_STACK_SIZE]) },
    const { InterruptStack([0; IST_STACK_SIZE]) },
];

struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    tss: SegmentSelector,
}

static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

fn stack_top(index: usize) -> VirtAddr {
    debug_assert!(index < 3);
    // SAFETY: `index` is checked, the static allocation lives for the life of
    // the kernel, and this creates no reference to the mutable static.
    let stack = unsafe {
        core::ptr::addr_of!(INTERRUPT_STACKS)
            .cast::<InterruptStack>()
            .add(index)
    };
    VirtAddr::from_ptr(stack) + IST_STACK_SIZE as u64
}

/// Installs the GDT and loads a TSS whose IST entries point at static stacks.
///
/// # Safety model
///
/// This must run exactly once while interrupts are disabled, before loading
/// the IDT entries that name these IST indices.  The current kernel is
/// single-core; SMP needs one TSS and one set of stacks per CPU.
pub(super) fn install() {
    let tss = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[NMI_IST_INDEX as usize] = stack_top(0);
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_top(1);
        tss.interrupt_stack_table[MACHINE_CHECK_IST_INDEX as usize] = stack_top(2);
        tss
    });

    let gdt = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(tss));
        (gdt, Selectors { code, data, tss })
    });

    gdt.0.load();
    // SAFETY: the GDT was loaded immediately above and both selectors refer to
    // entries owned by it.  The TSS and its IST stacks are static.
    unsafe {
        CS::set_reg(gdt.1.code);
        // UEFI leaves its own data/stack selectors loaded.  Loading our GDT
        // does not change those registers, so replace all of them before an
        // interrupt can return through the old (now invalid) SS selector.
        DS::set_reg(gdt.1.data);
        ES::set_reg(gdt.1.data);
        FS::set_reg(gdt.1.data);
        GS::set_reg(gdt.1.data);
        SS::set_reg(gdt.1.data);
        x86_64::instructions::tables::load_tss(gdt.1.tss);
    }
}
