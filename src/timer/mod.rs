use alloc::{format, string::String};

use crate::{TICKS, platform};
use core::{hint::spin_loop, sync::atomic::Ordering};

#[cfg(target_arch = "x86_64")]
const PIT_FREQUENCY: u64 = crate::interrupts::PIT_FREQUENCY as u64;
#[cfg(target_arch = "x86_64")]
const PIT_DIVISOR: u64 = crate::interrupts::pit_divisor() as u64;

#[cfg(not(target_arch = "x86_64"))]
const PIT_FREQUENCY: u64 = 1_000;
#[cfg(not(target_arch = "x86_64"))]
const PIT_DIVISOR: u64 = 1_000;

pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks().saturating_mul(PIT_DIVISOR).saturating_mul(1_000) / PIT_FREQUENCY
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
    let target = start.saturating_add(ticks_for_ms(ms));

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
    let target = start.saturating_add(ticks_for_ms(ms));

    while ticks() < target {
        platform::halt();
    }
}

pub fn deadline_ms(ms: u64) -> u64 {
    ticks().saturating_add(ticks_for_ms(ms))
}

pub fn expired(deadline: u64) -> bool {
    ticks() >= deadline
}

fn ticks_for_ms(milliseconds: u64) -> u64 {
    if milliseconds == 0 {
        return 0;
    }
    let denominator = PIT_DIVISOR.saturating_mul(1_000);
    milliseconds
        .saturating_mul(PIT_FREQUENCY)
        .saturating_add(denominator.saturating_sub(1))
        / denominator
}

#[cfg(test)]
mod tests {
    use super::{format_uptime, ticks_for_ms};

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

    #[test]
    fn converts_milliseconds_to_at_least_one_tick() {
        assert_eq!(ticks_for_ms(0), 0);
        assert!(ticks_for_ms(1) >= 1);
        assert!(ticks_for_ms(100) >= 100);
    }
}
