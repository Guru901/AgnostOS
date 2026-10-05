//! Architecture context representation for task switching.
//!
//! The scheduler prepares contexts for this primitive, while keeping the ABI
//! and safety contract isolated from task lifetime and scheduling policy.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TaskContext {
    /// Stack pointer restored by [`switch`]; must remain the first field so
    /// the naked `switch` assembly can use `[rdi]` / `[rsi]` at offset 0.
    pub stack_pointer: usize,
    pub rbx: usize,
    pub rbp: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
}

const _: () = assert!(core::mem::offset_of!(TaskContext, stack_pointer) == 0);

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
/// This function is naked so the `ret` in the assembly returns to the other
/// context's saved instruction pointer rather than through a compiler
/// epilogue. The compiler at the *call site* still sees a normal return when
/// the other context later switches back.
///
/// # Safety
///
/// * `from` and `to` must be valid for the whole switch (and, for `from`,
///   until some later switch writes it again).
/// * `to.stack_pointer` must reference six words in `rbx`, `rbp`, `r12`–`r15`
///   order, followed by a valid return address, on a live stack that will not
///   be moved.
/// * Neither context may be modified concurrently.
/// * The destination task must be the runnable task being dispatched.
/// * The caller must not hold a Rust reference that aliases either context
///   in a way that is used across the switch except through these pointers.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub unsafe extern "C" fn switch(from: *mut TaskContext, to: *const TaskContext) {
    // SysV: `from` in rdi, `to` in rsi. Pushes must match
    // [`super::stack::TaskStack::initialize_frame`].
    core::arch::naked_asm!(
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
    );
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
        assert_eq!(core::mem::offset_of!(TaskContext, stack_pointer), 0);
    }
}
