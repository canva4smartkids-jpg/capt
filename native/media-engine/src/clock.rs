//! The recording's *active clock*: wall-clock time since start minus paused time.
//! Video pacing and every audio timeline are driven by it, which keeps A/V in sync
//! across pauses.

use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct ActiveClock {
    start: Option<Instant>,
    /// (paused_at, resumed_at) intervals; the last one may be open.
    pauses: Vec<(Instant, Option<Instant>)>,
}

impl ActiveClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, at: Instant) {
        self.start = Some(at);
        self.pauses.clear();
    }

    pub fn started(&self) -> bool {
        self.start.is_some()
    }

    pub fn is_paused(&self) -> bool {
        matches!(self.pauses.last(), Some((_, None)))
    }

    pub fn pause(&mut self, at: Instant) {
        if self.start.is_some() && !self.is_paused() {
            self.pauses.push((at, None));
        }
    }

    pub fn resume(&mut self, at: Instant) {
        if let Some(last) = self.pauses.last_mut() {
            if last.1.is_none() {
                last.1 = Some(at.max(last.0));
            }
        }
    }

    /// Active seconds at instant `t` (negative before start, `None` if not started).
    /// Instants inside a pause map to the time the pause began.
    pub fn active_at(&self, t: Instant) -> Option<f64> {
        let start = self.start?;
        if t < start {
            return Some(-(start - t).as_secs_f64());
        }
        let mut active = (t - start).as_secs_f64();
        for (p0, p1) in &self.pauses {
            let p0 = (*p0).max(start);
            if p0 >= t {
                continue;
            }
            let end = p1.map(|e| e.min(t)).unwrap_or(t);
            if end > p0 {
                active -= (end - p0).as_secs_f64();
            }
        }
        Some(active.max(0.0))
    }

    pub fn now(&self) -> f64 {
        self.active_at(Instant::now()).unwrap_or(0.0)
    }

    /// Total paused seconds so far.
    pub fn paused_total(&self, now: Instant) -> f64 {
        let Some(start) = self.start else { return 0.0 };
        (now.saturating_duration_since(start)).as_secs_f64() - self.active_at(now).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn excludes_paused_time() {
        let t0 = Instant::now();
        let mut c = ActiveClock::new();
        assert_eq!(c.active_at(t0), None);
        c.start(t0);
        assert!((c.active_at(t0 + ms(1000)).unwrap() - 1.0).abs() < 1e-9);
        c.pause(t0 + ms(2000));
        assert!(c.is_paused());
        // inside the pause: frozen at 2 s
        assert!((c.active_at(t0 + ms(3000)).unwrap() - 2.0).abs() < 1e-9);
        c.resume(t0 + ms(4000));
        assert!(!c.is_paused());
        assert!((c.active_at(t0 + ms(6000)).unwrap() - 4.0).abs() < 1e-9);
        // history is preserved for instants before the pause
        assert!((c.active_at(t0 + ms(1500)).unwrap() - 1.5).abs() < 1e-9);
        assert!((c.paused_total(t0 + ms(6000)) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn negative_before_start_and_double_pause() {
        let t0 = Instant::now();
        let mut c = ActiveClock::new();
        c.start(t0 + ms(500));
        assert!((c.active_at(t0).unwrap() + 0.5).abs() < 1e-9);
        c.pause(t0 + ms(1500));
        c.pause(t0 + ms(1700)); // ignored
        c.resume(t0 + ms(2500));
        c.resume(t0 + ms(2600)); // ignored
        c.pause(t0 + ms(3500));
        c.resume(t0 + ms(4500));
        // 4 s wall after start, 2 s paused
        assert!((c.active_at(t0 + ms(4500)).unwrap() - 2.0).abs() < 1e-9);
    }
}
