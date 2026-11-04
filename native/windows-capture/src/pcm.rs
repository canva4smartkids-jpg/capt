//! Platform-independent PCM helpers for the WASAPI loopback path: classifying a
//! `WAVEFORMATEX`/`WAVEFORMATEXTENSIBLE` mix format and converting packets to interleaved f32.
//!
//! The Windows module reads the raw format fields; everything here is plain data so it is unit
//! tested on every OS.

/// `WAVE_FORMAT_PCM`
pub const WAVE_FORMAT_PCM: u16 = 0x0001;
/// `WAVE_FORMAT_IEEE_FLOAT`
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
/// `WAVE_FORMAT_EXTENSIBLE`
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// Bytes 4..16 shared by every `KSDATAFORMAT_SUBTYPE_*` GUID derived from a WAVE format tag
/// (`XXXXXXXX-0000-0010-8000-00AA00389B71`), as (data2, data3, data4).
pub const KSDATAFORMAT_BASE_TAIL: (u16, u16, [u8; 8]) = (0x0000, 0x0010, [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);

/// Sample encoding of one channel sample inside a WASAPI packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleEncoding {
    /// 8-bit unsigned PCM (offset binary).
    U8,
    /// 16-bit signed little-endian PCM.
    I16,
    /// 24-bit signed little-endian PCM packed in 3 bytes.
    I24Packed,
    /// Signed little-endian PCM in a 32-bit container (32 valid bits, or 24/20 valid bits
    /// left-justified — the scaling is identical because the low bits are zero).
    I32,
    /// 32-bit IEEE float.
    F32,
    /// 64-bit IEEE float.
    F64,
}

impl SampleEncoding {
    pub fn container_bytes(self) -> usize {
        match self {
            SampleEncoding::U8 => 1,
            SampleEncoding::I16 => 2,
            SampleEncoding::I24Packed => 3,
            SampleEncoding::I32 | SampleEncoding::F32 => 4,
            SampleEncoding::F64 => 8,
        }
    }
}

/// The raw fields of a `WAVEFORMATEX` (+ `WAVEFORMATEXTENSIBLE` extension when present).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawWaveFormat {
    pub format_tag: u16,
    pub channels: u16,
    pub samples_per_sec: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    /// `Samples.wValidBitsPerSample` of the extensible header.
    pub valid_bits_per_sample: Option<u16>,
    /// `SubFormat` GUID of the extensible header as (data1, data2, data3, data4).
    pub sub_format: Option<(u32, u16, u16, [u8; 8])>,
}

/// A validated stream format the converter can handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub encoding: SampleEncoding,
    /// Bytes per frame (all channels), i.e. `nBlockAlign`.
    pub block_align: usize,
}

/// Classifies a mix format. Errors describe formats this backend cannot convert.
pub fn classify(raw: &RawWaveFormat) -> Result<StreamFormat, String> {
    if raw.channels == 0 {
        return Err("mix format reports 0 channels".into());
    }
    if raw.samples_per_sec == 0 {
        return Err("mix format reports a 0 Hz sample rate".into());
    }
    let tag = if raw.format_tag == WAVE_FORMAT_EXTENSIBLE {
        let (d1, d2, d3, d4) = raw.sub_format.ok_or("WAVE_FORMAT_EXTENSIBLE without a SubFormat")?;
        if (d2, d3, d4) != KSDATAFORMAT_BASE_TAIL || d1 > u16::MAX as u32 {
            return Err(format!("unsupported WAVE_FORMAT_EXTENSIBLE SubFormat {d1:08x}-{d2:04x}-{d3:04x}-{d4:02x?}"));
        }
        d1 as u16
    } else {
        raw.format_tag
    };
    let encoding = match (tag, raw.bits_per_sample) {
        (WAVE_FORMAT_IEEE_FLOAT, 32) => SampleEncoding::F32,
        (WAVE_FORMAT_IEEE_FLOAT, 64) => SampleEncoding::F64,
        (WAVE_FORMAT_PCM, 8) => SampleEncoding::U8,
        (WAVE_FORMAT_PCM, 16) => SampleEncoding::I16,
        (WAVE_FORMAT_PCM, 24) => SampleEncoding::I24Packed,
        (WAVE_FORMAT_PCM, 32) => SampleEncoding::I32,
        (t, b) => return Err(format!("unsupported sample format (format tag 0x{t:04x}, {b} bits per sample)")),
    };
    if let Some(valid) = raw.valid_bits_per_sample {
        if valid > raw.bits_per_sample {
            return Err(format!("valid bits ({valid}) exceed container bits ({})", raw.bits_per_sample));
        }
    }
    let min_align = encoding.container_bytes() * raw.channels as usize;
    let block_align = if raw.block_align == 0 { min_align } else { raw.block_align as usize };
    if block_align < min_align {
        return Err(format!(
            "block align {block_align} is smaller than {} channels x {} bytes",
            raw.channels,
            encoding.container_bytes()
        ));
    }
    Ok(StreamFormat {
        sample_rate: raw.samples_per_sec,
        channels: raw.channels,
        encoding,
        block_align,
    })
}

/// Appends `frames` frames of `data` (packed per `fmt`) to `out` as interleaved f32 in [-1, 1].
/// Returns the number of frames converted (limited by the bytes available).
pub fn convert_to_f32(data: &[u8], frames: usize, fmt: &StreamFormat, out: &mut Vec<f32>) -> usize {
    let frames = frames.min(data.len() / fmt.block_align.max(1));
    let ch = fmt.channels as usize;
    let size = fmt.encoding.container_bytes();
    out.reserve(frames * ch);
    for f in 0..frames {
        let frame = &data[f * fmt.block_align..f * fmt.block_align + ch * size];
        for s in frame.chunks_exact(size) {
            out.push(decode_sample(s, fmt.encoding));
        }
    }
    frames
}

/// Appends `frames * channels` zero samples (packets flagged `AUDCLNT_BUFFERFLAGS_SILENT`).
pub fn push_silence(frames: usize, channels: u16, out: &mut Vec<f32>) {
    out.resize(out.len() + frames * channels as usize, 0.0);
}

#[inline]
fn decode_sample(s: &[u8], enc: SampleEncoding) -> f32 {
    match enc {
        SampleEncoding::U8 => (s[0] as f32 - 128.0) / 128.0,
        SampleEncoding::I16 => i16::from_le_bytes([s[0], s[1]]) as f32 / 32_768.0,
        SampleEncoding::I24Packed => {
            // Sign-extend by placing the 3 bytes in the top of an i32.
            let v = i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8;
            v as f32 / 8_388_608.0
        }
        SampleEncoding::I32 => (i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64 / 2_147_483_648.0) as f32,
        SampleEncoding::F32 => sanitize(f32::from_le_bytes([s[0], s[1], s[2], s[3]])),
        SampleEncoding::F64 => sanitize(f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]) as f32),
    }
}

/// Float mix buffers can legitimately exceed ±1.0 (the engine clips/limits); only NaN/inf are
/// replaced so a single bad sample cannot poison the encoder.
#[inline]
fn sanitize(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(tag: u16, ch: u16, bits: u16) -> RawWaveFormat {
        RawWaveFormat {
            format_tag: tag,
            channels: ch,
            samples_per_sec: 48_000,
            block_align: ch * bits / 8,
            bits_per_sample: bits,
            valid_bits_per_sample: None,
            sub_format: None,
        }
    }

    fn extensible(sub: u32, ch: u16, bits: u16, valid: u16) -> RawWaveFormat {
        let (d2, d3, d4) = KSDATAFORMAT_BASE_TAIL;
        RawWaveFormat {
            format_tag: WAVE_FORMAT_EXTENSIBLE,
            valid_bits_per_sample: Some(valid),
            sub_format: Some((sub, d2, d3, d4)),
            ..raw(0, ch, bits)
        }
    }

    #[test]
    fn classifies_common_mix_formats() {
        // The usual shared-mode mix format: WAVE_FORMAT_EXTENSIBLE / IEEE float / 32 bit.
        let f = classify(&extensible(3, 2, 32, 32)).unwrap();
        assert_eq!((f.sample_rate, f.channels, f.encoding, f.block_align), (48_000, 2, SampleEncoding::F32, 8));
        assert_eq!(classify(&extensible(1, 2, 16, 16)).unwrap().encoding, SampleEncoding::I16);
        assert_eq!(classify(&extensible(1, 6, 24, 24)).unwrap().encoding, SampleEncoding::I24Packed);
        assert_eq!(classify(&extensible(1, 2, 32, 24)).unwrap().encoding, SampleEncoding::I32);
        assert_eq!(classify(&raw(WAVE_FORMAT_IEEE_FLOAT, 1, 32)).unwrap().encoding, SampleEncoding::F32);
        assert_eq!(classify(&raw(WAVE_FORMAT_PCM, 2, 16)).unwrap().encoding, SampleEncoding::I16);
        assert_eq!(classify(&raw(WAVE_FORMAT_PCM, 2, 8)).unwrap().encoding, SampleEncoding::U8);
    }

    #[test]
    fn rejects_unknown_formats() {
        assert!(classify(&raw(0x0055 /* MP3 */, 2, 16)).is_err());
        assert!(classify(&raw(WAVE_FORMAT_PCM, 0, 16)).is_err());
        assert!(classify(&raw(WAVE_FORMAT_PCM, 2, 12)).is_err());
        let mut bad_guid = extensible(3, 2, 32, 32);
        bad_guid.sub_format = Some((3, 0, 0x10, [0; 8]));
        assert!(classify(&bad_guid).is_err());
        let mut no_guid = extensible(3, 2, 32, 32);
        no_guid.sub_format = None;
        assert!(classify(&no_guid).is_err());
        assert!(classify(&extensible(1, 2, 16, 24)).is_err());
        let mut tiny_align = raw(WAVE_FORMAT_PCM, 2, 16);
        tiny_align.block_align = 2;
        assert!(classify(&tiny_align).is_err());
    }

    #[test]
    fn converts_i16() {
        let fmt = classify(&raw(WAVE_FORMAT_PCM, 2, 16)).unwrap();
        let mut data = Vec::new();
        for v in [0i16, i16::MAX, i16::MIN, -16_384] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        assert_eq!(convert_to_f32(&data, 2, &fmt, &mut out), 2);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0], 0.0);
        assert!((out[1] - 0.999_969).abs() < 1e-5);
        assert_eq!(out[2], -1.0);
        assert_eq!(out[3], -0.5);
    }

    #[test]
    fn converts_packed_i24_with_sign_extension() {
        let fmt = classify(&raw(WAVE_FORMAT_PCM, 1, 24)).unwrap();
        // +max, -max-1, -1, +half
        let data = [0xFF, 0xFF, 0x7F, 0x00, 0x00, 0x80, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x40];
        let mut out = Vec::new();
        assert_eq!(convert_to_f32(&data, 4, &fmt, &mut out), 4);
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert_eq!(out[1], -1.0);
        assert!((out[2] + 1.0 / 8_388_608.0).abs() < 1e-9);
        assert_eq!(out[3], 0.5);
    }

    #[test]
    fn converts_i32_container_and_f32() {
        let fmt = classify(&extensible(1, 1, 32, 24)).unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&(0x4000_0000i32).to_le_bytes());
        data.extend_from_slice(&(i32::MIN).to_le_bytes());
        let mut out = Vec::new();
        convert_to_f32(&data, 2, &fmt, &mut out);
        assert_eq!(out, vec![0.5, -1.0]);

        let fmt = classify(&extensible(3, 2, 32, 32)).unwrap();
        let mut data = Vec::new();
        for v in [0.25f32, -0.75, f32::NAN, 1.5] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        convert_to_f32(&data, 2, &fmt, &mut out);
        assert_eq!(out, vec![0.25, -0.75, 0.0, 1.5]);
    }

    #[test]
    fn converts_u8_and_f64() {
        let fmt = classify(&raw(WAVE_FORMAT_PCM, 1, 8)).unwrap();
        let mut out = Vec::new();
        convert_to_f32(&[128, 0, 192], 3, &fmt, &mut out);
        assert_eq!(out, vec![0.0, -1.0, 0.5]);

        let fmt = classify(&raw(WAVE_FORMAT_IEEE_FLOAT, 1, 64)).unwrap();
        let mut out = Vec::new();
        convert_to_f32(&(-0.125f64).to_le_bytes(), 1, &fmt, &mut out);
        assert_eq!(out, vec![-0.125]);
    }

    #[test]
    fn respects_block_align_padding_and_short_buffers() {
        // 2 channels of 16-bit in an 8-byte block (padded frames).
        let mut f = raw(WAVE_FORMAT_PCM, 2, 16);
        f.block_align = 8;
        let fmt = classify(&f).unwrap();
        let data = [0x00, 0x40, 0x00, 0xC0, 0xAA, 0xAA, 0xAA, 0xAA, 0x00, 0x20, 0x00, 0x00, 0xBB, 0xBB];
        let mut out = Vec::new();
        // Only one complete 8-byte frame is present even though 2 were requested.
        assert_eq!(convert_to_f32(&data, 2, &fmt, &mut out), 1);
        assert_eq!(out, vec![0.5, -0.5]);
    }

    #[test]
    fn silence_appends_zeros() {
        let mut out = vec![1.0];
        push_silence(3, 2, &mut out);
        assert_eq!(out, vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }
}
