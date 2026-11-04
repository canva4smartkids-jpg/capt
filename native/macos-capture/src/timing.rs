//! Timestamp helpers.
//!
//! ScreenCaptureKit stamps sample buffers with presentation times on the host clock
//! (`CMClockGetHostTimeClock`, i.e. `mach_absolute_time`). The engine wants `Instant`s, so we
//! measure how old a sample is on the host clock and subtract that age from `Instant::now()`.

use std::time::{Duration, Instant};

/// Samples older than this (or from the future by more than [`MAX_FUTURE_SECS`]) are treated as
/// having an unusable timestamp; delivery time is used instead.
pub const MAX_AGE_SECS: f64 = 2.0;
pub const MAX_FUTURE_SECS: f64 = 0.05;

/// Best-effort `Instant` of a sample captured `age_secs` before `now`.
pub fn instant_from_age(now: Instant, age_secs: f64) -> Instant {
    if !age_secs.is_finite() || !(-MAX_FUTURE_SECS..=MAX_AGE_SECS).contains(&age_secs) {
        return now;
    }
    let age = Duration::from_secs_f64(age_secs.max(0.0));
    now.checked_sub(age).unwrap_or(now)
}

/// `SCStreamConfiguration.minimumFrameInterval` for a desired fps as a CMTime
/// `(value, timescale)` pair: 1/fps seconds. fps is clamped to 1..=120.
pub fn frame_interval(fps: u32) -> (i64, i32) {
    (1, fps.clamp(1, 120) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_is_subtracted() {
        let now = Instant::now();
        let t = instant_from_age(now, 0.040);
        assert_eq!(now.duration_since(t), Duration::from_millis(40));
    }

    #[test]
    fn small_negative_ages_clamp_to_now() {
        let now = Instant::now();
        assert_eq!(instant_from_age(now, -0.01), now);
    }

    #[test]
    fn implausible_ages_fall_back_to_now() {
        let now = Instant::now();
        assert_eq!(instant_from_age(now, 3600.0), now);
        assert_eq!(instant_from_age(now, -1.0), now);
        assert_eq!(instant_from_age(now, f64::NAN), now);
        assert_eq!(instant_from_age(now, f64::INFINITY), now);
    }

    #[test]
    fn frame_intervals() {
        assert_eq!(frame_interval(30), (1, 30));
        assert_eq!(frame_interval(60), (1, 60));
        assert_eq!(frame_interval(0), (1, 1));
        assert_eq!(frame_interval(1000), (1, 120));
    }
}
