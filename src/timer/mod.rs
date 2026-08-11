use crate::TICKS;
use core::sync::atomic::Ordering;

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn sleep_ms(ms: u64) {
    let start = ticks();
    let target = start + ms; // if your tick rate is 1 kHz

    while ticks() < target {
        x86_64::instructions::hlt();
    }
}
