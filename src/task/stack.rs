//! Fixed-storage kernel stacks for cooperative tasks.

// Keep the first fixed-storage scheduler bounded enough for static/kernel
// construction. This is a bootstrap stack size; larger workloads should move
// stack storage to the physical-frame allocator.
pub const TASK_STACK_SIZE: usize = 4 * 1024;

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
