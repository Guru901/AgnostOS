//! Architecture context representation for task switching.
//!
//! The scheduler does not invoke [`switch`] yet. Keeping this primitive
//! isolated lets its ABI and safety contract be reviewed independently from
//! task lifetime and scheduling policy.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TaskContext {
    pub stack_pointer: usize,
    pub rbx: usize,
    pub rbp: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
}

impl TaskContext {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stack_pointer: 0,
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }
    }
}

/// Switches callee-saved registers and stacks between two contexts.
///
/// # Safety
///
/// Both contexts must point to valid, live stacks. The destination stack
/// pointer must reference six words in `rbx`, `rbp`, `r12`–`r15` order,
/// followed by a valid return address. The caller must ensure that neither
/// context is concurrently modified and that the destination task is
/// runnable.
#[cfg(target_arch = "x86_64")]
pub unsafe fn switch(from: &mut TaskContext, to: &TaskContext) {
    // SAFETY: upheld by this function's contract. The assembly only accesses
    // the context fields and the two caller-provided stacks.
    unsafe {
        core::arch::asm!(
            "push r15",
            "push r14",
            "push r13",
            "push r12",
            "push rbp",
            "push rbx",
            "mov [rdi], rsp",
            "mov rsp, [rsi]",
            "pop rbx",
            "pop rbp",
            "pop r12",
            "pop r13",
            "pop r14",
            "pop r15",
            "ret",
            in("rdi") from,
            in("rsi") to,
            clobber_abi("C"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::TaskContext;

    #[test]
    fn context_has_only_saved_stack_and_callee_saved_registers() {
        assert_eq!(
            core::mem::size_of::<TaskContext>(),
            7 * core::mem::size_of::<usize>()
        );
        assert_eq!(
            core::mem::align_of::<TaskContext>(),
            core::mem::align_of::<usize>()
        );
    }
}
