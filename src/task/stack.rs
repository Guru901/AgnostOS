//! Fixed-storage kernel stacks for cooperative tasks.

// Keep the first fixed-storage scheduler bounded enough for static/kernel
// construction. This is a bootstrap stack size; larger workloads should move
// stack storage to the physical-frame allocator.
pub const TASK_STACK_SIZE: usize = 4 * 1024;
const STACK_FRAME_WORDS: usize = 7;

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

    /// Builds the initial frame consumed by [`crate::task::context::switch`].
    ///
    /// The frame is written only when the scheduler is about to execute the
    /// task, so its addresses remain valid if the scheduler was moved after
    /// the task was spawned.
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
        // SAFETY: stack_pointer is within this stack and aligned for usize;
        // the frame occupies exactly the final seven words below its top.
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
    use super::{TASK_STACK_SIZE, TaskStack};

    #[test]
    fn stack_is_aligned_and_has_expected_capacity() {
        let stack = TaskStack::new();
        assert_eq!(TASK_STACK_SIZE, 4 * 1024);
        assert_eq!(stack.top() % 16, 0);
    }
}
