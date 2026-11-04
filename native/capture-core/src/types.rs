//! Serde mirrors of `packages/capture-contracts` (TypeScript/zod is the source of truth).
//! Field names serialize as camelCase; enum spellings match the TS literals exactly.
//! `tests/contract.rs` validates serialized values against the exported JSON Schemas.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
    Android,
    AndroidTv,
    GoogleTv,
    Tizen,
    Webos,
    OtherTv,
}

impl Platform {
    /// Platform this binary was compiled for.
    pub fn current() -> Platform {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(target_os = "android") {
            Platform::Android
        } else {
            Platform::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupportLevel {
    Native,
    Partial,
    CompanionOnly,
    ExternalHardware,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FeatureKey {
    Screenshot,
    DisplayCapture,
    WindowCapture,
    RegionCapture,
    MultiDisplay,
    VideoRecording,
    PauseResume,
    CursorCapture,
    MicrophoneAudio,
    SystemAudio,
    SeparateAudioTracks,
    H264,
    H265,
    HardwareEncoding,
    GlobalShortcuts,
    RecordingHistory,
    Share,
    HdmiCaptureDevice,
    RemoteControl,
    DeviceDiscovery,
    TvNativeScreenshot,
    FileTransfer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureSupport {
    pub level: SupportLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<Vec<PermissionKind>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_os_version: Option<String>,
}

impl FeatureSupport {
    pub fn native() -> Self {
        Self { level: SupportLevel::Native, reason: None, requires: None, min_os_version: None }
    }
    pub fn with(level: SupportLevel, reason: impl Into<String>) -> Self {
        Self { level, reason: Some(reason.into()), requires: None, min_os_version: None }
    }
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::with(SupportLevel::Unsupported, reason)
    }
    pub fn requires(mut self, perms: Vec<PermissionKind>) -> Self {
        self.requires = Some(perms);
        self
    }
    pub fn min_os(mut self, v: impl Into<String>) -> Self {
        self.min_os_version = Some(v.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    H265,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    pub codec: VideoCodec,
    pub name: String,
    pub hardware: bool,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityReport {
    pub schema_version: u8,
    pub platform: Platform,
    pub os_name: String,
    pub os_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    pub app_version: String,
    pub generated_at: String,
    pub features: BTreeMap<FeatureKey, FeatureSupport>,
    pub encoders: Vec<EncoderInfo>,
    pub frame_rates: Vec<u32>,
    pub screenshot_formats: Vec<ImageFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_recording_resolution: Option<Size>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl CapabilityReport {
    pub fn level(&self, key: FeatureKey) -> SupportLevel {
        self.features.get(&key).map(|f| f.level).unwrap_or(SupportLevel::Unsupported)
    }
    pub fn is_usable(&self, key: FeatureKey) -> bool {
        matches!(self.level(key), SupportLevel::Native | SupportLevel::Partial)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionKind {
    ScreenCapture,
    Microphone,
    SystemAudio,
    Storage,
    Notifications,
    LocalNetwork,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionState {
    Granted,
    Denied,
    Prompt,
    Restricted,
    NotApplicable,
    PerSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub kind: PermissionKind,
    pub state: PermissionState,
    pub requires_system_settings: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PermissionStatus {
    pub fn new(kind: PermissionKind, state: PermissionState) -> Self {
        Self { kind, state, requires_system_settings: false, message: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Display,
    Window,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSource {
    pub id: String,
    pub kind: SourceKind,
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_primary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_descriptor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum CaptureTarget {
    Display { display_id: String },
    Window { window_id: String },
    Region { display_id: String, rect: Rect },
    Device { device_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaptureErrorCode {
    PermissionDenied,
    Unsupported,
    SourceNotFound,
    SourceClosed,
    EncoderUnavailable,
    EncoderFailed,
    StorageFull,
    IoError,
    InvalidConfig,
    InvalidState,
    DrmProtected,
    Interrupted,
    DeviceBusy,
    Network,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct CaptureError {
    pub code: CaptureErrorCode,
    pub message: String,
    pub recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Map<String, serde_json::Value>>,
}

impl CaptureError {
    pub fn new(code: CaptureErrorCode, message: impl Into<String>) -> Self {
        let recoverable = matches!(code, CaptureErrorCode::DeviceBusy | CaptureErrorCode::Network | CaptureErrorCode::SourceNotFound);
        Self { code, message: message.into(), recoverable, details: None }
    }
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(CaptureErrorCode::Unsupported, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(CaptureErrorCode::Internal, message)
    }
    pub fn io(err: impl std::fmt::Display) -> Self {
        Self::new(CaptureErrorCode::IoError, err.to_string())
    }
    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(CaptureErrorCode::InvalidState, message)
    }
    pub fn invalid_config(message: impl Into<String>) -> Self {
        Self::new(CaptureErrorCode::InvalidConfig, message)
    }
    pub fn with_detail(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.details.get_or_insert_with(Default::default).insert(key.to_string(), value.into());
        self
    }
}

impl From<std::io::Error> for CaptureError {
    fn from(e: std::io::Error) -> Self {
        // ENOSPC (28 on Linux/macOS) / ERROR_DISK_FULL (112) / ERROR_HANDLE_DISK_FULL (39) on Windows.
        let full = matches!(e.raw_os_error(), Some(28) | Some(112) | Some(39)) || e.kind() == std::io::ErrorKind::StorageFull;
        if full {
            CaptureError::new(CaptureErrorCode::StorageFull, e.to_string())
        } else {
            CaptureError::io(e)
        }
    }
}

pub type CaptureResult<T> = Result<T, CaptureError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum ResolutionSetting {
    Source,
    Height { height: u32 },
    Custom { width: u32, height: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityPreset {
    Low,
    Medium,
    High,
    Ultra,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HardwareAcceleration {
    #[default]
    Auto,
    Prefer,
    Off,
}

fn default_keyframe() -> f64 {
    2.0
}
fn default_true() -> bool {
    true
}
fn default_gain() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSettings {
    pub codec: VideoCodec,
    pub resolution: ResolutionSetting,
    pub fps: u32,
    pub quality: QualityPreset,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate_kbps: Option<u32>,
    #[serde(default = "default_keyframe")]
    pub keyframe_interval_sec: f64,
    #[serde(default)]
    pub hardware_acceleration: HardwareAcceleration,
    #[serde(default = "default_true")]
    pub show_cursor: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSourceSettings {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default = "default_gain")]
    pub gain: f64,
}

impl AudioSourceSettings {
    pub fn disabled() -> Self {
        Self { enabled: false, device_id: None, gain: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TrackMode {
    #[default]
    Mixed,
    Separate,
}

fn default_sample_rate() -> u32 {
    48_000
}
fn default_audio_kbps() -> u32 {
    160
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    pub microphone: AudioSourceSettings,
    pub system: AudioSourceSettings,
    #[serde(default)]
    pub track_mode: TrackMode,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "default_audio_kbps")]
    pub bitrate_kbps: u32,
}

fn default_template() -> String {
    "CaptureOne {date} {time}".to_string()
}
fn default_container() -> String {
    "mp4".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSettings {
    pub directory: String,
    #[serde(default = "default_template")]
    pub file_name_template: String,
    #[serde(default = "default_container")]
    pub container: String,
}

fn default_min_free() -> u64 {
    500
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_size_mb: Option<u64>,
    #[serde(default = "default_min_free")]
    pub min_free_disk_mb: u64,
}

impl Default for RecordingLimits {
    fn default() -> Self {
        Self { max_duration_sec: None, max_file_size_mb: None, min_free_disk_mb: default_min_free() }
    }
}

fn default_countdown() -> u32 {
    3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingConfig {
    pub target: CaptureTarget,
    pub video: VideoSettings,
    pub audio: AudioSettings,
    pub output: OutputSettings,
    #[serde(default)]
    pub limits: RecordingLimits,
    #[serde(default = "default_countdown")]
    pub countdown_sec: u32,
}

impl RecordingConfig {
    /// Same rules as the zod schema (which remains authoritative for the UI).
    pub fn validate(&self) -> CaptureResult<()> {
        let bad = |m: &str| Err(CaptureError::invalid_config(m));
        if ![24, 30, 60].contains(&self.video.fps) {
            return bad("fps must be 24, 30 or 60");
        }
        if self.video.quality == QualityPreset::Custom && self.video.bitrate_kbps.is_none() {
            return bad("bitrateKbps is required when quality is 'custom'");
        }
        if let Some(b) = self.video.bitrate_kbps {
            if !(250..=200_000).contains(&b) {
                return bad("bitrateKbps must be within 250..200000");
            }
        }
        if !(0.5..=10.0).contains(&self.video.keyframe_interval_sec) {
            return bad("keyframeIntervalSec must be within 0.5..10");
        }
        match self.video.resolution {
            ResolutionSetting::Height { height } if ![2160, 1440, 1080, 720, 480].contains(&height) => {
                return bad("resolution height must be one of 2160/1440/1080/720/480")
            }
            ResolutionSetting::Custom { width, height } if !(16..=7680).contains(&width) || !(16..=4320).contains(&height) => {
                return bad("custom resolution out of range")
            }
            _ => {}
        }
        for s in [&self.audio.microphone, &self.audio.system] {
            if !(0.0..=2.0).contains(&s.gain) {
                return bad("audio gain must be within 0..2");
            }
        }
        if ![44_100, 48_000].contains(&self.audio.sample_rate) {
            return bad("sampleRate must be 44100 or 48000");
        }
        if ![96, 128, 160, 192, 256].contains(&self.audio.bitrate_kbps) {
            return bad("audio bitrateKbps must be one of 96/128/160/192/256");
        }
        if ![0, 3, 5, 10].contains(&self.countdown_sec) {
            return bad("countdownSec must be 0, 3, 5 or 10");
        }
        if self.output.directory.trim().is_empty() {
            return bad("output directory is required");
        }
        if self.output.container != "mp4" {
            return bad("only mp4 container is supported");
        }
        if let CaptureTarget::Region { rect, .. } = &self.target {
            if rect.width < 16 || rect.height < 16 {
                return bad("region must be at least 16x16 pixels");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingState {
    Idle,
    Countdown,
    Starting,
    Recording,
    Paused,
    Stopping,
    Finalizing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    pub session_id: String,
    pub state: RecordingState,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub countdown_remaining_sec: Option<u32>,
    pub bytes_written: u64,
    pub frames_encoded: u64,
    pub frames_dropped: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_bytes_per_minute: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_disk_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoder: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CaptureError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    User,
    MaxDuration,
    MaxFileSize,
    LowDisk,
    SourceClosed,
    Error,
    Interrupted,
    /// `RecordingSession::cancel`: the recording was discarded.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackInfo {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaValidation {
    pub valid: bool,
    pub checked_at: String,
    pub tool: String,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Screenshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    pub id: String,
    pub kind: MediaKind,
    pub path: String,
    pub file_name: String,
    pub title: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<VideoCodec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_bitrate_kbps: Option<f64>,
    #[serde(default)]
    pub audio_tracks: Vec<AudioTrackInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_format: Option<ImageFormat>,
    /// "mp4" | "png" | "jpeg"
    pub container: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<MediaValidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_path: Option<String>,
}

fn default_jpeg_quality() -> u8 {
    90
}
fn default_png() -> ImageFormat {
    ImageFormat::Png
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotRequest {
    pub target: CaptureTarget,
    #[serde(default = "default_png")]
    pub format: ImageFormat,
    #[serde(default = "default_jpeg_quality")]
    pub jpeg_quality: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name_template: Option<String>,
    #[serde(default = "default_true")]
    pub show_cursor: bool,
    #[serde(default)]
    pub delay_sec: u32,
}
