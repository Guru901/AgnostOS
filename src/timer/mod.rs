use crate::TICKS;
use core::{hint::spin_loop, sync::atomic::Ordering};

pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks()
}

/// Busy-waits using spin_loop() instead of hlt. Only use this for very short
/// waits — it does not yield the CPU.
pub fn sleep_block(ms: u64) {
    if !x86_64::instructions::interrupts::are_enabled() {
        debug_assert!(false, "sleep_block called before IDT init");
        return;
    };

    let start = ticks();
    let target = start.saturating_add(ms);

    while ticks() < target {
        spin_loop();
    }
}

pub fn sleep_ms(ms: u64) {
    if !x86_64::instructions::interrupts::are_enabled() {
        debug_assert!(false, "sleep_ms called before IDT init");
        return;
    };

    let start = ticks();
    let target = start.saturating_add(ms);

    while ticks() < target {
        x86_64::instructions::hlt();
    }
}

pub fn deadline_ms(ms: u64) -> u64 {
    ticks().saturating_add(ms)
}

pub fn expired(deadline: u64) -> bool {
    ticks() >= deadline
}
