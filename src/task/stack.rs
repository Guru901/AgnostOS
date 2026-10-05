//! Fixed-storage kernel stacks for cooperative tasks.
//!
//! Each task owns a 4 KiB buffer embedded in the scheduler. There is no
//! unmapped guard page: overflowing a task stack corrupts the neighboring
//! scheduler slot in `.bss` (or the host-test `Scheduler` value). Hardware
//! IRQs that do not use an IST also run on the current stack, so interrupt
//! handlers share this 4 KiB while a task is executing. Keep task bodies and
//! IRQ work shallow until stacks move to guarded physical frames.

use crate::globals::task::{STACK_FRAME_WORDS, TASK_STACK_SIZE};

/// Per-task stack embedded in the scheduler. Not backed by the frame allocator yet.
#[repr(align(16))]
#[derive(Clone, Copy)]
pub struct TaskStack {
    bytes: [u8; TASK_STACK_SIZE],
}

impl TaskStack {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [0; TASK_STACK_SIZE],
        }
    }

    /// Returns the 16-byte-aligned top address of this stack.
    #[must_use]
    pub fn top(&self) -> usize {
        let end = self.bytes.as_ptr() as usize + TASK_STACK_SIZE;
        end & !15
    }

    /// Lowest address still inside this stack buffer.
    #[must_use]
    pub fn base(&self) -> usize {
        self.bytes.as_ptr() as usize
    }

    /// Builds the initial frame consumed by [`crate::task::context::switch`].
    ///
    /// The frame is written only when a task starts for the first time so the
    /// addresses match this stack's current location. After the first yield,
    /// [`crate::task::context::switch`] updates the saved stack pointer in
    /// place; rebuilding this frame would restart the task.
    ///
    /// Layout at `stack_pointer` (low address first), matching the pops in
    /// `switch`:
    ///
    /// ```text
    /// rbx, rbp, r12, r13, r14, r15, trampoline RIP
    /// ```
    ///
    /// `top` is 16-byte aligned and the frame is 7 words, so the saved RSP is
    /// `8` mod `16`, matching a normal `call` into `switch`. After six pops
    /// and `ret`, the trampoline is entered via `ret` with RSP 16-byte
    /// aligned; the trampoline immediately `call`s the Rust entry so SysV
    /// alignment is restored.
    pub fn initialize_frame(&mut self, context: &super::context::TaskContext) -> usize {
        let stack_pointer = self.top() - STACK_FRAME_WORDS * core::mem::size_of::<usize>();
        let frame = [
            context.rbx,
            context.rbp,
            context.r12,
            context.r13,
            context.r14,
            context.r15,
            super::task_trampoline as *const () as usize,
        ];
        // SAFETY: `stack_pointer` is inside this aligned stack; the copy
        // writes exactly seven `usize` words below `top`.
        unsafe {
            core::ptr::copy_nonoverlapping(
                frame.as_ptr(),
                stack_pointer as *mut usize,
                STACK_FRAME_WORDS,
            );
        }
        stack_pointer
    }
}

impl Default for TaskStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::TaskStack;
    use crate::globals::task::TASK_STACK_SIZE;
    use crate::task::context::TaskContext;

    #[test]
    fn stack_is_aligned_and_has_expected_capacity() {
        let stack = TaskStack::new();
        assert_eq!(TASK_STACK_SIZE, 4 * 1024);
        assert_eq!(stack.top() % 16, 0);
    }

    #[test]
    fn initial_frame_matches_switch_pop_order() {
        let mut stack = TaskStack::new();
        let mut context = TaskContext::new();
        context.rbx = 1;
        context.rbp = 2;
        context.r12 = 3;
        context.r13 = 4;
        context.r14 = 5;
        context.r15 = 6;
        let sp = stack.initialize_frame(&context);
        assert_eq!(sp % 16, 8);
        assert!(sp >= stack.base());
        assert!(sp < stack.top());
        // SAFETY: `sp` is the frame we just wrote on this stack.
        let words = unsafe { core::slice::from_raw_parts(sp as *const usize, 7) };
        assert_eq!(&words[..6], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(words[6], crate::task::task_trampoline as *const () as usize);
    }
}
