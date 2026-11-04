//! Linear-PCM conversion from CoreAudio `AudioBufferList` layouts to interleaved `f32`.
//!
//! ScreenCaptureKit delivers system audio as **non-interleaved Float32** (one `AudioBuffer`
//! per channel, `kAudioFormatFlagIsNonInterleaved | kAudioFormatFlagIsFloat | IsPacked`).
//! `capture_core::AudioBuffer` wants interleaved `f32`, so the planes are zipped here. The
//! conversion also accepts interleaved data and 16/32-bit signed integer or 64-bit float
//! samples, so an unexpected-but-valid layout degrades gracefully instead of failing.
//!
//! Everything here works on plain byte slices, so it is unit-tested on every OS.

use capture_core::{CaptureError, CaptureResult};

/// `kAudioFormatLinearPCM` (`'lpcm'`).
pub const FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
/// `kAudioFormatFlagIsFloat`.
pub const FLAG_IS_FLOAT: u32 = 1 << 0;
/// `kAudioFormatFlagIsBigEndian`.
pub const FLAG_IS_BIG_ENDIAN: u32 = 1 << 1;
/// `kAudioFormatFlagIsSignedInteger`.
pub const FLAG_IS_SIGNED_INTEGER: u32 = 1 << 2;
/// `kAudioFormatFlagIsNonInterleaved`.
pub const FLAG_IS_NON_INTERLEAVED: u32 = 1 << 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    F32,
    F64,
    I16,
    I32,
}

impl SampleType {
    pub fn bytes(self) -> usize {
        match self {
            SampleType::F32 | SampleType::I32 => 4,
            SampleType::F64 => 8,
            SampleType::I16 => 2,
        }
    }
}

/// The subset of an `AudioStreamBasicDescription` that matters for conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PcmLayout {
    pub sample_rate: f64,
    pub channels: u16,
    pub sample_type: SampleType,
    pub non_interleaved: bool,
    pub big_endian: bool,
}

impl PcmLayout {
    /// Validates and interprets ASBD fields (`mFormatID`, `mFormatFlags`, `mBitsPerChannel`,
    /// `mChannelsPerFrame`, `mSampleRate`).
    pub fn from_asbd(format_id: u32, flags: u32, bits_per_channel: u32, channels: u32, sample_rate: f64) -> CaptureResult<Self> {
        if format_id != FORMAT_LINEAR_PCM {
            return Err(CaptureError::internal(format!(
                "system audio arrived in a non-PCM format ('{}')",
                crate::pixels::fourcc_to_string(format_id)
            )));
        }
        if channels == 0 || channels > 32 {
            return Err(CaptureError::internal(format!("system audio reports an invalid channel count ({channels})")));
        }
        if !(sample_rate.is_finite() && (1000.0..=768_000.0).contains(&sample_rate)) {
            return Err(CaptureError::internal(format!("system audio reports an invalid sample rate ({sample_rate})")));
        }
        let is_float = flags & FLAG_IS_FLOAT != 0;
        let sample_type = match (is_float, bits_per_channel) {
            (true, 32) => SampleType::F32,
            (true, 64) => SampleType::F64,
            (false, 16) if flags & FLAG_IS_SIGNED_INTEGER != 0 => SampleType::I16,
            (false, 32) if flags & FLAG_IS_SIGNED_INTEGER != 0 => SampleType::I32,
            _ => {
                return Err(CaptureError::internal(format!(
                    "unsupported system audio sample format: {bits_per_channel}-bit, flags {flags:#x}"
                )))
            }
        };
        Ok(PcmLayout {
            sample_rate,
            channels: channels as u16,
            sample_type,
            non_interleaved: flags & FLAG_IS_NON_INTERLEAVED != 0,
            big_endian: flags & FLAG_IS_BIG_ENDIAN != 0,
        })
    }

    /// Sample rate rounded to the nearest integer Hz (what `AudioBuffer::sample_rate` carries).
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate.round() as u32
    }
}

fn read_sample(bytes: &[u8], ty: SampleType, big_endian: bool) -> f32 {
    // `bytes` always has exactly `ty.bytes()` elements (callers use `chunks_exact`), so the
    // `try_into` conversions cannot fail; fall back to silence rather than panic regardless.
    match ty {
        SampleType::F32 => {
            let b: [u8; 4] = bytes.try_into().unwrap_or([0; 4]);
            if big_endian {
                f32::from_be_bytes(b)
            } else {
                f32::from_le_bytes(b)
            }
        }
        SampleType::F64 => {
            let b: [u8; 8] = bytes.try_into().unwrap_or([0; 8]);
            (if big_endian { f64::from_be_bytes(b) } else { f64::from_le_bytes(b) }) as f32
        }
        SampleType::I16 => {
            let b: [u8; 2] = bytes.try_into().unwrap_or([0; 2]);
            let v = if big_endian { i16::from_be_bytes(b) } else { i16::from_le_bytes(b) };
            v as f32 / 32768.0
        }
        SampleType::I32 => {
            let b: [u8; 4] = bytes.try_into().unwrap_or([0; 4]);
            let v = if big_endian { i32::from_be_bytes(b) } else { i32::from_le_bytes(b) };
            (v as f64 / 2_147_483_648.0) as f32
        }
    }
}

/// Converts the buffers of an `AudioBufferList` into interleaved `f32` samples.
///
/// * non-interleaved: `buffers.len()` must equal `layout.channels`; buffer *c* holds channel
///   *c*. If the planes differ in length (should not happen) the shortest one wins.
/// * interleaved: exactly one buffer holding `frames * channels` samples.
///
/// The byte slices need no particular alignment (samples are decoded with `from_*_bytes`).
pub fn to_interleaved_f32(layout: &PcmLayout, buffers: &[&[u8]]) -> CaptureResult<Vec<f32>> {
    let bps = layout.sample_type.bytes();
    let channels = layout.channels as usize;
    if layout.non_interleaved {
        if buffers.len() != channels {
            return Err(CaptureError::internal(format!(
                "non-interleaved audio: expected {channels} buffers, got {}",
                buffers.len()
            )));
        }
        let frames = buffers.iter().map(|b| b.len() / bps).min().unwrap_or(0);
        let mut out = vec![0f32; frames * channels];
        for (c, plane) in buffers.iter().enumerate() {
            for (f, bytes) in plane.chunks_exact(bps).take(frames).enumerate() {
                out[f * channels + c] = read_sample(bytes, layout.sample_type, layout.big_endian);
            }
        }
        Ok(out)
    } else {
        if buffers.len() != 1 {
            return Err(CaptureError::internal(format!("interleaved audio: expected 1 buffer, got {}", buffers.len())));
        }
        let data = buffers[0];
        let frame_bytes = bps * channels;
        let frames = data.len() / frame_bytes;
        Ok(data[..frames * frame_bytes]
            .chunks_exact(bps)
            .map(|b| read_sample(b, layout.sample_type, layout.big_endian))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    const SCK_FLAGS: u32 = FLAG_IS_FLOAT | (1 << 3) /* packed */ | FLAG_IS_NON_INTERLEAVED;

    #[test]
    fn parses_sck_float_layout() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 2, 48_000.0).unwrap();
        assert_eq!(l.channels, 2);
        assert_eq!(l.sample_type, SampleType::F32);
        assert!(l.non_interleaved);
        assert!(!l.big_endian);
        assert_eq!(l.sample_rate_hz(), 48_000);
    }

    #[test]
    fn rejects_bad_layouts() {
        assert!(PcmLayout::from_asbd(u32::from_be_bytes(*b"aac "), SCK_FLAGS, 32, 2, 48_000.0).is_err());
        assert!(PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 0, 48_000.0).is_err());
        assert!(PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 2, 0.0).is_err());
        assert!(PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 2, f64::NAN).is_err());
        // 24-bit packed integers are not supported.
        assert!(PcmLayout::from_asbd(FORMAT_LINEAR_PCM, FLAG_IS_SIGNED_INTEGER, 24, 2, 48_000.0).is_err());
        // unsigned 16-bit is not a CoreAudio canonical format either.
        assert!(PcmLayout::from_asbd(FORMAT_LINEAR_PCM, 0, 16, 2, 48_000.0).is_err());
    }

    #[test]
    fn interleaves_non_interleaved_stereo() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 2, 48_000.0).unwrap();
        let left = f32_bytes(&[0.1, 0.2, 0.3]);
        let right = f32_bytes(&[-0.1, -0.2, -0.3]);
        let out = to_interleaved_f32(&l, &[&left, &right]).unwrap();
        assert_eq!(out, vec![0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
    }

    #[test]
    fn uneven_planes_truncate_to_shortest() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 2, 48_000.0).unwrap();
        let left = f32_bytes(&[1.0, 2.0, 3.0]);
        let mut right = f32_bytes(&[4.0, 5.0]);
        right.push(0xAA); // trailing partial sample is ignored
        let out = to_interleaved_f32(&l, &[&left, &right]).unwrap();
        assert_eq!(out, vec![1.0, 4.0, 2.0, 5.0]);
    }

    #[test]
    fn plane_count_must_match_channels() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 2, 48_000.0).unwrap();
        let left = f32_bytes(&[1.0]);
        assert!(to_interleaved_f32(&l, &[&left]).is_err());
    }

    #[test]
    fn unaligned_input_is_fine() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 1, 44_100.0).unwrap();
        let mut raw = vec![0u8];
        raw.extend(f32_bytes(&[0.5, -0.5]));
        let out = to_interleaved_f32(&l, &[&raw[1..]]).unwrap();
        assert_eq!(out, vec![0.5, -0.5]);
    }

    #[test]
    fn passes_through_interleaved_float() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, FLAG_IS_FLOAT, 32, 2, 48_000.0).unwrap();
        assert!(!l.non_interleaved);
        let data = f32_bytes(&[0.1, 0.2, 0.3, 0.4, 0.5]); // 2.5 frames -> 2 frames
        let out = to_interleaved_f32(&l, &[&data]).unwrap();
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
        assert!(to_interleaved_f32(&l, &[&data, &data]).is_err());
    }

    #[test]
    fn converts_integer_and_big_endian() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, FLAG_IS_SIGNED_INTEGER | FLAG_IS_NON_INTERLEAVED, 16, 2, 48_000.0).unwrap();
        let left: Vec<u8> = [i16::MAX, i16::MIN].iter().flat_map(|v| v.to_le_bytes()).collect();
        let right: Vec<u8> = [0i16, 16384].iter().flat_map(|v| v.to_le_bytes()).collect();
        let out = to_interleaved_f32(&l, &[&left, &right]).unwrap();
        assert!((out[0] - 0.99997).abs() < 1e-4);
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], -1.0);
        assert_eq!(out[3], 0.5);

        let be = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, FLAG_IS_FLOAT | FLAG_IS_BIG_ENDIAN, 32, 1, 48_000.0).unwrap();
        let data: Vec<u8> = [0.25f32, -1.0].iter().flat_map(|v| v.to_be_bytes()).collect();
        assert_eq!(to_interleaved_f32(&be, &[&data]).unwrap(), vec![0.25, -1.0]);

        let i32l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, FLAG_IS_SIGNED_INTEGER, 32, 1, 48_000.0).unwrap();
        let data: Vec<u8> = [i32::MIN, 1 << 30].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(to_interleaved_f32(&i32l, &[&data]).unwrap(), vec![-1.0, 0.5]);

        let f64l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, FLAG_IS_FLOAT, 64, 1, 48_000.0).unwrap();
        let data: Vec<u8> = [0.75f64].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(to_interleaved_f32(&f64l, &[&data]).unwrap(), vec![0.75]);
    }

    #[test]
    fn empty_buffers_give_no_samples() {
        let l = PcmLayout::from_asbd(FORMAT_LINEAR_PCM, SCK_FLAGS, 32, 2, 48_000.0).unwrap();
        assert!(to_interleaved_f32(&l, &[&[], &[]]).unwrap().is_empty());
    }
}
