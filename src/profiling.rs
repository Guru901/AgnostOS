//! Manual function-entry recording for QEMU profiling runs.
//!
//! Functions marked with `#[profiled]` append their names to a bounded buffer.
//! The buffer is exported by `shutdown`, outside the function entry hook.

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

const EVENT_CAPACITY: usize = 65_536;

struct EventBuffer(UnsafeCell<[&'static str; EVENT_CAPACITY]>);
unsafe impl Sync for EventBuffer {}

static EVENTS: EventBuffer = EventBuffer(UnsafeCell::new([""; EVENT_CAPACITY]));
static EVENT_COUNT: AtomicUsize = AtomicUsize::new(0);
static DROPPED_EVENTS: AtomicUsize = AtomicUsize::new(0);

/// Records one entry into a function annotated with `#[profiled]`.
pub(crate) fn record_function(name: &'static str) {
    let index = EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
    if index >= EVENT_CAPACITY {
        DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // SAFETY: The current kernel is single-core. Export disables interrupts,
    // so the buffer cannot be read while an entry is being written.
    unsafe { (*EVENTS.0.get())[index] = name };
}

/// Emits buffered function entries over QEMU's debug console.
pub(crate) fn dump() {
    use crate::interrupts::outb;

    x86_64::instructions::interrupts::disable();
    write_bytes(b"AGNOSTOS_PROFILE_V1\n");
    let count = EVENT_COUNT.load(Ordering::Relaxed).min(EVENT_CAPACITY);
    // SAFETY: Interrupts are disabled and the buffer is no longer being changed.
    let events = unsafe { &*EVENTS.0.get() };
    for event in events.iter().take(count) {
        write_bytes(b"E:");
        write_bytes(event.as_bytes());
        write_bytes(b"\n");
    }
    let dropped = DROPPED_EVENTS.load(Ordering::Relaxed);
    if dropped > 0 {
        write_bytes(b"AGNOSTOS_PROFILE_DROPPED=");
        write_decimal(dropped);
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

    fn write_decimal(mut value: usize) {
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
