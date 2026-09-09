use alloc::{format, string::String};

use crate::{TICKS, platform};
use core::{hint::spin_loop, sync::atomic::Ordering};

/// The oscillator frequency used by the legacy PIT.
///
/// This is the nominal hardware frequency.  Time conversion uses the divisor
/// below rather than assuming the requested interrupt rate is exact.
pub(crate) const PIT_INPUT_HZ: u64 = 1_193_182;
const PIT_TARGET_HZ: u64 = 1_000;

/// PIT channel 0 reload value, rounded to the closest representable rate.
///
/// A 1 kHz timer cannot be represented exactly by the PIT: this produces
/// 1,000.1525... IRQs per second.  Keeping the actual divisor alongside the
/// conversion routines prevents that small error from accumulating in uptime.
pub(crate) const PIT_DIVISOR: u16 = ((PIT_INPUT_HZ + PIT_TARGET_HZ / 2) / PIT_TARGET_HZ) as u16;

const TICK_DURATION_US_NUMERATOR: u64 = PIT_DIVISOR as u64 * 1_000_000;

pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks_to_us(ticks()) / 1_000
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
    let target = start.saturating_add(ms_to_ticks_ceil(ms));

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
    let target = start.saturating_add(ms_to_ticks_ceil(ms));

    while ticks() < target {
        platform::halt();
    }
}

pub fn deadline_ms(ms: u64) -> u64 {
    ticks().saturating_add(ms_to_ticks_ceil(ms))
}

pub fn expired(deadline: u64) -> bool {
    ticks() >= deadline
}

/// Converts elapsed PIT interrupts to microseconds using the programmed
/// reload value.  `u128` keeps the calculation correct even for long uptimes.
fn ticks_to_us(ticks: u64) -> u64 {
    ((u128::from(ticks) * u128::from(TICK_DURATION_US_NUMERATOR)) / u128::from(PIT_INPUT_HZ))
        .min(u128::from(u64::MAX)) as u64
}

/// Returns enough PIT ticks to cover a requested millisecond interval.
/// Rounding up ensures sleep and deadline users never fire early merely
/// because a PIT tick is slightly shorter than one millisecond.
fn ms_to_ticks_ceil(ms: u64) -> u64 {
    let numerator = u128::from(ms) * u128::from(PIT_INPUT_HZ);
    let denominator = u128::from(PIT_DIVISOR) * 1_000;
    let ticks = numerator.saturating_add(denominator - 1) / denominator;

    ticks.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::{PIT_DIVISOR, PIT_INPUT_HZ, format_uptime, ms_to_ticks_ceil, ticks_to_us};

    #[test]
    fn pit_divisor_is_the_closest_1khz_rate() {
        assert_eq!(PIT_DIVISOR, 1_193);
    }

    #[test]
    fn converts_ticks_using_the_programmed_pit_period() {
        // 1,001 PIT ticks at divisor 1,193 are just over one second.
        assert_eq!(ticks_to_us(1_001), 1_000_847);
        // This exact whole-period case protects the conversion's numerator.
        assert_eq!(ticks_to_us(PIT_INPUT_HZ), 1_193_000_000);
    }

    #[test]
    fn millisecond_waits_round_up_to_pit_ticks() {
        // 1,000 ticks are 999.847 ms, so a one-second wait needs 1,001.
        assert_eq!(ms_to_ticks_ceil(1_000), 1_001);
    }

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
