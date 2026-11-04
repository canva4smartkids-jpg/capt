//! `session.json`: the per-session manifest that makes interrupted recordings recoverable.

use crate::audio::AudioSourceKind;
use capture_core::{CaptureResult, RecordingConfig, TrackMode, VideoCodec};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SESSIONS_DIR: &str = ".captureone-sessions";
pub const THUMBNAILS_DIR: &str = ".captureone-thumbnails";
pub const MANIFEST_FILE: &str = "session.json";
pub const LOCK_FILE: &str = "session.lock";
pub const VIDEO_FILE: &str = "video.mp4";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestState {
    Starting,
    Recording,
    Finalizing,
    Completed,
    Failed,
    FailedValidation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestVideo {
    pub file: String,
    pub input_width: u32,
    pub input_height: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: VideoCodec,
    pub encoder: String,
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestAudio {
    pub file: String,
    pub source: AudioSourceKind,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionManifest {
    pub version: u32,
    pub session_id: String,
    pub state: ManifestState,
    pub app_version: String,
    pub platform: String,
    pub created_at: String,
    /// RFC 3339 local time used for the `{date}`/`{time}` template tokens.
    pub created_local: String,
    pub updated_at: String,
    pub source_name: String,
    pub file_name_template: String,
    pub config: RecordingConfig,
    pub video: Option<ManifestVideo>,
    pub audio: Vec<ManifestAudio>,
    pub track_mode: TrackMode,
    pub audio_bitrate_kbps: u32,
    /// Active (non-paused) milliseconds recorded so far (updated every ~2 s).
    pub active_ms: u64,
    pub final_path: Option<String>,
    pub error: Option<String>,
}

pub fn session_dir(output_dir: &Path, id: &str) -> PathBuf {
    output_dir.join(SESSIONS_DIR).join(id)
}

/// Writes the manifest atomically (temp file + rename).
pub fn write_manifest(dir: &Path, m: &SessionManifest) -> CaptureResult<()> {
    let mut m = m.clone();
    m.updated_at = capture_core::now_iso8601();
    let bytes = serde_json::to_vec_pretty(&m).map_err(|e| capture_core::CaptureError::internal(e.to_string()))?;
    let tmp = dir.join(format!("{MANIFEST_FILE}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(MANIFEST_FILE))?;
    Ok(())
}

pub fn read_manifest(dir: &Path) -> Option<SessionManifest> {
    serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE)).ok()?).ok()
}
