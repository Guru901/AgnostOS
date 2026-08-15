use alloc::{format, string::String};

use crate::{TICKS, platform};
use core::{hint::spin_loop, sync::atomic::Ordering};

pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks()
}

pub(crate) fn uptime() -> String {
    format_uptime(uptime_ms())
}

fn format_uptime(uptime_ms: u64) -> String {
    let total_seconds = uptime_ms / 1000;

    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    format!("{} hrs {} min {} sec", hours, minutes, seconds)
}

/// Busy-waits using spin_loop() instead of hlt. Only use this for very short
/// waits — it does not yield the CPU.
pub fn sleep_block(ms: u64) {
    if !platform::interrupts_enabled() {
        #[cfg(target_arch = "x86_64")]
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
    if !platform::interrupts_enabled() {
        #[cfg(target_arch = "x86_64")]
        debug_assert!(false, "sleep_ms called before IDT init");
        return;
    };

    let start = ticks();
    let target = start.saturating_add(ms);

    while ticks() < target {
        platform::halt();
    }
}

pub fn deadline_ms(ms: u64) -> u64 {
    ticks().saturating_add(ms)
}

pub fn expired(deadline: u64) -> bool {
    ticks() >= deadline
}

#[cfg(test)]
mod tests {
    use super::format_uptime;

    #[test]
    fn formats_subsecond_uptime_as_zero_seconds() {
        assert_eq!(format_uptime(999), "0 hrs 0 min 0 sec");
    }

    #[test]
    fn formats_minutes_and_seconds() {
        assert_eq!(format_uptime(125_000), "0 hrs 2 min 5 sec");
    }

    #[test]
    fn formats_hours_minutes_and_seconds() {
        assert_eq!(format_uptime(3_661_000), "1 hrs 1 min 1 sec");
    }
}
