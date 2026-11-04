//! Development/E2E fixture: runs a companion hub over a directory of real media files so the
//! TV web app can be exercised without the desktop app.
//!
//! ```text
//! cargo run -p companion-hub --example demo_hub -- --media-dir artifacts/samples --port 47800
//! ```
//!
//! Library entries are probed with ffprobe (dimensions, duration, codec) and thumbnails are
//! extracted with ffmpeg when available. Remote commands drive a *simulated* recording state
//! machine (nothing is captured); every status it emits carries the warning
//! "SIMULATED by demo_hub" and the encoder name `demo-simulated` so it can never be mistaken for
//! a real recording.

use capture_core::*;
use companion_hub::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Entry {
    meta: MediaMetadata,
    file: PathBuf,
    thumb: Option<PathBuf>,
}

struct Sim {
    session: Option<(String, RecordingState, Instant, u64)>, // id, state, started/resumed at, accumulated ms
    counter: u32,
}

struct DemoBackend {
    info: HubInfo,
    entries: Vec<Entry>,
    sim: Mutex<Sim>,
}

fn probe(path: &Path) -> Option<serde_json::Value> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn scan(dir: &Path, thumbs: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return entries };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.sort();
    for (i, file) in files.into_iter().enumerate() {
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let (kind, container, image_format) = match ext.as_str() {
            "mp4" => (MediaKind::Video, "mp4", None),
            "png" => (MediaKind::Screenshot, "png", Some(ImageFormat::Png)),
            "jpg" | "jpeg" => (MediaKind::Screenshot, "jpeg", Some(ImageFormat::Jpeg)),
            _ => continue,
        };
        let Some(p) = probe(&file) else {
            eprintln!("skipping {file:?}: ffprobe failed (not a valid media file?)");
            continue;
        };
        let streams = p["streams"].as_array().cloned().unwrap_or_default();
        let Some(v) = streams.iter().find(|s| s["codec_type"] == "video") else { continue };
        let width = v["width"].as_u64().unwrap_or(0) as u32;
        let height = v["height"].as_u64().unwrap_or(0) as u32;
        if width == 0 || height == 0 {
            continue;
        }
        let duration_ms = p["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()).map(|d| d * 1000.0);
        let fps = v["avg_frame_rate"].as_str().and_then(|r| {
            let (n, d) = r.split_once('/')?;
            let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
            (d > 0.0 && n > 0.0).then(|| n / d)
        });
        let video_codec = match v["codec_name"].as_str() {
            Some("h264") => Some(VideoCodec::H264),
            Some("hevc") => Some(VideoCodec::H265),
            _ => None,
        };
        let audio_tracks = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .enumerate()
            .map(|(idx, s)| AudioTrackInfo {
                index: idx as u32,
                codec: s["codec_name"].as_str().unwrap_or("unknown").to_string(),
                channels: s["channels"].as_u64().unwrap_or(2) as u32,
                sample_rate: s["sample_rate"].as_str().and_then(|r| r.parse().ok()).unwrap_or(48000),
                label: None,
                duration_ms: None,
            })
            .collect();
        let id = format!("demo-{i}");
        let thumb = thumbs.join(format!("{id}.jpg"));
        // Seek half a second into videos; stills have a single frame. ffmpeg can exit 0 without
        // writing a frame, so check the output file as well.
        let thumb_ok = |seek: Option<&str>| {
            let _ = std::fs::remove_file(&thumb);
            let mut cmd = Command::new("ffmpeg");
            cmd.args(["-v", "error", "-y"]);
            if let Some(ss) = seek {
                cmd.args(["-ss", ss]);
            }
            let ok = cmd.arg("-i").arg(&file).args(["-frames:v", "1", "-vf", "scale=480:-2"]).arg(&thumb).status().map(|s| s.success()).unwrap_or(false);
            ok && std::fs::metadata(&thumb).map(|m| m.len() > 0).unwrap_or(false)
        };
        let made = if kind == MediaKind::Video { thumb_ok(Some("0.5")) || thumb_ok(None) } else { thumb_ok(None) };
        let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        let file_name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = MediaMetadata {
            id: id.clone(),
            kind,
            path: file.display().to_string(),
            title: file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            file_name,
            created_at: now_iso8601(),
            size_bytes: size,
            width,
            height,
            duration_ms: if kind == MediaKind::Video { duration_ms } else { None },
            fps: if kind == MediaKind::Video { fps } else { None },
            video_codec: if kind == MediaKind::Video { video_codec } else { None },
            video_bitrate_kbps: None,
            audio_tracks,
            image_format,
            container: container.into(),
            source_name: Some("demo_hub media directory".into()),
            platform: Some("linux".into()),
            validation: Some(MediaValidation { valid: true, checked_at: now_iso8601(), tool: "ffprobe".into(), issues: vec![] }),
            thumbnail_path: made.then(|| thumb.display().to_string()),
        };
        entries.push(Entry { meta, file, thumb: made.then_some(thumb) });
    }
    entries
}

impl DemoBackend {
    fn status_locked(sim: &Sim) -> Option<RecordingStatus> {
        let (id, state, since, acc) = sim.session.as_ref()?;
        let elapsed = if *state == RecordingState::Recording { acc + since.elapsed().as_millis() as u64 } else { *acc };
        Some(RecordingStatus {
            session_id: id.clone(),
            state: *state,
            elapsed_ms: elapsed,
            countdown_remaining_sec: None,
            bytes_written: 0,
            frames_encoded: 0,
            frames_dropped: 0,
            estimated_bytes_per_minute: None,
            free_disk_bytes: None,
            output_path: None,
            encoder: Some("demo-simulated".into()),
            warnings: vec!["SIMULATED by demo_hub: nothing is being recorded".into()],
            error: None,
        })
    }
}

impl HubBackend for DemoBackend {
    fn hub_info(&self) -> HubInfo {
        self.info.clone()
    }
    fn list_recordings(&self) -> Vec<MediaMetadata> {
        self.entries.iter().map(|e| e.meta.clone()).collect()
    }
    fn recording_path(&self, id: &str) -> Option<PathBuf> {
        self.entries.iter().find(|e| e.meta.id == id).map(|e| e.file.clone())
    }
    fn thumbnail_path(&self, id: &str) -> Option<PathBuf> {
        self.entries.iter().find(|e| e.meta.id == id).and_then(|e| e.thumb.clone())
    }
    fn status(&self) -> Option<RecordingStatus> {
        self.sim.lock().ok().and_then(|s| Self::status_locked(&s))
    }
    fn command(&self, cmd: RemoteCommand) -> RemoteCommandResult {
        let Ok(mut sim) = self.sim.lock() else { return RemoteCommandResult::err("state poisoned") };
        let now = Instant::now();
        match (cmd, sim.session.clone()) {
            (RemoteCommand::StartRecording { .. }, None) => {
                sim.counter += 1;
                let id = format!("demo-session-{}", sim.counter);
                sim.session = Some((id.clone(), RecordingState::Recording, now, 0));
                RemoteCommandResult { ok: true, session_id: Some(id), ..Default::default() }
            }
            (RemoteCommand::PauseRecording, Some((id, RecordingState::Recording, since, acc))) => {
                sim.session = Some((id.clone(), RecordingState::Paused, now, acc + since.elapsed().as_millis() as u64));
                RemoteCommandResult { ok: true, session_id: Some(id), ..Default::default() }
            }
            (RemoteCommand::ResumeRecording, Some((id, RecordingState::Paused, _, acc))) => {
                sim.session = Some((id.clone(), RecordingState::Recording, now, acc));
                RemoteCommandResult { ok: true, session_id: Some(id), ..Default::default() }
            }
            (RemoteCommand::StopRecording, Some((id, _, _, _))) => {
                sim.session = None;
                RemoteCommandResult { ok: false, session_id: Some(id), media: None, error: Some("demo_hub simulates control only; no file was recorded".into()) }
            }
            (RemoteCommand::Screenshot, _) => RemoteCommandResult::err("demo_hub cannot capture screenshots (UNSUPPORTED in this fixture)"),
            (c, _) => RemoteCommandResult::err(format!("INVALID_STATE: {c:?} not valid now")),
        }
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let media_dir = PathBuf::from(arg(&args, "--media-dir").unwrap_or_else(|| "artifacts/samples".into()));
    let port: u16 = arg(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(COMPANION_DEFAULT_PORT);
    let bind = arg(&args, "--bind").and_then(|b| b.parse().ok()).unwrap_or(std::net::IpAddr::from([0, 0, 0, 0]));
    let state_dir = PathBuf::from(arg(&args, "--state-dir").unwrap_or_else(|| std::env::temp_dir().join("captureone-demo-hub").display().to_string()));
    let thumbs = state_dir.join("thumbs");
    if let Err(e) = std::fs::create_dir_all(&thumbs) {
        eprintln!("cannot create {thumbs:?}: {e}");
        std::process::exit(1);
    }
    let entries = scan(&media_dir, &thumbs);
    let mut features = BTreeMap::new();
    features.insert(FeatureKey::RemoteControl, FeatureSupport::with(SupportLevel::Partial, "demo_hub simulates recording control"));
    features.insert(FeatureKey::RecordingHistory, FeatureSupport::native());
    features.insert(FeatureKey::FileTransfer, FeatureSupport::native());
    features.insert(FeatureKey::VideoRecording, FeatureSupport::unsupported("demo_hub is a test fixture and does not record"));
    let caps = CapabilityReport {
        schema_version: 1,
        platform: Platform::current(),
        os_name: std::env::consts::OS.into(),
        os_version: "n/a".into(),
        device_model: None,
        app_version: APP_VERSION.into(),
        generated_at: now_iso8601(),
        features,
        encoders: vec![],
        frame_rates: vec![30],
        screenshot_formats: vec![],
        max_recording_resolution: None,
        notes: vec!["demo_hub test fixture".into()],
    };
    let backend = Arc::new(DemoBackend {
        info: HubInfo::new("demo-hub-0001", "CaptureOne demo hub", caps),
        entries,
        sim: Mutex::new(Sim { session: None, counter: 0 }),
    });
    let mdns = !args.iter().any(|a| a == "--no-mdns");
    let hub = match HubHandle::start_with_options(backend.clone(), HubConfig { bind, port, state_dir }, HubOptions { mdns, ..Default::default() }) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("hub failed to start: {e}");
            std::process::exit(1);
        }
    };
    println!("HUB_URL={}", hub.url());
    println!("HUB_PORT={}", hub.local_addr().port());
    println!("PAIRING_CODE={}", hub.pairing_code());
    println!("RECORDINGS={}", backend.entries.len());
    let mut last: Option<RecordingStatus> = None;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let st = backend.status();
        if st.is_some() || last.is_some() {
            hub.broadcast(HubEvent::RecordingStatus { status: st.clone() });
        }
        last = st;
    }
}
