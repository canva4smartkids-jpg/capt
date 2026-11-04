//! Clock mapping and frame-delivery pacing (platform independent).
//!
//! WGC frames (`Direct3D11CaptureFrame::SystemRelativeTime`) and WASAPI packets
//! (`IAudioCaptureClient::GetBuffer` `pu64QPCPosition`) are stamped in 100 ns units of the
//! QueryPerformanceCounter clock. `std::time::Instant` on Windows is also QPC based, so a capture
//! timestamp maps to an `Instant` by subtracting its age from a (now, QPC-now) reference pair.

use std::time::{Duration, Instant};

/// Timestamps older than this (or in the future) are treated as bogus and replaced by "now".
pub const MAX_TIMESTAMP_AGE: Duration = Duration::from_secs(5);

/// Converts a raw QPC counter value to 100 ns units.
pub fn qpc_to_hns(counter: i64, frequency: i64) -> i64 {
    if frequency <= 0 {
        return 0;
    }
    ((counter as i128 * 10_000_000) / frequency as i128) as i64
}

/// Maps an event stamped `event_hns` (100 ns QPC units) to an `Instant`, given that `now`
/// corresponds to `now_hns`.
pub fn instant_from_hns(now: Instant, now_hns: i64, event_hns: i64) -> Instant {
    let age_hns = now_hns.saturating_sub(event_hns);
    if age_hns <= 0 {
        return now;
    }
    let age = Duration::from_nanos((age_hns as u64).saturating_mul(100));
    if age > MAX_TIMESTAMP_AGE {
        return now;
    }
    now.checked_sub(age).unwrap_or(now)
}

/// Duration of `frames` audio frames at `rate`.
pub fn frames_duration(frames: usize, rate: u32) -> Duration {
    if rate == 0 {
        return Duration::ZERO;
    }
    Duration::from_nanos((frames as u128 * 1_000_000_000 / rate as u128) as u64)
}

/// Limits frame delivery to roughly the requested fps while guaranteeing that the most recent
/// frame is eventually delivered (the caller flushes a pending frame from a timer when `due`).
#[derive(Debug, Clone)]
pub struct DeliveryPacer {
    interval: Duration,
    tolerance: Duration,
    last: Option<Instant>,
}

impl DeliveryPacer {
    /// `fps == 0` disables pacing.
    pub fn new(fps: u32) -> Self {
        let interval = if fps == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(1_000_000_000 / fps as u64)
        };
        // Accept frames up to 25% early so OS-side jitter does not halve the delivered rate.
        Self {
            interval,
            tolerance: interval / 4,
            last: None,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// True when a frame captured now may be delivered.
    pub fn due(&self, now: Instant) -> bool {
        match self.last {
            None => true,
            Some(last) => now.saturating_duration_since(last) + self.tolerance >= self.interval,
        }
    }

    pub fn mark_delivered(&mut self, now: Instant) {
        self.last = Some(now);
    }

    /// Period for the flush timer (Win32 timers have ~10-16 ms granularity anyway).
    pub fn flush_period_ms(&self) -> u32 {
        let half = (self.interval / 2).as_millis() as u32;
        half.clamp(5, 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qpc_conversion() {
        assert_eq!(qpc_to_hns(10_000_000, 10_000_000), 10_000_000);
        assert_eq!(qpc_to_hns(3_000_000, 3_000_000), 10_000_000);
        // Large counters do not overflow.
        assert_eq!(qpc_to_hns(i64::MAX / 2, 10_000_000), i64::MAX / 2);
        assert_eq!(qpc_to_hns(5, 0), 0);
    }

    #[test]
    fn maps_timestamps_to_instants() {
        let now = Instant::now() + Duration::from_secs(10);
        let t = instant_from_hns(now, 50_000_000, 49_900_000); // 10 ms ago
        assert_eq!(now.duration_since(t), Duration::from_millis(10));
        assert_eq!(instant_from_hns(now, 50_000_000, 50_100_000), now, "future stamps clamp to now");
        assert_eq!(instant_from_hns(now, 500_000_000, 0), now, "ancient stamps clamp to now");
    }

    #[test]
    fn audio_durations() {
        assert_eq!(frames_duration(480, 48_000), Duration::from_millis(10));
        assert_eq!(frames_duration(441, 44_100), Duration::from_millis(10));
        assert_eq!(frames_duration(10, 0), Duration::ZERO);
    }

    #[test]
    fn pacer_limits_rate_with_tolerance() {
        let mut p = DeliveryPacer::new(30);
        let t0 = Instant::now();
        assert!(p.due(t0));
        p.mark_delivered(t0);
        assert!(!p.due(t0 + Duration::from_millis(16)));
        assert!(p.due(t0 + Duration::from_millis(26)), "25% early is accepted");
        assert!(p.due(t0 + Duration::from_millis(34)));
        assert_eq!(p.flush_period_ms(), 16);
        let unpaced = DeliveryPacer::new(0);
        assert!(unpaced.due(t0));
        assert_eq!(DeliveryPacer::new(60).flush_period_ms(), 8);
        assert_eq!(DeliveryPacer::new(1).flush_period_ms(), 100);
    }
}
