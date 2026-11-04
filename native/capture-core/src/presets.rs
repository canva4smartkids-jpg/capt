//! Rust port of packages/settings/src/presets.ts, driven by the same presets.json.
//! Cross-checked against tests/contract/fixtures/presets-cases.json.

use crate::types::{QualityPreset, ResolutionSetting, Size, VideoCodec};
use serde::Deserialize;
use std::sync::OnceLock;

const PRESETS_JSON: &str = include_str!("../../../packages/settings/presets.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bpp {
    low: f64,
    medium: f64,
    high: f64,
    ultra: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BppTable {
    h264: Bpp,
    h265: Bpp,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presets {
    bits_per_pixel: BppTable,
    pub min_bitrate_kbps: f64,
    pub max_bitrate_kbps: f64,
    pub container_overhead: f64,
}

pub fn presets() -> &'static Presets {
    static P: OnceLock<Presets> = OnceLock::new();
    P.get_or_init(|| serde_json::from_str(PRESETS_JSON).expect("presets.json is valid"))
}

fn even(n: f64) -> u32 {
    ((n / 2.0).floor() * 2.0).max(2.0) as u32
}

/// JS Math.round semantics (round half up) so results match the TS implementation exactly.
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

pub fn resolve_output_size(source: Size, setting: ResolutionSetting) -> Size {
    let (sw, sh) = (source.width.max(1) as f64, source.height.max(1) as f64);
    match setting {
        ResolutionSetting::Source => Size { width: even(sw), height: even(sh) },
        ResolutionSetting::Height { height } => {
            let h = height as f64;
            if h >= sh {
                Size { width: even(sw), height: even(sh) }
            } else {
                let scale = h / sh;
                Size { width: even(js_round(sw * scale)), height: even(h) }
            }
        }
        ResolutionSetting::Custom { width, height } => {
            let scale = (width as f64 / sw).min(height as f64 / sh).min(1.0);
            Size { width: even(js_round(sw * scale)), height: even(js_round(sh * scale)) }
        }
    }
}

pub fn video_bitrate_kbps(size: Size, fps: u32, codec: VideoCodec, quality: QualityPreset, custom_kbps: Option<u32>) -> u32 {
    let p = presets();
    let table = match codec {
        VideoCodec::H264 => &p.bits_per_pixel.h264,
        VideoCodec::H265 => &p.bits_per_pixel.h265,
    };
    let bpp = match quality {
        QualityPreset::Low => table.low,
        QualityPreset::Medium => table.medium,
        QualityPreset::High => table.high,
        QualityPreset::Ultra => table.ultra,
        QualityPreset::Custom => return custom_kbps.unwrap_or(p.min_bitrate_kbps as u32),
    };
    let kbps = size.width as f64 * size.height as f64 * fps as f64 * bpp / 1000.0;
    js_round(kbps.clamp(p.min_bitrate_kbps, p.max_bitrate_kbps)) as u32
}

pub fn estimate_file_size_bytes(video_kbps: u32, audio_kbps_total: u32, duration_sec: f64) -> u64 {
    let bytes = (video_kbps as f64 + audio_kbps_total as f64) * 1000.0 * duration_sec.max(0.0) / 8.0;
    js_round(bytes * (1.0 + presets().container_overhead)) as u64
}
