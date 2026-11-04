//! Encoding, muxing, A/V sync, validation and recording orchestration for CaptureOne desktop.
//!
//! NOTE: API surface only (engineering-rules §4.2). Bodies are being implemented.

use capture_core::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

pub trait RecorderListener: Send + Sync + 'static {
    fn on_state(&self, session_id: &str, from: RecordingState, to: RecordingState);
    /// ~2x per second while active
    fn on_status(&self, status: &RecordingStatus);
    fn on_warning(&self, session_id: &str, message: &str);
    fn on_error(&self, session_id: &str, error: &CaptureError);
    /// Exactly once per session, after finalization (user stop, limits, source closed, error).
    fn on_completed(&self, session_id: &str, reason: StopReason, result: &CaptureResult<MediaMetadata>);
}

struct EngineInner {
    backend: Arc<dyn CaptureBackend>,
    #[allow(dead_code)]
    cfg: EngineConfig,
}

#[derive(Clone)]
pub struct MediaEngine {
    inner: Arc<EngineInner>,
}

fn todo_err() -> CaptureError {
    CaptureError::internal("not implemented yet")
}

impl MediaEngine {
    pub fn new(backend: Arc<dyn CaptureBackend>, cfg: EngineConfig) -> CaptureResult<Self> {
        Ok(Self { inner: Arc::new(EngineInner { backend, cfg }) })
    }
    pub fn backend(&self) -> &Arc<dyn CaptureBackend> {
        &self.inner.backend
    }
    /// Backend capture capabilities merged with encoders verified by real test encodes.
    pub fn capabilities(&self) -> CapabilityReport {
        self.inner.backend.capabilities()
    }
    pub fn list_audio_devices(&self) -> CaptureResult<Vec<AudioDeviceInfo>> {
        Err(todo_err())
    }
    /// Captures, encodes (PNG/JPEG), writes into `output_dir`, validates by decoding, returns metadata.
    pub fn screenshot(&self, _req: &ScreenshotRequest, _output_dir: &Path) -> CaptureResult<MediaMetadata> {
        Err(todo_err())
    }
    /// PNG data URL of a source scaled to max_width (for source pickers).
    pub fn source_thumbnail(&self, _kind: SourceKind, _source_id: &str, _max_width: u32) -> CaptureResult<String> {
        Err(todo_err())
    }
    /// Starts recording immediately (the caller handles any countdown).
    pub fn start_recording(&self, _config: RecordingConfig, _listener: Arc<dyn RecorderListener>) -> CaptureResult<RecordingSession> {
        Err(todo_err())
    }
    /// ffprobe-based metadata + validation for an existing media file.
    pub fn inspect(&self, _path: &Path) -> CaptureResult<MediaMetadata> {
        Err(todo_err())
    }
    /// Finds interrupted sessions under `output_dir` and finalizes them into playable MP4s.
    pub fn recover_sessions(&self, _output_dir: &Path) -> Vec<CaptureResult<MediaMetadata>> {
        Vec::new()
    }
    pub fn ffmpeg_info(&self) -> FfmpegInfo {
        FfmpegInfo { ffmpeg_path: None, ffmpeg_version: None, ffprobe_path: None }
    }
}

/// Cheap clonable handle to a running recording.
#[derive(Clone)]
pub struct RecordingSession {
    id: Arc<str>,
}

impl RecordingSession {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn pause(&self) -> CaptureResult<()> {
        Err(todo_err())
    }
    pub fn resume(&self) -> CaptureResult<()> {
        Err(todo_err())
    }
    /// Blocks until the file is finalized and validated.
    pub fn stop(&self) -> CaptureResult<MediaMetadata> {
        Err(todo_err())
    }
    /// Stops and deletes all output.
    pub fn cancel(&self) -> CaptureResult<()> {
        Err(todo_err())
    }
    pub fn status(&self) -> RecordingStatus {
        RecordingStatus {
            session_id: self.id.to_string(),
            state: RecordingState::Idle,
            elapsed_ms: 0,
            countdown_remaining_sec: None,
            bytes_written: 0,
            frames_encoded: 0,
            frames_dropped: 0,
            estimated_bytes_per_minute: None,
            free_disk_bytes: None,
            output_path: None,
            encoder: None,
            warnings: Vec::new(),
            error: None,
        }
    }
}
