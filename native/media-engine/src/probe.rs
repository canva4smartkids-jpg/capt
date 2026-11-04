//! ffprobe JSON parsing and media validation.

use crate::tools::{new_command, run_with_timeout};
use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, IssueSeverity, MediaValidation, Size, ValidationIssue, VideoCodec};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoStreamInfo {
    pub index: u32,
    pub codec_name: String,
    pub codec_tag: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: Option<String>,
    pub fps: Option<f64>,
    pub nb_frames: Option<u64>,
    pub duration_sec: Option<f64>,
    pub bit_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioStreamInfo {
    pub index: u32,
    pub codec_name: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub label: Option<String>,
    pub duration_sec: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProbeInfo {
    pub format_name: String,
    pub duration_sec: Option<f64>,
    pub size_bytes: Option<u64>,
    pub bit_rate: Option<f64>,
    pub video: Option<VideoStreamInfo>,
    pub audio: Vec<AudioStreamInfo>,
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

/// Parses ffprobe rationals like "30/1" or "30000/1001".
pub fn rational(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    (d != 0.0 && n > 0.0).then(|| n / d)
}

pub fn codec_from_name(name: &str) -> Option<VideoCodec> {
    match name {
        "h264" => Some(VideoCodec::H264),
        "hevc" | "h265" => Some(VideoCodec::H265),
        _ => None,
    }
}

const GENERIC_HANDLERS: &[&str] = &["SoundHandler", "VideoHandler", "Core Media Audio", "ISO Media file produced by Google Inc."];

fn label_of(stream: &Value) -> Option<String> {
    let tags = stream.get("tags")?;
    for key in ["title", "handler_name"] {
        if let Some(t) = tags.get(key).and_then(Value::as_str) {
            let t = t.trim();
            if !t.is_empty() && !GENERIC_HANDLERS.contains(&t) {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Parses `ffprobe -print_format json -show_format -show_streams` output.
pub fn parse_ffprobe_json(text: &str) -> CaptureResult<ProbeInfo> {
    let v: Value = serde_json::from_str(text).map_err(|e| CaptureError::internal(format!("invalid ffprobe JSON: {e}")))?;
    let format = v.get("format").cloned().unwrap_or(Value::Null);
    let mut info = ProbeInfo {
        format_name: format.get("format_name").and_then(Value::as_str).unwrap_or_default().to_string(),
        duration_sec: format.get("duration").and_then(num),
        size_bytes: format.get("size").and_then(num).map(|f| f as u64),
        bit_rate: format.get("bit_rate").and_then(num),
        ..Default::default()
    };
    let streams = v.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    for st in &streams {
        let index = st.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
        let codec_name = st.get("codec_name").and_then(Value::as_str).unwrap_or_default().to_string();
        match st.get("codec_type").and_then(Value::as_str) {
            Some("video") if info.video.is_none() => {
                // Skip attached pictures (cover art).
                if st.get("disposition").and_then(|d| d.get("attached_pic")).and_then(Value::as_u64) == Some(1) {
                    continue;
                }
                let fps = st
                    .get("avg_frame_rate")
                    .and_then(Value::as_str)
                    .and_then(rational)
                    .or_else(|| st.get("r_frame_rate").and_then(Value::as_str).and_then(rational));
                info.video = Some(VideoStreamInfo {
                    index,
                    codec_name,
                    codec_tag: st.get("codec_tag_string").and_then(Value::as_str).unwrap_or_default().to_string(),
                    width: st.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                    height: st.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
                    pix_fmt: st.get("pix_fmt").and_then(Value::as_str).map(str::to_string),
                    fps,
                    nb_frames: st.get("nb_frames").and_then(num).map(|f| f as u64),
                    duration_sec: st.get("duration").and_then(num),
                    bit_rate: st.get("bit_rate").and_then(num),
                });
            }
            Some("audio") => info.audio.push(AudioStreamInfo {
                index,
                codec_name,
                channels: st.get("channels").and_then(Value::as_u64).unwrap_or(0) as u32,
                sample_rate: st.get("sample_rate").and_then(num).unwrap_or(0.0) as u32,
                label: label_of(st),
                duration_sec: st.get("duration").and_then(num),
            }),
            _ => {}
        }
    }
    Ok(info)
}

pub fn run_ffprobe(ffprobe: &Path, file: &Path) -> CaptureResult<ProbeInfo> {
    let mut cmd = new_command(ffprobe);
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(file);
    let out = run_with_timeout(cmd, Duration::from_secs(60))?;
    if !out.status.success() {
        return Err(CaptureError::new(CaptureErrorCode::EncoderFailed, format!("ffprobe could not read {}: {}", file.display(), out.stderr_text())));
    }
    parse_ffprobe_json(&String::from_utf8_lossy(&out.stdout))
}

/// What a produced file is expected to contain.
#[derive(Debug, Clone, Default)]
pub struct Expectation {
    pub duration_sec: Option<f64>,
    pub duration_tolerance_sec: f64,
    pub size: Option<Size>,
    pub codec: Option<VideoCodec>,
    pub audio_streams: Option<usize>,
}

/// Default tolerance: 0.5 s or 2 %, whichever is larger.
pub fn default_tolerance(expected_sec: f64) -> f64 {
    (expected_sec * 0.02).max(0.5)
}

fn issue(severity: IssueSeverity, code: &str, message: String) -> ValidationIssue {
    ValidationIssue { severity, code: code.to_string(), message }
}

/// Compares probe results with expectations. `valid` is false if any error-level issue exists.
pub fn validate(info: &ProbeInfo, file_size: u64, exp: &Expectation) -> MediaValidation {
    use IssueSeverity::{Error, Warning};
    let mut issues = Vec::new();
    if file_size == 0 {
        issues.push(issue(Error, "EMPTY_FILE", "file is empty".into()));
    }
    match &info.video {
        None => issues.push(issue(Error, "NO_VIDEO_STREAM", "no video stream found".into())),
        Some(v) => {
            if let Some(sz) = exp.size {
                if v.width != sz.width || v.height != sz.height {
                    issues.push(issue(Error, "DIMENSION_MISMATCH", format!("video is {}x{}, expected {}x{}", v.width, v.height, sz.width, sz.height)));
                }
            }
            if v.width == 0 || v.height == 0 {
                issues.push(issue(Error, "ZERO_DIMENSIONS", "video stream has zero dimensions".into()));
            }
            if let Some(c) = exp.codec {
                match codec_from_name(&v.codec_name) {
                    Some(actual) if actual == c => {
                        if c == VideoCodec::H265 && v.codec_tag != "hvc1" {
                            issues.push(issue(Warning, "HEVC_TAG", format!("HEVC stream tagged '{}' instead of 'hvc1' (QuickTime may refuse it)", v.codec_tag)));
                        }
                    }
                    _ => issues.push(issue(Error, "CODEC_MISMATCH", format!("video codec is '{}', expected {:?}", v.codec_name, c))),
                }
            }
            if v.nb_frames == Some(0) {
                issues.push(issue(Error, "NO_FRAMES", "video stream contains no frames".into()));
            }
        }
    }
    let duration = info.duration_sec.or_else(|| info.video.as_ref().and_then(|v| v.duration_sec));
    match (duration, exp.duration_sec) {
        (None, _) => issues.push(issue(Error, "NO_DURATION", "duration could not be determined".into())),
        (Some(d), _) if d <= 0.0 => issues.push(issue(Error, "ZERO_DURATION", "duration is zero".into())),
        (Some(d), Some(e)) if (d - e).abs() > exp.duration_tolerance_sec => issues.push(issue(
            Error,
            "DURATION_MISMATCH",
            format!("duration {d:.3}s differs from recorded time {e:.3}s by more than {:.2}s", exp.duration_tolerance_sec),
        )),
        _ => {}
    }
    if let Some(n) = exp.audio_streams {
        if info.audio.len() != n {
            issues.push(issue(Error, "AUDIO_STREAM_COUNT", format!("found {} audio stream(s), expected {n}", info.audio.len())));
        }
    }
    for a in &info.audio {
        if a.codec_name != "aac" {
            issues.push(issue(Warning, "AUDIO_CODEC", format!("audio stream {} uses '{}' (expected aac)", a.index, a.codec_name)));
        }
    }
    MediaValidation {
        valid: !issues.iter().any(|i| i.severity == IssueSeverity::Error),
        checked_at: capture_core::now_iso8601(),
        tool: "ffprobe".into(),
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SAMPLE: &str = r#"{
      "streams": [
        {"index":0,"codec_name":"h264","codec_type":"video","codec_tag_string":"avc1","width":1920,"height":1080,"pix_fmt":"yuv420p",
         "r_frame_rate":"30/1","avg_frame_rate":"30/1","duration":"5.000000","bit_rate":"812345","nb_frames":"150",
         "disposition":{"default":1,"attached_pic":0},"tags":{"handler_name":"VideoHandler"}},
        {"index":1,"codec_name":"aac","codec_type":"audio","sample_rate":"48000","channels":2,"duration":"5.013333",
         "tags":{"handler_name":"Microphone","title":"Microphone"}},
        {"index":2,"codec_name":"aac","codec_type":"audio","sample_rate":"48000","channels":2,
         "tags":{"handler_name":"SoundHandler"}}
      ],
      "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"5.013333","size":"512000","bit_rate":"817000"}
    }"#;

    #[test]
    fn parses_streams_and_format() {
        let p = parse_ffprobe_json(SAMPLE).unwrap();
        let v = p.video.as_ref().unwrap();
        assert_eq!((v.width, v.height, v.codec_name.as_str(), v.nb_frames), (1920, 1080, "h264", Some(150)));
        assert_eq!(v.fps, Some(30.0));
        assert_eq!(p.audio.len(), 2);
        assert_eq!(p.audio[0].label.as_deref(), Some("Microphone"));
        assert_eq!(p.audio[1].label, None, "generic handler names are not labels");
        assert_eq!(p.audio[0].sample_rate, 48000);
        assert!((p.duration_sec.unwrap() - 5.013333).abs() < 1e-6);
        assert_eq!(p.size_bytes, Some(512000));
    }

    #[test]
    fn rationals() {
        assert_eq!(rational("30/1"), Some(30.0));
        assert!((rational("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(rational("0/0"), None);
        assert_eq!(rational("garbage"), None);
    }

    #[test]
    fn validation_passes_and_fails() {
        let p = parse_ffprobe_json(SAMPLE).unwrap();
        let ok = validate(
            &p,
            512000,
            &Expectation { duration_sec: Some(5.0), duration_tolerance_sec: 0.4, size: Some(Size { width: 1920, height: 1080 }), codec: Some(VideoCodec::H264), audio_streams: Some(2) },
        );
        assert!(ok.valid, "{:?}", ok.issues);
        let bad = validate(
            &p,
            512000,
            &Expectation { duration_sec: Some(8.0), duration_tolerance_sec: 0.4, size: Some(Size { width: 1280, height: 720 }), codec: Some(VideoCodec::H265), audio_streams: Some(1) },
        );
        assert!(!bad.valid);
        let codes: Vec<_> = bad.issues.iter().map(|i| i.code.as_str()).collect();
        for c in ["DURATION_MISMATCH", "DIMENSION_MISMATCH", "CODEC_MISMATCH", "AUDIO_STREAM_COUNT"] {
            assert!(codes.contains(&c), "missing {c} in {codes:?}");
        }
        let empty = validate(&p, 0, &Expectation::default());
        assert!(!empty.valid);
    }

    #[test]
    fn hevc_tag_warning_and_missing_video() {
        let j = SAMPLE.replace("\"h264\"", "\"hevc\"").replace("avc1", "hev1");
        let p = parse_ffprobe_json(&j).unwrap();
        let v = validate(&p, 1, &Expectation { codec: Some(VideoCodec::H265), ..Default::default() });
        assert!(v.valid);
        assert!(v.issues.iter().any(|i| i.code == "HEVC_TAG" && i.severity == IssueSeverity::Warning));
        let none = parse_ffprobe_json(r#"{"streams":[],"format":{"duration":"1.0"}}"#).unwrap();
        assert!(!validate(&none, 10, &Expectation::default()).valid);
        assert!(parse_ffprobe_json("not json").is_err());
    }

    #[test]
    fn tolerance() {
        assert_eq!(default_tolerance(5.0), 0.5);
        assert!((default_tolerance(100.0) - 2.0).abs() < 1e-9);
    }
}
