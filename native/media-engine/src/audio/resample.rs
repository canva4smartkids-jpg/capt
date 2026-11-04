//! Streaming channel remix (any -> stereo) and linear-interpolation resampler.

#[derive(Debug, Clone)]
pub struct Resampler {
    out_rate: u32,
    in_rate: u32,
    /// Fractional read position relative to the start of the next input buffer (>= -1).
    pos: f64,
    prev: Option<[f32; 2]>,
}

/// Interleaved input of any channel count -> interleaved stereo frames.
pub fn to_stereo(input: &[f32], channels: u16) -> Vec<[f32; 2]> {
    let ch = channels.max(1) as usize;
    input
        .chunks_exact(ch)
        .map(|f| match ch {
            1 => [f[0], f[0]],
            2 => [f[0], f[1]],
            // FL FR FC [LFE] ... : fold the center into both sides
            _ => {
                let c = if ch >= 3 { f[2] * 0.707 } else { 0.0 };
                [f[0] + c, f[1] + c]
            }
        })
        .collect()
}

impl Resampler {
    pub fn new(out_rate: u32) -> Self {
        Self { out_rate, in_rate: 0, pos: 0.0, prev: None }
    }

    /// Converts one buffer to stereo at `out_rate` and applies `gain` (clamped to [-1, 1]).
    pub fn process(&mut self, input: &[f32], in_channels: u16, in_rate: u32, gain: f32) -> Vec<f32> {
        if in_rate == 0 {
            return Vec::new();
        }
        if in_rate != self.in_rate {
            self.in_rate = in_rate;
            self.pos = 0.0;
            self.prev = None;
        }
        let frames = to_stereo(input, in_channels);
        let apply = |s: f32| (s * gain).clamp(-1.0, 1.0);
        if in_rate == self.out_rate {
            let mut out = Vec::with_capacity(frames.len() * 2);
            for f in &frames {
                out.push(apply(f[0]));
                out.push(apply(f[1]));
            }
            return out;
        }
        if frames.is_empty() {
            return Vec::new();
        }
        let step = in_rate as f64 / self.out_rate as f64;
        let n = frames.len() as f64;
        let at = |i: isize| -> [f32; 2] {
            if i < 0 {
                self.prev.unwrap_or(frames[0])
            } else {
                frames[(i as usize).min(frames.len() - 1)]
            }
        };
        let mut out = Vec::with_capacity(((n / step) as usize + 2) * 2);
        let mut p = if self.prev.is_none() { self.pos.max(0.0) } else { self.pos };
        while p < n - 1.0 {
            let i = p.floor();
            let frac = (p - i) as f32;
            let (a, b) = (at(i as isize), at(i as isize + 1));
            out.push(apply(a[0] + (b[0] - a[0]) * frac));
            out.push(apply(a[1] + (b[1] - a[1]) * frac));
            p += step;
        }
        self.pos = p - n;
        self.prev = frames.last().copied();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remixes_mono_and_surround() {
        assert_eq!(to_stereo(&[0.5, -0.5], 1), vec![[0.5, 0.5], [-0.5, -0.5]]);
        let s = to_stereo(&[0.1, 0.2, 0.5, 0.0, 0.0, 0.0], 6);
        assert!((s[0][0] - (0.1 + 0.3535)).abs() < 1e-3);
    }

    #[test]
    fn passthrough_applies_gain_and_clamps() {
        let mut r = Resampler::new(48_000);
        let out = r.process(&[0.4, -0.4, 0.8, 0.8], 2, 48_000, 2.0);
        assert_eq!(out, vec![0.8, -0.8, 1.0, 1.0]);
    }

    #[test]
    fn resampling_preserves_duration_across_buffers() {
        for (inr, outr) in [(44_100u32, 48_000u32), (48_000, 44_100), (16_000, 48_000), (96_000, 48_000)] {
            let mut r = Resampler::new(outr);
            let mut total = 0usize;
            let per = (inr / 100) as usize; // 10 ms buffers
            for b in 0..300 {
                let buf: Vec<f32> = (0..per).map(|i| (((b * per + i) as f32) * 0.01).sin()).collect();
                total += r.process(&buf, 1, inr, 1.0).len() / 2;
            }
            let expect = outr as f64 * 3.0;
            // at most ~one input frame of latency is held back for interpolation
            let latency = (outr as f64 / inr as f64).ceil() + 1.0;
            assert!((total as f64 - expect).abs() <= latency, "{inr}->{outr}: {total} vs {expect}");
        }
    }

    #[test]
    fn resampled_sine_keeps_frequency() {
        // 1 kHz at 44.1 kHz -> 48 kHz: count zero crossings over 1 s
        let mut r = Resampler::new(48_000);
        let mut out = Vec::new();
        for b in 0..100 {
            let buf: Vec<f32> = (0..441).map(|i| ((b * 441 + i) as f32 / 44_100.0 * 2.0 * std::f32::consts::PI * 1000.0).sin()).collect();
            out.extend(r.process(&buf, 1, 44_100, 1.0));
        }
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        let crossings = left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((1990..=2010).contains(&crossings), "crossings {crossings}");
    }
}
