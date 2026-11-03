//! PIT based guest stack sampling for QEMU profiling runs.
//!
//! Samples are collected automatically after the IDT/PIT is installed. The
//! bounded buffer is emitted to the QEMU debug console by the `shutdown`
//! command, keeping I/O out of the timer interrupt.

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use x86_64::structures::idt::InterruptStackFrame;

const SAMPLE_EVERY_TICKS: u8 = 10;
const SAMPLE_CAPACITY: usize = 32_768;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy)]
struct Sample {
    depth: u8,
    ips: [u64; MAX_DEPTH],
}

impl Sample {
    const EMPTY: Self = Self {
        depth: 0,
        ips: [0; MAX_DEPTH],
    };
}

struct SampleBuffer(UnsafeCell<[Sample; SAMPLE_CAPACITY]>);

// The kernel is currently single-core, and only the PIT handler writes samples.
unsafe impl Sync for SampleBuffer {}

static SAMPLES: SampleBuffer = SampleBuffer(UnsafeCell::new([Sample::EMPTY; SAMPLE_CAPACITY]));
static SAMPLE_COUNT: AtomicUsize = AtomicUsize::new(0);
static TICK_DIVIDER: AtomicU8 = AtomicU8::new(0);
static IMAGE_BASE: AtomicU64 = AtomicU64::new(0);
static CODE_START: AtomicU64 = AtomicU64::new(0);
static CODE_END: AtomicU64 = AtomicU64::new(0);
static UNWIND_TABLE_RVA: AtomicU32 = AtomicU32::new(0);
static UNWIND_TABLE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Writes the loaded image base so host-side symbol resolution can account for
/// UEFI relocation of the PE image.
#[inline(never)]
pub(crate) fn initialize(image_base: usize, image_size: usize) {
    use crate::interrupts::outb;

    IMAGE_BASE.store(image_base as u64, Ordering::Relaxed);
    // Locate the PE exception directory. UEFI images use the Windows x64
    // RUNTIME_FUNCTION / UNWIND_INFO format for stack unwinding.
    // SAFETY: `image_base` and `image_size` came from UEFI's LoadedImage
    // protocol; the PE headers and data directory lie within that image.
    unsafe {
        let base = image_base as *const u8;
        let pe_offset = read_u32(base.add(0x3c)) as usize;
        let optional = base.add(pe_offset + 24);
        let magic = read_u16(optional);
        let directory_offset = match magic {
            0x20b => 112, // PE32+
            _ => 96,      // PE32
        };
        let exception = optional.add(directory_offset + 3 * 8);
        let rva = read_u32(exception);
        let size = read_u32(exception.add(4)) as usize;
        if rva != 0
            && size >= 12
            && rva as usize <= image_size
            && (rva as usize)
                .checked_add(size)
                .is_some_and(|end| end <= image_size)
        {
            UNWIND_TABLE_RVA.store(rva, Ordering::Relaxed);
            UNWIND_TABLE_COUNT.store(size / 12, Ordering::Relaxed);
        }

        let file_header = base.add(pe_offset + 4);
        let section_count = read_u16(file_header.add(2)) as usize;
        let optional_size = read_u16(file_header.add(16)) as usize;
        let sections = optional.add(optional_size);
        for index in 0..section_count {
            let section = sections.add(index * 40);
            if core::slice::from_raw_parts(section, 5) == b".text" {
                let virtual_size = read_u32(section.add(8)) as u64;
                let virtual_address = read_u32(section.add(12)) as u64;
                let start = (image_base as u64).saturating_add(virtual_address);
                let end = start.saturating_add(virtual_size);
                if start >= image_base as u64 && end <= image_base as u64 + image_size as u64 {
                    CODE_START.store(start, Ordering::Relaxed);
                    CODE_END.store(end, Ordering::Relaxed);
                }
                break;
            }
        }
    }
    for byte in b"AGNOSTOS_PROFILE_BASE=" {
        // SAFETY: Port 0xe9 is the QEMU debug console configured by profile.sh.
        unsafe { outb(*byte, 0xe9) };
    }
    write_address(image_base);
    // SAFETY: Port 0xe9 is the QEMU debug console configured by profile.sh.
    unsafe { outb(b'\n', 0xe9) };

    for byte in b"AGNOSTOS_PROFILE_ANCHOR=" {
        // SAFETY: Port 0xe9 is the QEMU debug console configured by profile.sh.
        unsafe { outb(*byte, 0xe9) };
    }
    write_address(initialize as *const () as usize);
    // SAFETY: Port 0xe9 is the QEMU debug console configured by profile.sh.
    unsafe { outb(b'\n', 0xe9) };
}

fn write_address(address: usize) {
    use crate::interrupts::outb;
    let mut shift = 60;
    let mut started = false;
    loop {
        let nibble = ((address as u64 >> shift) & 0xf) as usize;
        if nibble != 0 || started || shift == 0 {
            started = true;
            // SAFETY: Port 0xe9 is the QEMU debug console configured by profile.sh.
            unsafe { outb(b"0123456789abcdef"[nibble], 0xe9) };
        }
        if shift == 0 {
            break;
        }
        shift -= 4;
    }
}

/// Records a bounded stack sample on every tenth 1 ms timer tick.
#[inline(never)]
pub(crate) fn sample(frame: &InterruptStackFrame) {
    if TICK_DIVIDER.fetch_add(1, Ordering::Relaxed) + 1 < SAMPLE_EVERY_TICKS {
        return;
    }
    TICK_DIVIDER.store(0, Ordering::Relaxed);

    let index = SAMPLE_COUNT.load(Ordering::Relaxed);
    if index >= SAMPLE_CAPACITY {
        return;
    }

    let mut sample = Sample::EMPTY;
    sample.ips[0] = frame.instruction_pointer.as_u64();
    sample.depth = 1;

    let rsp: u64;
    // SAFETY: Reading RSP is observational and does not modify machine state.
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags));
    }

    let mut callers = [0_u64; MAX_DEPTH - 1];
    let depth = unwind_sample(
        sample.ips[0],
        rsp,
        read_register_rbp(),
        frame.stack_pointer.as_u64(),
        &mut callers,
    );
    for caller in callers.iter().take(depth) {
        sample.ips[sample.depth as usize] = *caller;
        sample.depth += 1;
    }

    // SAFETY: The PIT interrupt is the sole writer and writes a unique slot;
    // shutdown disables interrupts before reading the buffer.
    unsafe { (*SAMPLES.0.get())[index] = sample };
    SAMPLE_COUNT.store(index + 1, Ordering::Relaxed);
}

#[inline(always)]
fn read_register_rbp() -> u64 {
    let rbp: u64;
    // SAFETY: Reading RBP is observational and does not modify machine state.
    unsafe {
        core::arch::asm!("mov {}, rbp", out(reg) rbp, options(nomem, nostack, preserves_flags));
    }
    rbp
}

#[derive(Clone, Copy)]
struct UnwindStep {
    return_ip: u64,
    caller_rsp: u64,
    caller_rbp: u64,
}

/// Walks from the profiler through its timer handler, then unwinds the
/// interrupted kernel stack using the PE unwind records emitted by rustc.
fn unwind_sample(
    interrupted_ip: u64,
    current_rsp: u64,
    current_rbp: u64,
    interrupted_rsp: u64,
    output: &mut [u64],
) -> usize {
    let code_start = CODE_START.load(Ordering::Relaxed);
    let code_end = CODE_END.load(Ordering::Relaxed);
    let stack_limit = current_rsp.saturating_add(64 * 1024);
    if interrupted_rsp < current_rsp || interrupted_rsp > stack_limit {
        return 0;
    }

    let mut pc = sample as *const () as u64;
    let mut rsp = current_rsp;
    let mut rbp = current_rbp;

    // First unwind the profiler and timer handler frames. They are not part of
    // the sampled workload, so keep stepping until the return address reaches
    // the CPU-saved instruction pointer.
    for _ in 0..MAX_DEPTH {
        let Some(step) = unwind_one(pc, rsp, rbp, stack_limit, true) else {
            return 0;
        };
        if step.return_ip == interrupted_ip {
            rsp = interrupted_rsp;
            pc = interrupted_ip;
            rbp = step.caller_rbp;
            break;
        }
        if step.return_ip < code_start || step.return_ip >= code_end || step.caller_rsp <= rsp {
            return 0;
        }
        pc = step.return_ip - 1;
        rsp = step.caller_rsp;
        rbp = step.caller_rbp;
    }

    if pc != interrupted_ip {
        return 0;
    }

    let mut count = 0;
    for _ in 0..output.len() {
        let Some(step) = unwind_one(pc, rsp, rbp, stack_limit, false) else {
            break;
        };
        if step.return_ip < code_start || step.return_ip >= code_end || step.caller_rsp <= rsp {
            break;
        }
        output[count] = step.return_ip;
        count += 1;
        pc = step.return_ip - 1;
        rsp = step.caller_rsp;
        rbp = step.caller_rbp;
    }
    count
}

/// Applies one Windows x64 unwind recipe. A function with no runtime-function
/// record is a leaf, whose return address is at the current RSP.
fn unwind_one(pc: u64, rsp: u64, rbp: u64, stack_limit: u64, body: bool) -> Option<UnwindStep> {
    let base = IMAGE_BASE.load(Ordering::Relaxed);
    let code_start = CODE_START.load(Ordering::Relaxed);
    let code_end = CODE_END.load(Ordering::Relaxed);
    if pc < code_start || pc >= code_end {
        return None;
    }
    let rva = u32::try_from(pc.checked_sub(base)?).ok()?;
    let table = UNWIND_TABLE_RVA.load(Ordering::Relaxed) as u64;
    let count = UNWIND_TABLE_COUNT.load(Ordering::Relaxed);
    let mut low = 0usize;
    let mut high = count;
    let mut record = None;
    while low < high {
        let mid = low + (high - low) / 2;
        // SAFETY: the exception directory was checked against the PE image size.
        let entry = (base + table + (mid * 12) as u64) as *const u8;
        let begin = unsafe { read_u32(entry) };
        let end = unsafe { read_u32(entry.add(4)) };
        if rva < begin {
            high = mid;
        } else if rva >= end {
            low = mid + 1;
        } else {
            record = Some((begin, end, unsafe { read_u32(entry.add(8)) }));
            break;
        }
    }

    let (mut caller_rsp, mut caller_rbp) = (rsp, rbp);
    if let Some((begin, end, unwind_rva)) = record {
        // Windows x64 epilogues restore the caller state in the instruction
        // stream, rather than through the prolog unwind codes. We cannot
        // safely simulate every supported epilogue instruction here, so drop
        // samples whose remaining instruction stream matches an epilogue.
        let function_end = base.checked_add(end as u64)?;
        if is_epilogue(pc, function_end) {
            return None;
        }
        // SAFETY: UnwindData is an RVA within the loaded PE image.
        let info = (base + unwind_rva as u64) as *const u8;
        let version_flags = unsafe { *info };
        let version = version_flags & 0x7;
        let flags = version_flags >> 3;
        if version == 0 || version > 2 || flags & 0x4 != 0 {
            return None;
        }
        let prolog_size = unsafe { *info.add(1) } as u32;
        let code_count = unsafe { *info.add(2) } as usize;
        let frame = unsafe { *info.add(3) };
        let frame_register = frame & 0x0f;
        let frame_offset = (frame >> 4) as u64 * 16;
        let offset = rva.saturating_sub(begin);
        let prolog_offset = if body || offset >= prolog_size {
            u32::MAX
        } else {
            offset
        };
        let codes = unsafe { info.add(4) };
        let mut index = 0usize;

        while index < code_count {
            let code_offset = unsafe { *codes.add(index * 2) } as u32;
            let operation = unsafe { *codes.add(index * 2 + 1) };
            let op = operation & 0x0f;
            let op_info = operation >> 4;
            let active = prolog_offset >= code_offset;
            let mut slots = 1usize;
            match op {
                0 => {
                    // UWOP_PUSH_NONVOL
                    if active {
                        let value = unsafe { read_u64(caller_rsp as *const u8) };
                        if op_info == 5 {
                            caller_rbp = value;
                        }
                        caller_rsp = caller_rsp.checked_add(8)?;
                    }
                }
                1 => {
                    // UWOP_ALLOC_LARGE
                    let size = if op_info == 0 {
                        slots = 2;
                        unsafe { read_u16(codes.add((index + 1) * 2)) as u64 * 8 }
                    } else if op_info == 1 {
                        slots = 3;
                        let low = unsafe { read_u16(codes.add((index + 1) * 2)) } as u64;
                        let high = unsafe { read_u16(codes.add((index + 2) * 2)) } as u64;
                        low | (high << 16)
                    } else {
                        return None;
                    };
                    if active {
                        caller_rsp = caller_rsp.checked_add(size)?;
                    }
                }
                2 => {
                    // UWOP_ALLOC_SMALL
                    if active {
                        caller_rsp = caller_rsp.checked_add(op_info as u64 * 8 + 8)?;
                    }
                }
                3 => {
                    // UWOP_SET_FPREG
                    if active {
                        if frame_register != 5 {
                            return None;
                        }
                        caller_rsp = caller_rbp.checked_sub(frame_offset)?;
                    }
                }
                4 => {
                    // UWOP_SAVE_NONVOL
                    slots = 2;
                    if active && op_info == 5 {
                        let offset = unsafe { read_u16(codes.add((index + 1) * 2)) as u64 * 8 };
                        caller_rbp = unsafe { read_u64((caller_rsp + offset) as *const u8) };
                    }
                }
                5 => {
                    // UWOP_SAVE_NONVOL_FAR
                    slots = 3;
                    if active && op_info == 5 {
                        let low = unsafe { read_u16(codes.add((index + 1) * 2)) } as u64;
                        let high = unsafe { read_u16(codes.add((index + 2) * 2)) } as u64;
                        caller_rbp =
                            unsafe { read_u64((caller_rsp + low + (high << 16)) as *const u8) };
                    }
                }
                8 | 9 => {
                    // UWOP_SAVE_XMM128 / _FAR
                    slots = if op == 8 { 2 } else { 3 };
                }
                10 => {
                    // UWOP_PUSH_MACHFRAME
                    if active {
                        caller_rsp = caller_rsp.checked_add(if op_info == 0 { 40 } else { 48 })?;
                    }
                }
                _ => return None,
            }
            index += slots;
        }
    }

    if caller_rsp < rsp || caller_rsp > stack_limit.saturating_sub(8) {
        return None;
    }
    let return_ip = unsafe { read_u64(caller_rsp as *const u8) };
    Some(UnwindStep {
        return_ip,
        caller_rsp: caller_rsp.checked_add(8)?,
        caller_rbp,
    })
}

fn is_epilogue(pc: u64, function_end: u64) -> bool {
    let length = function_end.saturating_sub(pc).min(64) as usize;
    if length == 0 {
        return false;
    }
    // SAFETY: `pc` is inside the loaded .text section and the runtime-function
    // end bounds this read to the current function.
    let code = unsafe { core::slice::from_raw_parts(pc as *const u8, length) };
    let mut index = 0;

    // An epilogue may begin by releasing its fixed stack allocation.
    if code.get(index..index + 3) == Some(&[0x48, 0x83, 0xc4]) {
        index += 4; // add rsp, imm8
    } else if code.get(index..index + 3) == Some(&[0x48, 0x81, 0xc4]) {
        index += 7; // add rsp, imm32
    } else if matches!(code.get(index), Some(0x48..=0x4f)) && code.get(index + 1) == Some(&0x8d) {
        let rex = code[index];
        let Some(&modrm) = code.get(index + 2) else {
            return false;
        };
        let mode = modrm >> 6;
        let destination = (modrm >> 3) & 7;
        let base = modrm & 7;
        if destination != 4 || rex & 4 != 0 || !(mode == 1 || mode == 2) {
            return false;
        }
        let mut displacement = 0;
        let base_register = if base == 4 {
            let Some(&sib) = code.get(index + 3) else {
                return false;
            };
            if sib >> 3 & 7 != 4 {
                return false;
            }
            displacement = 1;
            (sib & 7) + if rex & 1 != 0 { 8 } else { 0 }
        } else {
            base + if rex & 1 != 0 { 8 } else { 0 }
        };
        if !matches!(base_register, 3 | 5 | 6 | 7 | 12 | 13 | 14 | 15) {
            return false;
        }
        index += 3 + displacement + if mode == 1 { 1 } else { 4 };
    }

    // Remaining nonvolatile registers may be restored with POPs. The PC can
    // already be partway through this sequence when the timer interrupts.
    loop {
        let Some(&opcode) = code.get(index) else {
            return false;
        };
        let (register, size) = if (0x58..=0x5f).contains(&opcode) {
            (opcode - 0x58, 1)
        } else if opcode == 0x41 {
            let Some(&next) = code.get(index + 1) else {
                return false;
            };
            if !(0x58..=0x5f).contains(&next) {
                break;
            }
            (next - 0x58 + 8, 2)
        } else {
            break;
        };
        if !matches!(register, 3 | 5 | 6 | 7 | 12 | 13 | 14 | 15) {
            return false;
        }
        index += size;
    }

    match code.get(index..) {
        Some([0xc3, ..]) | Some([0xc2, _, _, ..]) | Some([0xf3, 0xc3, ..]) => true,
        Some([0xe9, _, _, _, _, ..]) | Some([0xeb, _, ..]) => true,
        Some([0xff, modrm, ..]) => (modrm >> 3) & 7 == 4,
        _ => false,
    }
}

unsafe fn read_u16(pointer: *const u8) -> u16 {
    // SAFETY: The caller ensures pointer is within a PE metadata region.
    unsafe { core::ptr::read_unaligned(pointer.cast()) }
}

unsafe fn read_u32(pointer: *const u8) -> u32 {
    // SAFETY: The caller ensures pointer is within a PE metadata region.
    unsafe { core::ptr::read_unaligned(pointer.cast()) }
}

unsafe fn read_u64(pointer: *const u8) -> u64 {
    // SAFETY: The caller ensures pointer is within the active stack or PE image.
    unsafe { core::ptr::read_unaligned(pointer.cast()) }
}

/// Emits all buffered samples as `P:<ip> <caller> ...` lines over port 0xe9.
pub(crate) fn dump() {
    use crate::interrupts::outb;

    x86_64::instructions::interrupts::disable();
    write_bytes(b"AGNOSTOS_PROFILE_V1\n");
    let count = SAMPLE_COUNT.load(Ordering::Relaxed);
    // SAFETY: Interrupts are disabled and the buffer is no longer being changed.
    let samples = unsafe { &*SAMPLES.0.get() };
    for sample in samples.iter().take(count) {
        write_bytes(b"P:");
        for ip in sample.ips.iter().take(sample.depth as usize) {
            write_hex(*ip);
            write_bytes(b" ");
        }
        write_bytes(b"\n");
    }
    write_bytes(b"AGNOSTOS_PROFILE_END\n");

    fn write_bytes(bytes: &[u8]) {
        for byte in bytes {
            // SAFETY: Port 0xe9 is the QEMU isa-debugcon device configured by
            // scripts/profile.sh.
            unsafe { outb(*byte, 0xe9) };
        }
    }

    fn write_hex(value: u64) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut started = false;
        for shift in (0..16).rev() {
            let nibble = ((value >> (shift * 4)) & 0xf) as usize;
            if nibble != 0 || started || shift == 0 {
                started = true;
                write_bytes(&[HEX[nibble]]);
            }
        }
    }
}
