//! Encoding, muxing, A/V sync, validation and recording orchestration for CaptureOne desktop.
//!
//! Capture backends (`CaptureBackend`) deliver BGRA frames and f32 PCM; this crate turns them
//! into validated MP4/PNG/JPEG files using an external ffmpeg/ffprobe:
//!
//! * video: backend -> latest-frame slot -> constant-rate pacer on the *active clock*
//!   (paused time excluded) -> bounded queue -> raw BGRA on ffmpeg stdin -> fragmented MP4;
//! * audio: microphone (cpal) / system audio (backend) -> audio timeline on the same clock
//!   (silence padding, pause drop, drift correction) -> resampler -> AAC fragmented M4A;
//! * stop: mux with stream copy (or `amix` for mixed tracks), atomic no-overwrite rename,
//!   ffprobe validation, thumbnail; the session directory is removed only when valid;
//! * crash safety: `<outputDir>/.captureone-sessions/<id>/` holds a manifest plus the
//!   fragmented intermediates; `recover_sessions` turns them into playable MP4s.

pub mod args;
pub mod audio;
pub mod clock;
pub mod device;
pub mod encoders;
mod finalize;
pub mod manifest;
pub mod naming;
pub mod pacer;
pub mod probe;
mod recover;
mod screenshot;
mod session;
pub mod tools;

pub use session::RecordingSession;

use capture_core::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    /// None = locate (env CAPTUREONE_FFMPEG, sidecar next to exe, PATH)
    pub ffmpeg_path: Option<PathBuf>,
    /// None = locate (env CAPTUREONE_FFPROBE, sidecar, PATH)
    pub ffprobe_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegInfo {
    pub ffmpeg_path: Option<String>,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    /// "input" | "loopback"
    pub kind: String,
    pub is_default: bool,
}

/// Receives session events. Callbacks run on engine threads: keep them short and never
/// call `RecordingSession::stop`/`cancel` from inside a callback.
pub trait RecorderListener: Send + Sync + 'static {
    fn on_state(&self, session_id: &str, from: RecordingState, to: RecordingState);
    /// ~2x per second while active
    fn on_status(&self, status: &RecordingStatus);
    fn on_warning(&self, session_id: &str, message: &str);
    fn on_error(&self, session_id: &str, error: &CaptureError);
    /// Exactly once per session, after finalization (user stop, limits, source closed, error).
    /// Not called when `start_recording` itself returns an error (the listener then sees
    /// `Starting -> Failed` and `on_error`).
    fn on_completed(&self, session_id: &str, reason: StopReason, result: &CaptureResult<MediaMetadata>);
}

pub(crate) struct EngineInner {
    pub(crate) backend: Arc<dyn CaptureBackend>,
    pub(crate) ffmpeg: Option<PathBuf>,
    pub(crate) ffprobe: Option<PathBuf>,
    ffmpeg_version: Option<String>,
    encoders: OnceLock<Vec<EncoderInfo>>,
    device_demuxer: OnceLock<bool>,
}

impl EngineInner {
    /// Encoders known to ffmpeg, each verified by a real test encode (probed once, cached).
    pub(crate) fn encoders(&self) -> Vec<EncoderInfo> {
        self.encoders.get_or_init(|| self.ffmpeg.as_deref().map(encoders::probe_encoders).unwrap_or_default()).clone()
    }
}

#[derive(Clone)]
pub struct MediaEngine {
    inner: Arc<EngineInner>,
}

impl MediaEngine {
    pub fn new(backend: Arc<dyn CaptureBackend>, cfg: EngineConfig) -> CaptureResult<Self> {
        let ffmpeg = tools::locate_tool(cfg.ffmpeg_path.as_deref(), "CAPTUREONE_FFMPEG", "ffmpeg");
        let ffprobe = tools::locate_tool(cfg.ffprobe_path.as_deref(), "CAPTUREONE_FFPROBE", "ffprobe");
        if cfg.ffmpeg_path.is_some() && ffmpeg.is_none() {
            return Err(CaptureError::invalid_config(format!("ffmpeg not found at {}", cfg.ffmpeg_path.as_deref().map(Path::display).map(|d| d.to_string()).unwrap_or_default())));
        }
        if cfg.ffprobe_path.is_some() && ffprobe.is_none() {
            return Err(CaptureError::invalid_config(format!("ffprobe not found at {}", cfg.ffprobe_path.as_deref().map(Path::display).map(|d| d.to_string()).unwrap_or_default())));
        }
        let ffmpeg_version = ffmpeg.as_deref().and_then(tools::tool_version);
        // An ffmpeg that cannot even print its version is unusable.
        let ffmpeg = if ffmpeg_version.is_some() { ffmpeg } else { None };
        Ok(Self { inner: Arc::new(EngineInner { backend, ffmpeg, ffprobe, ffmpeg_version, encoders: OnceLock::new(), device_demuxer: OnceLock::new() }) })
    }

    pub fn backend(&self) -> &Arc<dyn CaptureBackend> {
        &self.inner.backend
    }

    /// Backend capture capabilities merged with encoders verified by real test encodes.
    pub fn capabilities(&self) -> CapabilityReport {
        let mut r = self.inner.backend.capabilities();
        let encs = self.inner.encoders();
        let has = |codec: VideoCodec| encs.iter().any(|e| e.verified && e.codec == codec);
        let hw: Vec<&str> = encs.iter().filter(|e| e.verified && e.hardware).map(|e| e.name.as_str()).collect();
        let no_ffmpeg = self.inner.ffmpeg.is_none() || self.inner.ffprobe.is_none();
        let reason_none = if no_ffmpeg { "ffmpeg/ffprobe not found".to_string() } else { "no encoder passed a test encode".to_string() };
        for (key, codec) in [(FeatureKey::H264, VideoCodec::H264), (FeatureKey::H265, VideoCodec::H265)] {
            let names: Vec<&str> = encs.iter().filter(|e| e.verified && e.codec == codec).map(|e| e.name.as_str()).collect();
            let f = if has(codec) && !no_ffmpeg { FeatureSupport::with(SupportLevel::Native, format!("verified encoders: {}", names.join(", "))) } else { FeatureSupport::unsupported(reason_none.clone()) };
            r.features.insert(key, f);
        }
        r.features.insert(
            FeatureKey::HardwareEncoding,
            if hw.is_empty() || no_ffmpeg {
                FeatureSupport::unsupported("no hardware encoder passed a test encode; software encoding is used")
            } else {
                FeatureSupport::with(SupportLevel::Native, format!("verified: {}", hw.join(", ")))
            },
        );
        let can_capture = r.is_usable(FeatureKey::DisplayCapture) || r.is_usable(FeatureKey::WindowCapture);
        let can_record = can_capture && !no_ffmpeg && (has(VideoCodec::H264) || has(VideoCodec::H265));
        let recording = if can_record {
            r.features.get(&FeatureKey::VideoRecording).filter(|f| f.level != SupportLevel::Unsupported).cloned().unwrap_or_else(FeatureSupport::native)
        } else if !can_capture {
            FeatureSupport::unsupported("screen capture is not available")
        } else {
            FeatureSupport::unsupported(reason_none.clone())
        };
        r.features.insert(FeatureKey::VideoRecording, recording);
        r.features.insert(FeatureKey::PauseResume, if can_record { FeatureSupport::native() } else { FeatureSupport::unsupported("recording is unavailable") });
        let mic = if !can_record {
            FeatureSupport::unsupported("recording is unavailable")
        } else if audio::has_input_device() {
            let mut f = r.features.get(&FeatureKey::MicrophoneAudio).filter(|f| f.level != SupportLevel::Unsupported).cloned().unwrap_or_else(FeatureSupport::native);
            if cfg!(target_os = "macos") && f.requires.is_none() {
                f = f.requires(vec![PermissionKind::Microphone]);
            }
            f
        } else {
            FeatureSupport::unsupported("no audio input device found")
        };
        let any_audio = mic.level != SupportLevel::Unsupported || r.is_usable(FeatureKey::SystemAudio);
        r.features.insert(FeatureKey::MicrophoneAudio, mic);
        r.features.insert(
            FeatureKey::SeparateAudioTracks,
            if can_record && any_audio { FeatureSupport::native() } else { FeatureSupport::unsupported("no audio source available") },
        );
        if !can_record && r.is_usable(FeatureKey::SystemAudio) {
            r.features.insert(FeatureKey::SystemAudio, FeatureSupport::unsupported("recording is unavailable"));
        }
        let device = match self.inner.ffmpeg.as_deref() {
            Some(ff) if *self.inner.device_demuxer.get_or_init(|| device::demuxer_available(ff)) && can_record => FeatureSupport::with(
                SupportLevel::Partial,
                "requires a UVC/HDMI capture card; HDCP-protected sources are blank. Select the card's audio input as the microphone device for its audio.",
            ),
            Some(_) if can_record => FeatureSupport::unsupported("this ffmpeg build has no OS video-device demuxer (v4l2/dshow/avfoundation)"),
            _ => FeatureSupport::unsupported("recording is unavailable (ffmpeg/ffprobe or encoder missing)"),
        };
        r.features.insert(FeatureKey::HdmiCaptureDevice, device);
        r.encoders = encs;
        match (&self.inner.ffmpeg, &self.inner.ffmpeg_version) {
            (Some(p), Some(v)) => r.notes.push(format!("Encoders verified with real test encodes using ffmpeg {v} at {}", p.display())),
            _ => r.notes.push("ffmpeg was not found: recording is unavailable (screenshots still work)".into()),
        }
        r
    }

    pub fn list_audio_devices(&self) -> CaptureResult<Vec<AudioDeviceInfo>> {
        let mut devices = audio::list_input_devices().unwrap_or_default();
        if self.inner.backend.capabilities().is_usable(FeatureKey::SystemAudio) {
            devices.push(AudioDeviceInfo { id: "system-default".into(), name: "System audio (default output)".into(), kind: "loopback".into(), is_default: true });
        }
        Ok(devices)
    }

    /// Video capture devices (HDMI capture cards, UVC webcams) found on this machine.
    pub fn list_capture_devices(&self) -> Vec<CaptureSource> {
        device::list_devices(self.inner.ffmpeg.as_deref())
    }

    /// Backend sources plus capture devices when `SourceKind::Device` is requested.
    pub fn list_sources(&self, kinds: &[SourceKind]) -> CaptureResult<Vec<CaptureSource>> {
        let mut out = self.inner.backend.list_sources(kinds)?;
        out.retain(|s| s.kind != SourceKind::Device);
        if kinds.contains(&SourceKind::Device) {
            out.extend(self.list_capture_devices());
        }
        Ok(out)
    }

    /// Captures, encodes (PNG/JPEG), writes into `output_dir`, validates by decoding, returns metadata.
    /// `req.delay_sec` is not applied here (the caller handles delays/countdowns).
    pub fn screenshot(&self, req: &ScreenshotRequest, output_dir: &Path) -> CaptureResult<MediaMetadata> {
        screenshot::screenshot(self.inner.backend.as_ref(), self.inner.ffmpeg.as_deref(), req, output_dir)
    }

    /// PNG data URL of a source scaled to max_width (for source pickers).
    pub fn source_thumbnail(&self, kind: SourceKind, source_id: &str, max_width: u32) -> CaptureResult<String> {
        screenshot::source_thumbnail(self.inner.backend.as_ref(), self.inner.ffmpeg.as_deref(), kind, source_id, max_width)
    }

    /// Starts recording immediately (the caller handles any countdown).
    pub fn start_recording(&self, config: RecordingConfig, listener: Arc<dyn RecorderListener>) -> CaptureResult<RecordingSession> {
        session::start(&self.inner, config, listener)
    }

    /// ffprobe-based metadata + validation for an existing media file (PNG/JPEG are
    /// validated by decoding).
    pub fn inspect(&self, path: &Path) -> CaptureResult<MediaMetadata> {
        if !path.is_file() {
            return Err(CaptureError::new(CaptureErrorCode::SourceNotFound, format!("{} does not exist", path.display())));
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
            return screenshot::inspect_image(path);
        }
        let ffprobe = self.inner.ffprobe.as_deref().ok_or_else(|| CaptureError::new(CaptureErrorCode::EncoderUnavailable, "ffprobe not found"))?;
        let probe = probe::run_ffprobe(ffprobe, path)?;
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let validation = probe::validate(&probe, size, &probe::Expectation::default());
        Ok(finalize::video_metadata(finalize::VideoMetaInput {
            id: &uuid::Uuid::new_v4().to_string(),
            path,
            size_bytes: size,
            probe: &probe,
            validation: Some(validation),
            created_at: finalize::file_time_iso(path),
            source_name: None,
            platform: None,
            thumbnail: None,
        }))
    }

    /// Finds interrupted sessions under `output_dir` and finalizes them into playable MP4s.
    pub fn recover_sessions(&self, output_dir: &Path) -> Vec<CaptureResult<MediaMetadata>> {
        recover::recover_sessions(&self.inner, output_dir)
    }

    pub fn ffmpeg_info(&self) -> FfmpegInfo {
        FfmpegInfo {
            ffmpeg_path: self.inner.ffmpeg.as_ref().map(|p| p.to_string_lossy().to_string()),
            ffmpeg_version: self.inner.ffmpeg_version.clone(),
            ffprobe_path: self.inner.ffprobe.as_ref().map(|p| p.to_string_lossy().to_string()),
        }
    }
}
