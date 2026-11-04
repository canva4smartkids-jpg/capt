//! captureone-cli: headless front-end of the CaptureOne engine, used by CI to verify real
//! capture and recording on every desktop OS. Results are JSON on stdout; progress events
//! are JSON lines on stderr.
//!
//! Exit codes: 0 success (and media valid), 1 error, 2 usage error, 3 media failed validation.

mod args;

use args::{parse_target, Args};
use capture_core::*;
use media_engine::{EngineConfig, MediaEngine, RecorderListener};
use parking_lot::Mutex;
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const USAGE: &str = r#"captureone-cli <command> [options]

Commands:
  capabilities                      Capability report (encoders verified by test encodes)
  ffmpeg-info                       Located ffmpeg/ffprobe and version
  sources [--kind display|window|device] Capture sources
  audio-devices                     Microphones and loopback sources
  thumbnail --kind display|window|device --id <id> [--max-width 320]
  screenshot --target <t> [--format png|jpeg] [--quality 90] [--no-cursor]
             [--template <name>] --out <dir>
  record --target <t> [--duration <sec>] [--fps 24|30|60] [--codec h264|h265]
         [--quality low|medium|high|ultra] [--bitrate <kbps>] [--height 2160|1440|1080|720|480]
         [--size <w>x<h>] [--keyframe <sec>] [--hw auto|prefer|off] [--no-cursor]
         [--mic] [--mic-device <id>] [--mic-gain <0..2>] [--system-audio] [--system-gain <0..2>]
         [--tracks mixed|separate] [--sample-rate 44100|48000] [--audio-kbps 160]
         [--pause-at <sec> --resume-at <sec>] [--max-duration <sec>] [--max-size-mb <mb>]
         [--min-free-mb <mb>] [--template <name>] [--cancel-at <sec>] --out <dir>
  inspect <file>                    ffprobe metadata + validation of a file
  recover --out <dir>               Finalize interrupted sessions into playable MP4s

Targets: display | display:<id> | window:<id> | device:<id> (see `sources --kind device`) | region:<display>:<x>,<y>,<w>,<h> | region:<x>,<y>,<w>,<h>
--duration is the recorded (active) time; with --pause-at/--resume-at (wall-clock seconds
since start) the wall time is extended by the pause. Without --duration, recording runs
until Ctrl-C / SIGTERM or a limit is reached.
Environment: CAPTUREONE_FFMPEG / CAPTUREONE_FFPROBE override tool paths; CAPTUREONE_LOG=info|debug
"#;

fn create_backend() -> CaptureResult<Arc<dyn CaptureBackend>> {
    #[cfg(target_os = "linux")]
    {
        linux_capture::create_backend()
    }
    #[cfg(target_os = "windows")]
    {
        windows_capture_backend::create_backend()
    }
    #[cfg(target_os = "macos")]
    {
        macos_capture::create_backend()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(CaptureError::unsupported("no capture backend for this OS"))
    }
}

struct StderrLogger(log::LevelFilter);

impl log::Log for StderrLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= self.0
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("{}", json!({"event": "log", "level": r.level().to_string(), "target": r.target(), "message": r.args().to_string()}));
        }
    }
    fn flush(&self) {}
}

fn init_logging() {
    let level = match std::env::var("CAPTUREONE_LOG").unwrap_or_default().as_str() {
        "debug" => log::LevelFilter::Debug,
        "info" => log::LevelFilter::Info,
        "warn" => log::LevelFilter::Warn,
        _ => return,
    };
    let logger: &'static StderrLogger = Box::leak(Box::new(StderrLogger(level)));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

fn print_json<T: serde::Serialize>(v: &T) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("{}", json!({"event": "error", "message": format!("serialization failed: {e}")})),
    }
}

fn fail(e: &CaptureError) -> ExitCode {
    eprintln!("{}", json!({"event": "error", "error": e}));
    print_json(&json!({"error": e}));
    ExitCode::from(1)
}

fn media_exit(m: &MediaMetadata) -> ExitCode {
    if m.validation.as_ref().map(|v| v.valid).unwrap_or(false) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}

fn engine() -> CaptureResult<MediaEngine> {
    MediaEngine::new(create_backend()?, EngineConfig::default())
}

fn main() -> ExitCode {
    init_logging();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    if matches!(cmd.as_str(), "-h" | "--help" | "help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match Args::parse(&argv[1..]) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let res = match cmd.as_str() {
        "capabilities" => engine().map(|e| {
            print_json(&e.capabilities());
            ExitCode::SUCCESS
        }),
        "ffmpeg-info" => engine().map(|e| {
            print_json(&e.ffmpeg_info());
            ExitCode::SUCCESS
        }),
        "sources" => cmd_sources(&args),
        "audio-devices" => engine().and_then(|e| e.list_audio_devices()).map(|d| {
            print_json(&d);
            ExitCode::SUCCESS
        }),
        "thumbnail" => cmd_thumbnail(&args),
        "screenshot" => cmd_screenshot(&args),
        "record" => cmd_record(&args),
        "inspect" => cmd_inspect(&args),
        "recover" => cmd_recover(&args),
        other => {
            eprintln!("error: unknown command '{other}'\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match res {
        Ok(code) => code,
        Err(e) if e.code == CaptureErrorCode::InvalidConfig && e.message.starts_with("usage:") => {
            eprintln!("error: {}\n\n{USAGE}", e.message);
            ExitCode::from(2)
        }
        Err(e) => fail(&e),
    }
}

fn usage(msg: impl Into<String>) -> CaptureError {
    CaptureError::invalid_config(format!("usage: {}", msg.into()))
}

fn cmd_sources(a: &Args) -> CaptureResult<ExitCode> {
    let kinds = match a.get("kind") {
        None => vec![SourceKind::Display, SourceKind::Window],
        Some("display") => vec![SourceKind::Display],
        Some("window") => vec![SourceKind::Window],
        Some("device") => vec![SourceKind::Device],
        Some(k) => return Err(usage(format!("unknown --kind '{k}'"))),
    };
    let e = engine()?;
    print_json(&e.list_sources(&kinds)?);
    Ok(ExitCode::SUCCESS)
}

fn cmd_thumbnail(a: &Args) -> CaptureResult<ExitCode> {
    let kind = match a.get("kind") {
        Some("display") | None => SourceKind::Display,
        Some("window") => SourceKind::Window,
        Some("device") => SourceKind::Device,
        Some(k) => return Err(usage(format!("unknown --kind '{k}'"))),
    };
    let id = a.get("id").ok_or_else(|| usage("--id is required"))?;
    let w: u32 = a.parse_or("max-width", 320).map_err(usage)?;
    let url = engine()?.source_thumbnail(kind, id, w)?;
    print_json(&json!({"dataUrl": url}));
    Ok(ExitCode::SUCCESS)
}

fn out_dir(a: &Args) -> CaptureResult<PathBuf> {
    a.get("out").map(PathBuf::from).ok_or_else(|| usage("--out <dir> is required"))
}

fn resolve_cli_target(e: &MediaEngine, a: &Args) -> CaptureResult<CaptureTarget> {
    let t = a.get("target").unwrap_or("display");
    let primary = || -> CaptureResult<String> {
        let list = e.backend().list_sources(&[SourceKind::Display])?;
        list.iter()
            .find(|s| s.is_primary == Some(true))
            .or_else(|| list.first())
            .map(|s| s.id.clone())
            .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "no display found"))
    };
    parse_target(t, &primary).map_err(|m| if m.starts_with("usage") { usage(m.trim_start_matches("usage: ")) } else { CaptureError::new(CaptureErrorCode::SourceNotFound, m) })
}

fn cmd_screenshot(a: &Args) -> CaptureResult<ExitCode> {
    let e = engine()?;
    let target = resolve_cli_target(&e, a)?;
    let format = match a.get("format").unwrap_or("png") {
        "png" => ImageFormat::Png,
        "jpeg" | "jpg" => ImageFormat::Jpeg,
        f => return Err(usage(format!("unknown --format '{f}'"))),
    };
    let req = ScreenshotRequest {
        target,
        format,
        jpeg_quality: a.parse_or("quality", 90).map_err(usage)?,
        output_directory: None,
        file_name_template: a.get("template").map(str::to_string),
        show_cursor: !a.flag("no-cursor"),
        delay_sec: 0,
    };
    let m = e.screenshot(&req, &out_dir(a)?)?;
    print_json(&m);
    Ok(media_exit(&m))
}

fn cmd_inspect(a: &Args) -> CaptureResult<ExitCode> {
    let file = a.positional.first().ok_or_else(|| usage("inspect <file>"))?;
    let m = engine()?.inspect(std::path::Path::new(file))?;
    print_json(&m);
    Ok(media_exit(&m))
}

fn cmd_recover(a: &Args) -> CaptureResult<ExitCode> {
    let dir = out_dir(a)?;
    let results = engine()?.recover_sessions(&dir);
    let mut code = ExitCode::SUCCESS;
    let out: Vec<serde_json::Value> = results
        .iter()
        .map(|r| match r {
            Ok(m) => {
                if !m.validation.as_ref().map(|v| v.valid).unwrap_or(false) && code == ExitCode::SUCCESS {
                    code = ExitCode::from(3);
                }
                json!({"ok": m})
            }
            Err(e) => {
                code = ExitCode::from(1);
                json!({"error": e})
            }
        })
        .collect();
    print_json(&out);
    Ok(code)
}

struct CliListener {
    done: Mutex<Option<StopReason>>,
}

fn emit(v: serde_json::Value) {
    eprintln!("{v}");
}

impl RecorderListener for CliListener {
    fn on_state(&self, session_id: &str, from: RecordingState, to: RecordingState) {
        emit(json!({"event": "state", "sessionId": session_id, "from": from, "to": to}));
    }
    fn on_status(&self, status: &RecordingStatus) {
        emit(json!({"event": "status", "status": status}));
    }
    fn on_warning(&self, session_id: &str, message: &str) {
        emit(json!({"event": "warning", "sessionId": session_id, "message": message}));
    }
    fn on_error(&self, session_id: &str, error: &CaptureError) {
        emit(json!({"event": "error", "sessionId": session_id, "error": error}));
    }
    fn on_completed(&self, session_id: &str, reason: StopReason, result: &CaptureResult<MediaMetadata>) {
        let (ok, valid) = match result {
            Ok(m) => (true, m.validation.as_ref().map(|v| v.valid).unwrap_or(false)),
            Err(_) => (false, false),
        };
        emit(json!({"event": "completed", "sessionId": session_id, "stopReason": reason, "ok": ok, "valid": valid}));
        *self.done.lock() = Some(reason);
    }
}

fn build_config(a: &Args, target: CaptureTarget, out: &std::path::Path) -> CaptureResult<RecordingConfig> {
    let codec = match a.get("codec").unwrap_or("h264") {
        "h264" => VideoCodec::H264,
        "h265" | "hevc" => VideoCodec::H265,
        c => return Err(usage(format!("unknown --codec '{c}'"))),
    };
    let quality = match a.get("quality").unwrap_or("medium") {
        "low" => QualityPreset::Low,
        "medium" => QualityPreset::Medium,
        "high" => QualityPreset::High,
        "ultra" => QualityPreset::Ultra,
        "custom" => QualityPreset::Custom,
        q => return Err(usage(format!("unknown --quality '{q}'"))),
    };
    let resolution = if let Some(sz) = a.get("size") {
        let (w, h) = sz.split_once('x').ok_or_else(|| usage("--size must be <w>x<h>"))?;
        ResolutionSetting::Custom { width: w.parse().map_err(|_| usage("bad --size width"))?, height: h.parse().map_err(|_| usage("bad --size height"))? }
    } else if let Some(h) = a.get("height") {
        ResolutionSetting::Height { height: h.parse().map_err(|_| usage("bad --height"))? }
    } else {
        ResolutionSetting::Source
    };
    let hw = match a.get("hw").unwrap_or("auto") {
        "auto" => HardwareAcceleration::Auto,
        "prefer" => HardwareAcceleration::Prefer,
        "off" => HardwareAcceleration::Off,
        h => return Err(usage(format!("unknown --hw '{h}'"))),
    };
    let track_mode = match a.get("tracks").unwrap_or("mixed") {
        "mixed" => TrackMode::Mixed,
        "separate" => TrackMode::Separate,
        t => return Err(usage(format!("unknown --tracks '{t}'"))),
    };
    let bitrate: Option<u32> = a.get("bitrate").map(|b| b.parse().map_err(|_| usage("bad --bitrate"))).transpose()?;
    let cfg = RecordingConfig {
        target,
        video: VideoSettings {
            codec,
            resolution,
            fps: a.parse_or("fps", 30).map_err(usage)?,
            quality: if bitrate.is_some() && a.get("quality").is_none() { QualityPreset::Custom } else { quality },
            bitrate_kbps: bitrate,
            keyframe_interval_sec: a.parse_or("keyframe", 2.0).map_err(usage)?,
            hardware_acceleration: hw,
            show_cursor: !a.flag("no-cursor"),
        },
        audio: AudioSettings {
            microphone: AudioSourceSettings { enabled: a.flag("mic"), device_id: a.get("mic-device").map(str::to_string), gain: a.parse_or("mic-gain", 1.0).map_err(usage)? },
            system: AudioSourceSettings { enabled: a.flag("system-audio"), device_id: a.get("system-device").map(str::to_string), gain: a.parse_or("system-gain", 1.0).map_err(usage)? },
            track_mode,
            sample_rate: a.parse_or("sample-rate", 48_000).map_err(usage)?,
            bitrate_kbps: a.parse_or("audio-kbps", 160).map_err(usage)?,
        },
        output: OutputSettings {
            directory: out.to_string_lossy().to_string(),
            file_name_template: a.get("template").unwrap_or("CaptureOne {date} {time}").to_string(),
            container: "mp4".into(),
        },
        limits: RecordingLimits {
            max_duration_sec: a.get("max-duration").map(|v| v.parse().map_err(|_| usage("bad --max-duration"))).transpose()?,
            max_file_size_mb: a.get("max-size-mb").map(|v| v.parse().map_err(|_| usage("bad --max-size-mb"))).transpose()?,
            min_free_disk_mb: a.parse_or("min-free-mb", 500).map_err(usage)?,
        },
        countdown_sec: 0,
    };
    cfg.validate()?;
    Ok(cfg)
}

fn cmd_record(a: &Args) -> CaptureResult<ExitCode> {
    let e = engine()?;
    let target = resolve_cli_target(&e, a)?;
    let out = out_dir(a)?;
    let cfg = build_config(a, target, &out)?;
    let duration: Option<f64> = a.get("duration").map(|v| v.parse().map_err(|_| usage("bad --duration"))).transpose()?;
    let pause_at: Option<f64> = a.get("pause-at").map(|v| v.parse().map_err(|_| usage("bad --pause-at"))).transpose()?;
    let resume_at: Option<f64> = a.get("resume-at").map(|v| v.parse().map_err(|_| usage("bad --resume-at"))).transpose()?;
    if let (Some(p), Some(r)) = (pause_at, resume_at) {
        if r <= p {
            return Err(usage("--resume-at must be after --pause-at"));
        }
    }
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let flag = interrupted.clone();
        let _ = ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst));
    }
    let listener = Arc::new(CliListener { done: Mutex::new(None) });
    let session = e.start_recording(cfg, listener.clone())?;
    let t0 = Instant::now();
    let pause_len = match (pause_at, resume_at) {
        (Some(p), Some(r)) => r - p,
        (Some(_), None) => f64::INFINITY,
        _ => 0.0,
    };
    let stop_at = duration.map(|d| if pause_at.map(|p| p < d + 1e-9).unwrap_or(false) { d + pause_len } else { d });
    let cancel_at: Option<f64> = a.get("cancel-at").map(|v| v.parse().map_err(|_| usage("bad --cancel-at"))).transpose()?;
    let (mut paused, mut resumed) = (false, false);
    loop {
        if let Some(c) = cancel_at {
            if t0.elapsed().as_secs_f64() >= c {
                session.cancel()?;
                emit(json!({"event": "final", "cancelled": true, "wallMs": t0.elapsed().as_millis() as u64}));
                print_json(&json!({"cancelled": true, "sessionId": session.id()}));
                return Ok(ExitCode::SUCCESS);
            }
        }
        if listener.done.lock().is_some() || interrupted.load(Ordering::SeqCst) {
            break;
        }
        let el = t0.elapsed().as_secs_f64();
        if let Some(p) = pause_at {
            if !paused && el >= p {
                paused = true;
                if let Err(err) = session.pause() {
                    emit(json!({"event": "warning", "message": format!("pause failed: {}", err.message)}));
                }
            }
        }
        if let Some(r) = resume_at {
            if paused && !resumed && el >= r {
                resumed = true;
                if let Err(err) = session.resume() {
                    emit(json!({"event": "warning", "message": format!("resume failed: {}", err.message)}));
                }
            }
        }
        if stop_at.map(|s| el >= s).unwrap_or(false) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let res = session.stop();
    let st = session.status();
    emit(json!({"event": "final", "stopReason": *listener.done.lock(), "framesEncoded": st.frames_encoded, "framesDropped": st.frames_dropped, "elapsedMs": st.elapsed_ms, "wallMs": t0.elapsed().as_millis() as u64}));
    let m = res?;
    print_json(&m);
    Ok(media_exit(&m))
}
