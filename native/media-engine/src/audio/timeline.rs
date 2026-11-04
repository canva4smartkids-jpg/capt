//! Audio timeline: places captured PCM on the active clock so every audio track has
//! exactly the recording's active duration and stays in sync with video.
//!
//! * samples captured before t=0 or while paused are dropped by the caller / here;
//! * if the first buffer starts after t=0, silence is prepended;
//! * gaps (device underruns, WASAPI loopback delivering nothing during silence) larger than
//!   the tolerance are filled with silence; overlaps beyond the tolerance (device clock
//!   running fast) are dropped;
//! * at the end the track is padded to the final active duration.

/// Gap/overlap tolerated before correcting (absorbs callback jitter).
pub const TOLERANCE_SEC: f64 = 0.06;
/// If no audio arrived for this long, fill silence (source stalled / silent loopback).
pub const STALL_SEC: f64 = 0.3;
/// Silence is filled up to `now - REFILL_LAG_SEC` so late buffers still fit.
pub const REFILL_LAG_SEC: f64 = 0.15;

#[derive(Debug, Clone)]
pub struct AudioTimeline {
    rate: u32,
    channels: usize,
    written: u64,
    started: bool,
    pub silence_inserted_frames: u64,
    pub dropped_frames: u64,
}

impl AudioTimeline {
    pub fn new(rate: u32, channels: u16) -> Self {
        Self { rate, channels: channels.max(1) as usize, written: 0, started: false, silence_inserted_frames: 0, dropped_frames: 0 }
    }

    pub fn written_frames(&self) -> u64 {
        self.written
    }

    pub fn written_sec(&self) -> f64 {
        self.written as f64 / self.rate as f64
    }

    fn frames_at(&self, sec: f64) -> i64 {
        (sec * self.rate as f64).round() as i64
    }

    fn silence(&mut self, frames: u64, out: &mut Vec<f32>) {
        out.resize(out.len() + frames as usize * self.channels, 0.0);
        self.written += frames;
        self.silence_inserted_frames += frames;
    }

    /// Places interleaved `samples` (at the timeline's rate/channels) whose first frame was
    /// captured at active time `start_sec`. Returns the PCM to append to the track.
    pub fn push(&mut self, samples: &[f32], start_sec: f64) -> Vec<f32> {
        let n = samples.len() / self.channels;
        let mut out = Vec::with_capacity(samples.len());
        let mut skip: usize = 0;
        let mut pos = self.frames_at(start_sec);
        if pos < 0 {
            // captured before t=0
            skip = ((-pos) as usize).min(n);
            self.dropped_frames += skip as u64;
            pos = 0;
        }
        if skip < n {
            let written = self.written as i64;
            let diff = pos - written;
            let tol = self.frames_at(TOLERANCE_SEC);
            if !self.started && diff > 0 {
                self.silence(diff as u64, &mut out); // first buffer after t=0: align exactly
            } else if diff > tol {
                self.silence(diff as u64, &mut out);
            } else if diff < -tol {
                let overlap = ((-diff) as usize).min(n - skip);
                skip += overlap;
                self.dropped_frames += overlap as u64;
            }
            self.started = true;
        }
        let rest = &samples[skip * self.channels..n * self.channels];
        out.extend_from_slice(rest);
        self.written += (n - skip) as u64;
        out
    }

    /// Called periodically: if the source has stalled for `STALL_SEC`, fills silence up to
    /// `now - REFILL_LAG_SEC`.
    pub fn fill_stall(&mut self, active_now_sec: f64) -> Vec<f32> {
        let mut out = Vec::new();
        let now = self.frames_at(active_now_sec);
        if now - (self.written as i64) > self.frames_at(STALL_SEC) {
            let target = (now - self.frames_at(REFILL_LAG_SEC)).max(0) as u64;
            if target > self.written {
                self.silence(target - self.written, &mut out);
                self.started = true;
            }
        }
        out
    }

    /// Pads the track with silence up to `total_sec`.
    pub fn finish(&mut self, total_sec: f64) -> Vec<f32> {
        let mut out = Vec::new();
        let target = self.frames_at(total_sec).max(0) as u64;
        if target > self.written {
            self.silence(target - self.written, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 48_000;

    fn buf(frames: usize, v: f32) -> Vec<f32> {
        vec![v; frames * 2]
    }

    #[test]
    fn prepends_silence_when_first_buffer_is_late() {
        let mut t = AudioTimeline::new(R, 2);
        let out = t.push(&buf(480, 0.5), 0.25);
        assert_eq!(out.len(), (12_000 + 480) * 2);
        assert!(out[..24_000].iter().all(|&s| s == 0.0));
        assert_eq!(out[24_000], 0.5);
        assert_eq!(t.silence_inserted_frames, 12_000);
    }

    #[test]
    fn drops_samples_before_start() {
        let mut t = AudioTimeline::new(R, 2);
        let out = t.push(&buf(960, 0.5), -0.01); // 480 frames before t=0
        assert_eq!(out.len(), 480 * 2);
        assert_eq!(t.written_frames(), 480);
    }

    #[test]
    fn jitter_within_tolerance_is_ignored() {
        let mut t = AudioTimeline::new(R, 2);
        t.push(&buf(480, 0.1), 0.0);
        // next buffer reported 30 ms late (jitter) — written as-is, no silence
        let out = t.push(&buf(480, 0.1), 0.04);
        assert_eq!(out.len(), 960);
        assert_eq!(t.silence_inserted_frames, 0);
    }

    #[test]
    fn fills_gaps_and_drops_overlaps() {
        let mut t = AudioTimeline::new(R, 2);
        t.push(&buf(4800, 0.1), 0.0); // 0..0.1 s
        let out = t.push(&buf(4800, 0.1), 0.3); // 200 ms gap
        assert_eq!(out.len(), (9600 + 4800) * 2);
        assert_eq!(t.written_frames(), 4800 * 3 + 9600 - 4800);
        // overlap: buffer claims to start at 0.2 s although we are at 0.4 s
        let before = t.written_frames();
        let out = t.push(&buf(4800 * 3, 0.1), 0.2);
        assert_eq!(out.len() / 2, 4800 * 3 - 9600);
        assert_eq!(t.written_frames(), before + 4800);
        assert_eq!(t.dropped_frames, 9600);
    }

    #[test]
    fn stall_fill_and_finish_padding() {
        let mut t = AudioTimeline::new(R, 2);
        assert!(t.fill_stall(0.2).is_empty(), "no fill before the stall threshold");
        let out = t.fill_stall(1.0);
        assert_eq!(out.len() / 2, ((1.0 - REFILL_LAG_SEC) * R as f64).round() as usize);
        // late real data near "now" is placed after a small additional gap correction
        t.push(&buf(480, 0.3), 0.98);
        let end = t.finish(2.0);
        assert_eq!(t.written_frames(), 96_000);
        assert!(end.iter().all(|&s| s == 0.0));
        assert!(t.finish(1.0).is_empty(), "never truncates");
    }

    /// A device clock running 0.2 % fast / slow over 60 s with 10 ms buffers and +-4 ms
    /// timestamp jitter must stay within the tolerance of real time.
    #[test]
    fn drift_is_corrected() {
        for skew in [1.002_f64, 0.998] {
            let mut t = AudioTimeline::new(R, 2);
            let frames_per_buf = 480usize;
            let mut device_frames: u64 = 0;
            let mut i = 0u64;
            loop {
                // wall time at which this buffer's first sample was captured
                let start = device_frames as f64 / (R as f64 * skew);
                if start > 60.0 {
                    break;
                }
                let jitter = ((i * 7919) % 9) as f64 / 1000.0 - 0.004;
                t.push(&buf(frames_per_buf, 0.2), (start + jitter).max(0.0));
                device_frames += frames_per_buf as u64;
                i += 1;
            }
            let err = t.written_sec() - 60.0;
            assert!(err.abs() <= TOLERANCE_SEC + 0.02, "skew {skew}: drift {err:.4}s");
            assert!(t.silence_inserted_frames + t.dropped_frames > 0, "skew {skew} needed corrections");
        }
    }
}
