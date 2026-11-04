//! Aggregated call-tree profiling for QEMU runs.
//!
//! Functions in `#[profiled_module]` files update a bounded call tree on entry.
//! The tree is exported on shutdown, avoiding an event log that grows with
//! every repeated call.

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

const NODE_CAPACITY: usize = 16_384;
const ROOT: u32 = 0;
const NO_NODE: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Node {
    name: &'static str,
    parent: u32,
    first_child: u32,
    next_sibling: u32,
    calls: u64,
}

impl Node {
    const EMPTY: Self = Self {
        name: "",
        parent: NO_NODE,
        first_child: NO_NODE,
        next_sibling: NO_NODE,
        calls: 0,
    };

}

struct NodeBuffer(UnsafeCell<[Node; NODE_CAPACITY]>);
unsafe impl Sync for NodeBuffer {}

static NODES: NodeBuffer = NodeBuffer(UnsafeCell::new([Node::EMPTY; NODE_CAPACITY]));
static NODE_COUNT: AtomicUsize = AtomicUsize::new(1);
static CURRENT_NODE: AtomicU32 = AtomicU32::new(ROOT);
static DROPPED_PATHS: AtomicUsize = AtomicUsize::new(0);

/// Signals that the shell is ready to receive automated QEMU input.
#[agnostos_profile_macro::not_profiled]
pub(crate) fn guest_ready() {
    use crate::interrupts::outb;

    for byte in b"AGNOSTOS_PROFILE_READY\n" {
        // SAFETY: Port 0xe9 is the QEMU isa-debugcon device configured by
        // scripts/profile.sh.
        unsafe { outb(*byte, 0xe9) };
    }
}

/// Adds one call to the aggregate tree and restores the previous caller on drop.
#[agnostos_profile_macro::not_profiled]
pub(crate) fn enter(name: &'static str) -> ProfileGuard {
    let interrupts_were_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();

    let parent = CURRENT_NODE.load(Ordering::Relaxed);
    let current = if parent == NO_NODE {
        NO_NODE
    } else {
        // SAFETY: Interrupts are disabled and the kernel is single-core; this
        // mutation cannot overlap another profile-tree update.
        let nodes = unsafe { &mut *NODES.0.get() };
        let mut child = nodes[parent as usize].first_child;
        while child != NO_NODE && nodes[child as usize].name != name {
            child = nodes[child as usize].next_sibling;
        }
        if child != NO_NODE {
            nodes[child as usize].calls = nodes[child as usize].calls.saturating_add(1);
            child
        } else {
            let index = NODE_COUNT.load(Ordering::Relaxed);
            if index >= NODE_CAPACITY {
                DROPPED_PATHS.fetch_add(1, Ordering::Relaxed);
                NO_NODE
            } else {
                nodes[index] = Node {
                    name,
                    parent,
                    first_child: NO_NODE,
                    next_sibling: nodes[parent as usize].first_child,
                    calls: 1,
                };
                nodes[parent as usize].first_child = index as u32;
                NODE_COUNT.store(index + 1, Ordering::Relaxed);
                index as u32
            }
        }
    };
    CURRENT_NODE.store(current, Ordering::Relaxed);
    restore_interrupts(interrupts_were_enabled);
    ProfileGuard { previous: parent }
}

pub(crate) struct ProfileGuard {
    previous: u32,
}

impl Drop for ProfileGuard {
    #[agnostos_profile_macro::not_profiled]
    fn drop(&mut self) {
        let interrupts_were_enabled = x86_64::instructions::interrupts::are_enabled();
        x86_64::instructions::interrupts::disable();
        CURRENT_NODE.store(self.previous, Ordering::Relaxed);
        restore_interrupts(interrupts_were_enabled);
    }
}

#[agnostos_profile_macro::not_profiled]
fn restore_interrupts(were_enabled: bool) {
    if were_enabled {
        x86_64::instructions::interrupts::enable();
    }
}

/// Emits aggregate call-tree nodes over QEMU's debug console.
#[agnostos_profile_macro::not_profiled]
pub(crate) fn dump() {
    use crate::interrupts::outb;

    x86_64::instructions::interrupts::disable();
    write_bytes(b"AGNOSTOS_PROFILE_V2\n");
    let count = NODE_COUNT.load(Ordering::Relaxed).min(NODE_CAPACITY);
    let nodes = unsafe { &*NODES.0.get() };
    for (index, node) in nodes.iter().enumerate().take(count).skip(1) {
        write_bytes(b"N:");
        write_decimal(index as u64);
        write_bytes(b" ");
        write_decimal(node.parent as u64);
        write_bytes(b" ");
        write_decimal(node.calls);
        write_bytes(b" ");
        write_bytes(node.name.as_bytes());
        write_bytes(b"\n");
    }
    let dropped = DROPPED_PATHS.load(Ordering::Relaxed);
    if dropped > 0 {
        write_bytes(b"AGNOSTOS_PROFILE_DROPPED_PATHS=");
        write_decimal(dropped as u64);
        write_bytes(b"\n");
    }
    write_bytes(b"AGNOSTOS_PROFILE_END\n");

    #[agnostos_profile_macro::not_profiled]
    fn write_bytes(bytes: &[u8]) {
        for byte in bytes {
            // SAFETY: Port 0xe9 is the QEMU isa-debugcon device configured by
            // scripts/profile.sh.
            unsafe { outb(*byte, 0xe9) };
        }
    }

    #[agnostos_profile_macro::not_profiled]
    fn write_decimal(mut value: u64) {
        let mut digits = [0u8; 20];
        let mut cursor = digits.len();
        loop {
            cursor -= 1;
            digits[cursor] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        write_bytes(&digits[cursor..]);
    }
}
