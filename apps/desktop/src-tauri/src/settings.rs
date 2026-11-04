//! Persisted AppSettings (mirror of `AppSettings` in packages/settings; zod is the
//! source of truth). Loading never fails: invalid fields fall back to defaults one
//! by one; saving is strict and rejects invalid input with INVALID_CONFIG.

use capture_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBindings {
    #[serde(default = "d_start_stop")]
    pub start_stop: String,
    #[serde(default = "d_pause_resume")]
    pub pause_resume: String,
    #[serde(default = "d_screenshot")]
    pub screenshot: String,
    #[serde(default = "d_region_screenshot")]
    pub region_screenshot: String,
}

fn d_start_stop() -> String {
    "CommandOrControl+Shift+R".into()
}
fn d_pause_resume() -> String {
    "CommandOrControl+Shift+P".into()
}
fn d_screenshot() -> String {
    "CommandOrControl+Shift+S".into()
}
fn d_region_screenshot() -> String {
    "CommandOrControl+Shift+A".into()
}

impl Default for ShortcutBindings {
    fn default() -> Self {
        Self { start_stop: d_start_stop(), pause_resume: d_pause_resume(), screenshot: d_screenshot(), region_screenshot: d_region_screenshot() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "d_schema")]
    pub schema_version: u8,
    #[serde(default)]
    pub output_directory: String,
    #[serde(default = "d_template")]
    pub file_name_template: String,
    #[serde(default = "d_video")]
    pub video: VideoSettings,
    #[serde(default = "d_audio")]
    pub audio: AudioSettings,
    #[serde(default = "d_png")]
    pub screenshot_format: ImageFormat,
    #[serde(default = "d_jpeg")]
    pub jpeg_quality: u8,
    #[serde(default = "d_countdown")]
    pub countdown_sec: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_sec: Option<u64>,
    #[serde(default = "d_min_free")]
    pub min_free_disk_mb: u64,
    #[serde(default)]
    pub shortcuts: ShortcutBindings,
    #[serde(default = "d_true")]
    pub show_recording_indicator: bool,
    #[serde(default)]
    pub minimize_on_record: bool,
    #[serde(default)]
    pub companion_hub_enabled: bool,
    #[serde(default)]
    pub theme: Theme,
}

fn d_schema() -> u8 {
    1
}
fn d_template() -> String {
    "CaptureOne {date} {time}".into()
}
fn d_png() -> ImageFormat {
    ImageFormat::Png
}
fn d_jpeg() -> u8 {
    90
}
fn d_countdown() -> u32 {
    3
}
fn d_min_free() -> u64 {
    500
}
fn d_true() -> bool {
    true
}
fn d_video() -> VideoSettings {
    VideoSettings {
        codec: VideoCodec::H264,
        resolution: ResolutionSetting::Source,
        fps: 30,
        quality: QualityPreset::Medium,
        bitrate_kbps: None,
        keyframe_interval_sec: 2.0,
        hardware_acceleration: HardwareAcceleration::Auto,
        show_cursor: true,
    }
}
fn d_audio() -> AudioSettings {
    AudioSettings { microphone: AudioSourceSettings::disabled(), system: AudioSourceSettings::disabled(), track_mode: TrackMode::Mixed, sample_rate: 48_000, bitrate_kbps: 160 }
}

impl Settings {
    pub fn defaults(output_directory: &str) -> Self {
        Self {
            schema_version: 1,
            output_directory: output_directory.to_string(),
            file_name_template: d_template(),
            video: d_video(),
            audio: d_audio(),
            screenshot_format: ImageFormat::Png,
            jpeg_quality: 90,
            countdown_sec: 3,
            max_duration_sec: None,
            min_free_disk_mb: 500,
            shortcuts: ShortcutBindings::default(),
            show_recording_indicator: true,
            minimize_on_record: false,
            companion_hub_enabled: false,
            theme: Theme::System,
        }
    }

    /// Same rules as the zod schema. The output directory may be empty here (filled in on load).
    pub fn validate(&self) -> CaptureResult<()> {
        if self.schema_version != 1 {
            return Err(CaptureError::invalid_config("unsupported settings schemaVersion"));
        }
        if !(1..=100).contains(&self.jpeg_quality) {
            return Err(CaptureError::invalid_config("jpegQuality must be within 1..100"));
        }
        if matches!(self.max_duration_sec, Some(0)) {
            return Err(CaptureError::invalid_config("maxDurationSec must be positive"));
        }
        if self.file_name_template.trim().is_empty() {
            return Err(CaptureError::invalid_config("fileNameTemplate must not be empty"));
        }
        // Reuse the recording-config validator for the video/audio/countdown rules.
        self.recording_config(CaptureTarget::Display { display_id: "validate".into() }, Some("validate")).validate()
    }

    /// A RecordingConfig from these defaults for `target`.
    pub fn recording_config(&self, target: CaptureTarget, output_dir_override: Option<&str>) -> RecordingConfig {
        RecordingConfig {
            target,
            video: self.video.clone(),
            audio: self.audio.clone(),
            output: OutputSettings {
                directory: output_dir_override.map(str::to_string).unwrap_or_else(|| self.output_directory.clone()),
                file_name_template: self.file_name_template.clone(),
                container: "mp4".into(),
            },
            limits: RecordingLimits { max_duration_sec: self.max_duration_sec, max_file_size_mb: None, min_free_disk_mb: self.min_free_disk_mb },
            countdown_sec: self.countdown_sec,
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Parses `raw` with per-field fallback: each top-level field that fails to parse or
/// validate is replaced by its default; unknown fields are dropped.
pub fn from_value_lenient(raw: &Value, default_output_dir: &str) -> (Settings, Vec<String>) {
    let defaults = Settings::defaults(default_output_dir);
    let mut dropped = Vec::new();
    let Some(obj) = raw.as_object() else {
        if !raw.is_null() {
            dropped.push("<root>".to_string());
        }
        return (defaults, dropped);
    };
    let mut merged: Map<String, Value> = match defaults.to_json() {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    for (key, value) in obj {
        if !merged.contains_key(key) && key != "maxDurationSec" {
            continue; // unknown key
        }
        let mut candidate = merged.clone();
        candidate.insert(key.clone(), value.clone());
        let ok = serde_json::from_value::<Settings>(Value::Object(candidate.clone())).ok().filter(|s| s.validate().is_ok()).is_some();
        if ok {
            merged = candidate;
        } else {
            dropped.push(key.clone());
        }
    }
    let mut settings = serde_json::from_value::<Settings>(Value::Object(merged)).unwrap_or(defaults);
    if settings.output_directory.trim().is_empty() {
        settings.output_directory = default_output_dir.to_string();
    }
    (settings, dropped)
}

/// Strict parse for `save_settings`.
pub fn from_value_strict(raw: &Value) -> CaptureResult<Settings> {
    let s: Settings = serde_json::from_value(raw.clone()).map_err(|e| CaptureError::invalid_config(format!("invalid settings: {e}")))?;
    s.validate()?;
    if s.output_directory.trim().is_empty() {
        return Err(CaptureError::invalid_config("outputDirectory is required"));
    }
    Ok(s)
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE)
}

/// Loads settings; missing or corrupt files yield defaults (never an error).
pub fn load(config_dir: &Path, default_output_dir: &str) -> Settings {
    let path = settings_path(config_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or_else(|e| {
            log::warn!("settings file {} is not valid JSON ({e}); using defaults", path.display());
            Value::Null
        }),
        Err(_) => Value::Null,
    };
    let (settings, dropped) = from_value_lenient(&raw, default_output_dir);
    if !dropped.is_empty() {
        log::warn!("settings: replaced invalid fields with defaults: {}", dropped.join(", "));
    }
    settings
}

/// Atomically writes settings (temp file + rename).
pub fn save(config_dir: &Path, settings: &Settings) -> CaptureResult<()> {
    std::fs::create_dir_all(config_dir)?;
    let path = settings_path(config_dir);
    let tmp = config_dir.join(format!("{SETTINGS_FILE}.tmp"));
    let text = serde_json::to_string_pretty(settings).map_err(|e| CaptureError::internal(e.to_string()))?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_validate_and_round_trip() {
        let s = Settings::defaults("/videos/CaptureOne");
        s.validate().unwrap();
        let back: Settings = serde_json::from_value(s.to_json()).unwrap();
        assert_eq!(back, s);
        // Key spelling matches the TS schema.
        let v = s.to_json();
        for key in ["schemaVersion", "outputDirectory", "fileNameTemplate", "screenshotFormat", "jpegQuality", "countdownSec", "minFreeDiskMb", "showRecordingIndicator", "minimizeOnRecord", "companionHubEnabled", "theme"] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["shortcuts"]["regionScreenshot"], "CommandOrControl+Shift+A");
        assert_eq!(v["video"]["hardwareAcceleration"], "auto");
    }

    #[test]
    fn parses_ts_default_settings_json() {
        // Output of `defaultSettings("/v")` from packages/settings.
        let ts = json!({
            "schemaVersion": 1, "outputDirectory": "/v", "fileNameTemplate": "CaptureOne {date} {time}",
            "video": { "codec": "h264", "resolution": { "mode": "source" }, "fps": 30, "quality": "medium", "keyframeIntervalSec": 2, "hardwareAcceleration": "auto", "showCursor": true },
            "audio": { "microphone": { "enabled": false, "gain": 1 }, "system": { "enabled": false, "gain": 1 }, "trackMode": "mixed", "sampleRate": 48000, "bitrateKbps": 160 },
            "screenshotFormat": "png", "jpegQuality": 90, "countdownSec": 3, "minFreeDiskMb": 500,
            "shortcuts": { "startStop": "CommandOrControl+Shift+R", "pauseResume": "CommandOrControl+Shift+P", "screenshot": "CommandOrControl+Shift+S", "regionScreenshot": "CommandOrControl+Shift+A" },
            "showRecordingIndicator": true, "minimizeOnRecord": false, "companionHubEnabled": false, "theme": "system"
        });
        let s = from_value_strict(&ts).unwrap();
        assert_eq!(s, Settings::defaults("/v"));
    }

    #[test]
    fn lenient_load_replaces_only_invalid_fields() {
        let raw = json!({
            "countdownSec": 10,
            "theme": "neon",
            "jpegQuality": 300,
            "video": { "codec": "h264", "resolution": { "mode": "source" }, "fps": 25, "quality": "medium" },
            "fileNameTemplate": "Rec {n}",
            "maxDurationSec": 600,
            "unknownKey": true
        });
        let (s, dropped) = from_value_lenient(&raw, "/default");
        assert_eq!(s.countdown_sec, 10);
        assert_eq!(s.file_name_template, "Rec {n}");
        assert_eq!(s.max_duration_sec, Some(600));
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.jpeg_quality, 90);
        assert_eq!(s.video.fps, 30);
        assert_eq!(s.output_directory, "/default");
        let mut d = dropped.clone();
        d.sort();
        assert_eq!(d, vec!["jpegQuality", "theme", "video"]);
    }

    #[test]
    fn load_falls_back_for_missing_and_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path(), "/v"), Settings::defaults("/v"));
        std::fs::write(settings_path(dir.path()), "{ not json").unwrap();
        assert_eq!(load(dir.path(), "/v"), Settings::defaults("/v"));
        std::fs::write(settings_path(dir.path()), "[1,2,3]").unwrap();
        assert_eq!(load(dir.path(), "/v"), Settings::defaults("/v"));
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::defaults("/data/rec");
        s.theme = Theme::Dark;
        s.countdown_sec = 5;
        s.audio.microphone.enabled = true;
        s.shortcuts.screenshot = "Alt+F9".into();
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path(), "/other"), s);
        assert!(!dir.path().join("settings.json.tmp").exists());
    }

    #[test]
    fn strict_parse_rejects_invalid_input() {
        let mut v = Settings::defaults("/v").to_json();
        v["countdownSec"] = json!(7);
        assert_eq!(from_value_strict(&v).unwrap_err().code, CaptureErrorCode::InvalidConfig);
        let mut v = Settings::defaults("/v").to_json();
        v["outputDirectory"] = json!("");
        assert_eq!(from_value_strict(&v).unwrap_err().code, CaptureErrorCode::InvalidConfig);
        let mut v = Settings::defaults("/v").to_json();
        v["video"]["quality"] = json!("custom");
        assert!(from_value_strict(&v).is_err(), "custom quality requires a bitrate");
        v["video"]["bitrateKbps"] = json!(8000);
        assert!(from_value_strict(&v).is_ok());
    }
}
