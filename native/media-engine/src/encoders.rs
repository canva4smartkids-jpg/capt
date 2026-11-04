//! Video encoder discovery (ffmpeg `-encoders` + a real test encode per candidate),
//! selection according to `hardwareAcceleration`, and per-encoder ffmpeg arguments.

use crate::tools::{new_command, run_with_timeout};
use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, EncoderInfo, HardwareAcceleration, VideoCodec};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub name: &'static str,
    pub codec: VideoCodec,
    pub hardware: bool,
}

const fn hw(name: &'static str, codec: VideoCodec) -> Candidate {
    Candidate { name, codec, hardware: true }
}
const fn sw(name: &'static str, codec: VideoCodec) -> Candidate {
    Candidate { name, codec, hardware: false }
}

use VideoCodec::{H264, H265};

/// All candidates in preference order for the current OS (hardware first, then software).
pub fn candidates() -> Vec<Candidate> {
    let hw_order: &[Candidate] = if cfg!(target_os = "windows") {
        &[
            hw("h264_nvenc", H264),
            hw("h264_qsv", H264),
            hw("h264_amf", H264),
            hw("h264_mf", H264),
            hw("hevc_nvenc", H265),
            hw("hevc_qsv", H265),
            hw("hevc_amf", H265),
            hw("hevc_mf", H265),
        ]
    } else if cfg!(target_os = "macos") {
        &[hw("h264_videotoolbox", H264), hw("hevc_videotoolbox", H265)]
    } else {
        &[
            hw("h264_nvenc", H264),
            hw("h264_vaapi", H264),
            hw("h264_qsv", H264),
            hw("h264_amf", H264),
            hw("hevc_nvenc", H265),
            hw("hevc_vaapi", H265),
            hw("hevc_qsv", H265),
            hw("hevc_amf", H265),
        ]
    };
    let mut v = hw_order.to_vec();
    v.extend_from_slice(&[sw("libx264", H264), sw("libopenh264", H264), sw("libx265", H265)]);
    v
}

/// Parses `ffmpeg -hide_banner -encoders` output into the set of encoder names.
pub fn parse_encoder_list(stdout: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    let mut in_list = false;
    for line in stdout.lines() {
        let t = line.trim();
        if t.starts_with("------") {
            in_list = true;
            continue;
        }
        if !in_list {
            continue;
        }
        let mut parts = t.split_whitespace();
        if let (Some(flags), Some(name)) = (parts.next(), parts.next()) {
            if flags.len() == 6 && flags.starts_with('V') {
                set.insert(name.to_string());
            }
        }
    }
    set
}

pub fn is_hevc(encoder: &str) -> bool {
    encoder.starts_with("hevc") || encoder == "libx265"
}

/// Pixel format fed to the encoder.
pub fn encoder_pix_fmt(encoder: &str) -> &'static str {
    match encoder {
        "libx264" | "libx265" | "libopenh264" => "yuv420p",
        _ => "nv12",
    }
}

/// Global args placed before `-i` (device setup for VAAPI).
pub fn encoder_pre_input_args(encoder: &str) -> Vec<String> {
    if encoder.ends_with("_vaapi") {
        let dev = std::env::var("CAPTUREONE_VAAPI_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
        vec!["-vaapi_device".into(), dev]
    } else {
        Vec::new()
    }
}

/// Tail of the `-vf` chain converting to the encoder's input format.
pub fn encoder_filter_tail(encoder: &str) -> String {
    if encoder.ends_with("_vaapi") {
        "format=nv12,hwupload".into()
    } else {
        format!("format={}", encoder_pix_fmt(encoder))
    }
}

#[derive(Debug, Clone)]
pub struct VideoEncodeParams {
    pub encoder: String,
    pub bitrate_kbps: u32,
    pub fps: u32,
    pub keyframe_interval_sec: f64,
    /// width*height*fps of the encoded stream, used to pick software presets.
    pub pixel_rate: u64,
}

/// `-c:v ...` and rate-control arguments for one encoder (no input/output/filter args).
pub fn encoder_args(p: &VideoEncodeParams) -> Vec<String> {
    let b = p.bitrate_kbps.max(1);
    let gop = ((p.fps as f64 * p.keyframe_interval_sec).round() as u32).max(1);
    let heavy = p.pixel_rate > 1920 * 1080 * 30;
    let mut a: Vec<String> = vec!["-c:v".into(), p.encoder.clone()];
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    let bv = format!("{b}k");
    let maxrate = format!("{}k", b + b / 2);
    let bufsize = format!("{}k", b * 2);
    let g = gop.to_string();
    match p.encoder.as_str() {
        "libx264" => {
            push(&["-preset", if heavy { "superfast" } else { "veryfast" }, "-profile:v", "high"]);
            push(&["-b:v", &bv, "-maxrate", &maxrate, "-bufsize", &bufsize, "-g", &g, "-keyint_min", &g, "-sc_threshold", "0"]);
        }
        "libx265" => {
            push(&["-preset", if p.pixel_rate > 1280 * 720 * 30 { "ultrafast" } else { "superfast" }]);
            push(&["-b:v", &bv, "-maxrate", &maxrate, "-bufsize", &bufsize, "-g", &g]);
            let params = format!("log-level=error:keyint={gop}:min-keyint={gop}:scenecut=0");
            push(&["-x265-params", &params]);
        }
        "libopenh264" => push(&["-b:v", &bv, "-maxrate", &maxrate, "-g", &g]),
        e if e.ends_with("_nvenc") => push(&["-preset", "p4", "-rc", "vbr", "-b:v", &bv, "-maxrate", &maxrate, "-bufsize", &bufsize, "-g", &g]),
        e if e.ends_with("_qsv") => push(&["-preset", "veryfast", "-b:v", &bv, "-maxrate", &maxrate, "-bufsize", &bufsize, "-g", &g]),
        e if e.ends_with("_amf") => push(&["-usage", "transcoding", "-quality", "speed", "-rc", "vbr_peak", "-b:v", &bv, "-maxrate", &maxrate, "-g", &g]),
        e if e.ends_with("_mf") => push(&["-hw_encoding", "1", "-rate_control", "u_vbr", "-b:v", &bv, "-g", &g]),
        e if e.ends_with("_videotoolbox") => push(&["-realtime", "1", "-allow_sw", "0", "-b:v", &bv, "-maxrate", &maxrate, "-bufsize", &bufsize, "-g", &g]),
        e if e.ends_with("_vaapi") => push(&["-rc_mode", "VBR", "-b:v", &bv, "-maxrate", &maxrate, "-g", &g]),
        _ => push(&["-b:v", &bv, "-g", &g]),
    }
    a.extend(["-pix_fmt".to_string(), encoder_pix_fmt(&p.encoder).to_string()]);
    a
}

/// Runs a real encode of 5 synthetic 256x256 BGRA frames through the same rawvideo
/// stdin path used for recording. `Ok(())` means the encoder produced output.
pub fn test_encode(ffmpeg: &Path, encoder: &str) -> CaptureResult<()> {
    const W: usize = 256;
    const H: usize = 256;
    const FRAMES: usize = 5;
    let mut cmd = new_command(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    cmd.args(encoder_pre_input_args(encoder));
    cmd.args(["-f", "rawvideo", "-pix_fmt", "bgra", "-s", &format!("{W}x{H}"), "-r", "30", "-i", "-"]);
    cmd.args(["-vf", &encoder_filter_tail(encoder)]);
    cmd.args(encoder_args(&VideoEncodeParams {
        encoder: encoder.to_string(),
        bitrate_kbps: 1000,
        fps: 30,
        keyframe_interval_sec: 1.0,
        pixel_rate: (W * H * 30) as u64,
    }));
    cmd.args(["-f", "null", "-"]);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| CaptureError::new(CaptureErrorCode::EncoderUnavailable, format!("cannot start ffmpeg: {e}")))?;
    let stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            use std::io::Read;
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    if let Some(mut stdin) = child.stdin.take() {
        let mut frame = vec![0u8; W * H * 4];
        for i in 0..FRAMES {
            for (j, px) in frame.chunks_exact_mut(4).enumerate() {
                let (x, y) = (j % W, j / W);
                px.copy_from_slice(&[(x + i * 7) as u8, (y + i * 3) as u8, ((x ^ y) as u8).wrapping_add(i as u8), 255]);
            }
            if stdin.write_all(&frame).is_err() {
                break; // encoder died; status below explains why
            }
        }
    }
    let status = crate::tools::wait_child(&mut child, Duration::from_secs(20))?;
    let err = err_thread.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(CaptureError::new(CaptureErrorCode::EncoderUnavailable, format!("test encode with {encoder} failed: {}", err.trim())))
    }
}

/// Lists candidate encoders known to this ffmpeg build and verifies each with a test encode
/// (in parallel). Unverified candidates are reported with `verified: false`.
pub fn probe_encoders(ffmpeg: &Path) -> Vec<EncoderInfo> {
    if let Some(cached) = cache::load(ffmpeg) {
        return cached;
    }
    let mut cmd = new_command(ffmpeg);
    cmd.args(["-hide_banner", "-encoders"]);
    let listed = match run_with_timeout(cmd, Duration::from_secs(20)) {
        Ok(out) if out.status.success() => parse_encoder_list(&String::from_utf8_lossy(&out.stdout)),
        _ => return Vec::new(),
    };
    let cands: Vec<Candidate> = candidates().into_iter().filter(|c| listed.contains(c.name)).collect();
    let started = Instant::now();
    let results: Vec<EncoderInfo> = std::thread::scope(|s| {
        let handles: Vec<_> = cands
            .iter()
            .map(|c| {
                let c = *c;
                s.spawn(move || {
                    let res = test_encode(ffmpeg, c.name);
                    if let Err(e) = &res {
                        log::info!("encoder {} not usable: {}", c.name, e.message);
                    }
                    EncoderInfo { codec: c.codec, name: c.name.to_string(), hardware: c.hardware, verified: res.is_ok() }
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(cands.iter())
            .map(|(h, c)| h.join().unwrap_or(EncoderInfo { codec: c.codec, name: c.name.to_string(), hardware: c.hardware, verified: false }))
            .collect()
    });
    log::info!("encoder probe took {:?}", started.elapsed());
    cache::store(ffmpeg, &results);
    results
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncoderChoice {
    pub encoder: EncoderInfo,
    pub warning: Option<String>,
}

/// Picks an encoder from `encoders` (in preference order) for `codec`:
/// * `off`    — verified software encoder only;
/// * `auto`   — verified hardware encoder if any, else software (silently);
/// * `prefer` — hardware if verified, else software with a warning.
pub fn select_encoder(codec: VideoCodec, accel: HardwareAcceleration, encoders: &[EncoderInfo]) -> CaptureResult<EncoderChoice> {
    let pick = |hardware: bool| encoders.iter().find(|e| e.verified && e.codec == codec && e.hardware == hardware).cloned();
    let codec_name = match codec {
        H264 => "H.264",
        H265 => "H.265",
    };
    let none = || {
        CaptureError::new(CaptureErrorCode::EncoderUnavailable, format!("no working {codec_name} encoder found in ffmpeg (all candidates failed a test encode)"))
            .with_detail("codec", serde_json::to_value(codec).unwrap_or_default())
    };
    match accel {
        HardwareAcceleration::Off => pick(false).map(|e| EncoderChoice { encoder: e, warning: None }).ok_or_else(none),
        HardwareAcceleration::Auto => pick(true).or_else(|| pick(false)).map(|e| EncoderChoice { encoder: e, warning: None }).ok_or_else(none),
        HardwareAcceleration::Prefer => match pick(true) {
            Some(e) => Ok(EncoderChoice { encoder: e, warning: None }),
            None => pick(false)
                .map(|e| EncoderChoice {
                    warning: Some(format!("no working hardware {codec_name} encoder; falling back to software encoder {}", e.name)),
                    encoder: e,
                })
                .ok_or_else(none),
        },
    }
}

/// On-disk cache of probe results (24 h), keyed by ffmpeg path, size and mtime, so the CLI
/// and app do not re-run test encodes on every launch. Disable with CAPTUREONE_NO_ENCODER_CACHE=1.
mod cache {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Entry {
        key: String,
        checked_at: u64,
        encoders: Vec<EncoderInfo>,
    }

    const TTL_SECS: u64 = 24 * 3600;

    fn key(ffmpeg: &Path) -> Option<String> {
        let md = std::fs::metadata(ffmpeg).ok()?;
        let mtime = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        Some(format!("{}|{}|{}", ffmpeg.display(), md.len(), mtime))
    }

    fn file() -> std::path::PathBuf {
        std::env::temp_dir().join("captureone").join("encoder-probe-v1.json")
    }

    fn now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    fn disabled() -> bool {
        std::env::var("CAPTUREONE_NO_ENCODER_CACHE").map(|v| v == "1").unwrap_or(false)
    }

    pub fn load(ffmpeg: &Path) -> Option<Vec<EncoderInfo>> {
        if disabled() {
            return None;
        }
        let k = key(ffmpeg)?;
        let entries: Vec<Entry> = serde_json::from_slice(&std::fs::read(file()).ok()?).ok()?;
        entries.into_iter().find(|e| e.key == k && now().saturating_sub(e.checked_at) < TTL_SECS).map(|e| e.encoders)
    }

    pub fn store(ffmpeg: &Path, encoders: &[EncoderInfo]) {
        if disabled() || encoders.is_empty() {
            return;
        }
        let Some(k) = key(ffmpeg) else { return };
        let path = file();
        let mut entries: Vec<Entry> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        entries.retain(|e| e.key != k && now().saturating_sub(e.checked_at) < TTL_SECS);
        entries.push(Entry { key: k, checked_at: now(), encoders: encoders.to_vec() });
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&entries) {
            let tmp = path.with_extension(format!("tmp{}", std::process::id()));
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(name: &str, codec: VideoCodec, hardware: bool, verified: bool) -> EncoderInfo {
        EncoderInfo { codec, name: name.into(), hardware, verified }
    }

    #[test]
    fn parses_encoder_listing() {
        let out = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264 (codec h264)\n V..... h264_qsv             H.264 QSV\n A....D aac                  AAC\n V....D libx265              libx265 H.265\n";
        let set = parse_encoder_list(out);
        assert!(set.contains("libx264") && set.contains("h264_qsv") && set.contains("libx265"));
        assert!(!set.contains("aac"), "audio encoders are excluded");
        assert!(!set.contains("V....."), "legend lines are excluded");
    }

    #[test]
    fn selection_auto_prefers_verified_hardware() {
        let list = vec![enc("h264_nvenc", H264, true, false), enc("h264_qsv", H264, true, true), enc("libx264", H264, false, true)];
        let c = select_encoder(H264, HardwareAcceleration::Auto, &list).unwrap();
        assert_eq!(c.encoder.name, "h264_qsv");
        assert!(c.warning.is_none());
    }

    #[test]
    fn selection_auto_falls_back_silently() {
        let list = vec![enc("h264_nvenc", H264, true, false), enc("libx264", H264, false, true)];
        let c = select_encoder(H264, HardwareAcceleration::Auto, &list).unwrap();
        assert_eq!(c.encoder.name, "libx264");
        assert!(c.warning.is_none());
    }

    #[test]
    fn selection_prefer_warns_on_fallback() {
        let list = vec![enc("hevc_nvenc", H265, true, false), enc("libx265", H265, false, true)];
        let c = select_encoder(H265, HardwareAcceleration::Prefer, &list).unwrap();
        assert_eq!(c.encoder.name, "libx265");
        assert!(c.warning.unwrap().contains("falling back"));
    }

    #[test]
    fn selection_off_never_uses_hardware() {
        let list = vec![enc("h264_qsv", H264, true, true), enc("libopenh264", H264, false, true)];
        assert_eq!(select_encoder(H264, HardwareAcceleration::Off, &list).unwrap().encoder.name, "libopenh264");
        let hw_only = vec![enc("h264_qsv", H264, true, true)];
        let err = select_encoder(H264, HardwareAcceleration::Off, &hw_only).unwrap_err();
        assert_eq!(err.code, CaptureErrorCode::EncoderUnavailable);
    }

    #[test]
    fn selection_respects_codec() {
        let list = vec![enc("libx264", H264, false, true)];
        assert!(select_encoder(H265, HardwareAcceleration::Auto, &list).is_err());
    }

    #[test]
    fn x264_args() {
        let a = encoder_args(&VideoEncodeParams { encoder: "libx264".into(), bitrate_kbps: 6000, fps: 30, keyframe_interval_sec: 2.0, pixel_rate: 1920 * 1080 * 30 });
        let s = a.join(" ");
        assert!(s.contains("-c:v libx264"));
        assert!(s.contains("-preset veryfast"));
        assert!(s.contains("-b:v 6000k -maxrate 9000k -bufsize 12000k"));
        assert!(s.contains("-g 60 -keyint_min 60"));
        assert!(s.ends_with("-pix_fmt yuv420p"));
        let heavy = encoder_args(&VideoEncodeParams { encoder: "libx264".into(), bitrate_kbps: 6000, fps: 60, keyframe_interval_sec: 2.0, pixel_rate: 1920 * 1080 * 60 });
        assert!(heavy.join(" ").contains("-preset superfast") && heavy.join(" ").contains("-g 120"));
    }

    #[test]
    fn hw_args_use_nv12_and_vaapi_upload() {
        let a = encoder_args(&VideoEncodeParams { encoder: "h264_nvenc".into(), bitrate_kbps: 1000, fps: 60, keyframe_interval_sec: 1.0, pixel_rate: 1 });
        assert!(a.join(" ").ends_with("-pix_fmt nv12"));
        assert_eq!(encoder_filter_tail("hevc_vaapi"), "format=nv12,hwupload");
        assert_eq!(encoder_pre_input_args("h264_vaapi")[0], "-vaapi_device");
        assert!(encoder_pre_input_args("libx264").is_empty());
        assert!(is_hevc("hevc_videotoolbox") && is_hevc("libx265") && !is_hevc("h264_mf"));
    }

    #[test]
    fn candidate_order_has_software_last() {
        let c = candidates();
        let first_sw = c.iter().position(|c| !c.hardware).unwrap();
        assert!(c[first_sw..].iter().all(|c| !c.hardware));
        assert!(c.iter().any(|c| c.name == "libx264") && c.iter().any(|c| c.name == "libx265"));
    }
}
