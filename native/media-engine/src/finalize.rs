//! Shared finalization steps (used by `stop` and by crash recovery): mux the intermediates
//! into the final MP4 without overwriting anything, validate it with ffprobe and create a
//! thumbnail.

use crate::args::{mux_args, thumbnail_args, MuxSpec};
use crate::manifest::THUMBNAILS_DIR;
use crate::naming::{persist_no_clobber, temp_path};
use crate::probe::{codec_from_name, run_ffprobe, validate, Expectation, ProbeInfo};
use crate::tools::{classify_ffmpeg_error, new_command, run_with_timeout};
use capture_core::{AudioTrackInfo, CaptureError, CaptureErrorCode, CaptureResult, MediaKind, MediaMetadata, MediaValidation};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct MuxOutcome {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub probe: ProbeInfo,
    pub validation: MediaValidation,
    pub thumbnail: Option<PathBuf>,
}

/// Runs the mux into a temp file in `output_dir`, then moves it to the first free name.
pub fn mux_to_final(
    ffmpeg: &Path,
    ffprobe: &Path,
    spec_without_out: &MuxSpec,
    output_dir: &Path,
    id: &str,
    names: impl Iterator<Item = String>,
    expectation: &Expectation,
    timeout: Duration,
) -> CaptureResult<MuxOutcome> {
    let tmp = temp_path(output_dir, id, "mp4");
    let mut spec = spec_without_out.clone();
    spec.out_path = tmp.clone();
    let mut cmd = new_command(ffmpeg);
    cmd.args(mux_args(&spec));
    let out = run_with_timeout(cmd, timeout)?;
    if !out.status.success() || !tmp.is_file() {
        let _ = std::fs::remove_file(&tmp);
        let err = out.stderr_text();
        return Err(CaptureError::new(classify_ffmpeg_error(&err, CaptureErrorCode::EncoderFailed), format!("muxing failed: {err}")));
    }
    let path = persist_no_clobber(&tmp, output_dir, names)?;
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let probe = run_ffprobe(ffprobe, &path)?;
    let validation = validate(&probe, size_bytes, expectation);
    let duration = probe.duration_sec.unwrap_or(0.0);
    let thumbnail = make_thumbnail(ffmpeg, &path, output_dir, duration);
    Ok(MuxOutcome { path, size_bytes, probe, validation, thumbnail })
}

/// JPEG thumbnail in `<output_dir>/.captureone-thumbnails/<stem>.jpg`.
pub fn make_thumbnail(ffmpeg: &Path, video: &Path, output_dir: &Path, duration_sec: f64) -> Option<PathBuf> {
    let dir = output_dir.join(THUMBNAILS_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    let stem = video.file_stem()?.to_string_lossy().to_string();
    let out = dir.join(format!("{stem}.jpg"));
    let seek = (duration_sec / 2.0).min(1.0).max(0.0);
    for s in [seek, 0.0] {
        let mut cmd = new_command(ffmpeg);
        cmd.args(thumbnail_args(video, s, 480, &out));
        if let Ok(o) = run_with_timeout(cmd, Duration::from_secs(30)) {
            if o.status.success() && std::fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false) {
                return Some(out);
            }
        }
    }
    None
}

pub struct VideoMetaInput<'a> {
    pub id: &'a str,
    pub path: &'a Path,
    pub size_bytes: u64,
    pub probe: &'a ProbeInfo,
    pub validation: Option<MediaValidation>,
    pub created_at: String,
    pub source_name: Option<String>,
    pub platform: Option<String>,
    pub thumbnail: Option<PathBuf>,
}

pub fn video_metadata(m: VideoMetaInput<'_>) -> MediaMetadata {
    let v = m.probe.video.as_ref();
    let duration = m.probe.duration_sec.or_else(|| v.and_then(|v| v.duration_sec));
    MediaMetadata {
        id: m.id.to_string(),
        kind: MediaKind::Video,
        path: m.path.to_string_lossy().to_string(),
        file_name: m.path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
        title: m.path.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
        created_at: m.created_at,
        size_bytes: m.size_bytes,
        width: v.map_or(0, |v| v.width),
        height: v.map_or(0, |v| v.height),
        duration_ms: duration.map(|d| (d * 1000.0).round()),
        fps: v.and_then(|v| v.fps),
        video_codec: v.and_then(|v| codec_from_name(&v.codec_name)),
        video_bitrate_kbps: v.and_then(|v| v.bit_rate).map(|b| (b / 1000.0).round()),
        audio_tracks: m
            .probe
            .audio
            .iter()
            .enumerate()
            .map(|(i, a)| AudioTrackInfo {
                index: i as u32,
                codec: a.codec_name.clone(),
                channels: a.channels,
                sample_rate: a.sample_rate,
                label: a.label.clone(),
                duration_ms: a.duration_sec.map(|d| (d * 1000.0).round()),
            })
            .collect(),
        image_format: None,
        container: "mp4".into(),
        source_name: m.source_name,
        platform: m.platform,
        validation: m.validation,
        thumbnail_path: m.thumbnail.map(|p| p.to_string_lossy().to_string()),
    }
}

pub fn platform_string() -> String {
    serde_json::to_value(capture_core::Platform::current()).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "unknown".into())
}

/// ISO-8601 time of a file's modification.
pub fn file_time_iso(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| capture_core::iso8601_from_unix(d.as_secs()))
        .unwrap_or_else(capture_core::now_iso8601)
}
