//! Serde mirrors of the desktop IPC payloads (packages/capture-contracts/src/desktop-ipc.ts
//! and events.ts) plus event emission helpers.

use capture_core::*;
use serde::{Deserialize, Serialize};

pub const CAPTURE_EVENT: &str = "capture://event";
pub const SHORTCUT_EVENT: &str = "app://shortcut";
pub const HUB_EVENT: &str = "hub://event";

/// `CaptureEvent` discriminated union (field `type`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum CaptureEvent {
    StateChanged {
        session_id: String,
        from: RecordingState,
        to: RecordingState,
        at: String,
    },
    Status {
        status: RecordingStatus,
    },
    Countdown {
        session_id: String,
        remaining_sec: u32,
    },
    Warning {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        message: String,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        error: CaptureError,
    },
    Completed {
        session_id: String,
        reason: StopReason,
        media: MediaMetadata,
    },
    Screenshot {
        media: MediaMetadata,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShortcutAction {
    StartStop,
    PauseResume,
    Screenshot,
    RegionScreenshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShortcutPayload {
    pub action: ShortcutAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub total_bytes: u64,
    pub video_count: u64,
    pub screenshot_count: u64,
    pub free_disk_bytes: u64,
    pub output_directory: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub app_version: String,
    pub platform: String,
    pub os_version: String,
    pub ffmpeg_path: Option<String>,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_path: Option<String>,
    pub data_directory: String,
    pub log_directory: String,
    pub database_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDeviceInfo {
    pub name: String,
    pub device_kind: String,
    pub paired_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStatus {
    pub enabled: bool,
    pub url: Option<String>,
    pub pairing_code: Option<String>,
    pub paired_devices: Vec<PairedDeviceInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TvPairResult {
    pub paired: bool,
    pub message: String,
}

pub fn emit_capture(app: &tauri::AppHandle, event: CaptureEvent) {
    use tauri::Emitter;
    if let Err(e) = app.emit(CAPTURE_EVENT, &event) {
        log::warn!("failed to emit capture event: {e}");
    }
}

pub fn emit_shortcut(app: &tauri::AppHandle, action: ShortcutAction) {
    use tauri::Emitter;
    if let Err(e) = app.emit(SHORTCUT_EVENT, ShortcutPayload { action }) {
        log::warn!("failed to emit shortcut event: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn schema(name: &str) -> Value {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../../packages/capture-contracts/schema/{name}.schema.json"));
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p:?}: {e}"))).unwrap()
    }

    fn assert_valid(name: &str, value: &Value) {
        let s = schema(name);
        let validator = jsonschema::validator_for(&s).expect("schema compiles");
        let errors: Vec<String> = validator.iter_errors(value).map(|e| format!("{e} at {}", e.instance_path())).collect();
        assert!(errors.is_empty(), "{name} invalid: {errors:#?}\n{}", serde_json::to_string_pretty(value).unwrap());
    }

    fn media() -> MediaMetadata {
        MediaMetadata {
            id: "m1".into(),
            kind: MediaKind::Video,
            path: "/v/a.mp4".into(),
            file_name: "a.mp4".into(),
            title: String::new(),
            created_at: "2026-11-04T10:00:00Z".into(),
            size_bytes: 10,
            width: 1920,
            height: 1080,
            duration_ms: Some(1000.0),
            fps: Some(30.0),
            video_codec: Some(VideoCodec::H264),
            video_bitrate_kbps: Some(4000.0),
            audio_tracks: vec![],
            image_format: None,
            container: "mp4".into(),
            source_name: None,
            platform: Some("linux".into()),
            validation: Some(MediaValidation { valid: true, checked_at: "2026-11-04T10:00:01Z".into(), tool: "ffprobe 6.1".into(), issues: vec![] }),
            thumbnail_path: None,
        }
    }

    fn status() -> RecordingStatus {
        RecordingStatus {
            session_id: "s1".into(),
            state: RecordingState::Recording,
            elapsed_ms: 1500,
            countdown_remaining_sec: None,
            bytes_written: 1024,
            frames_encoded: 45,
            frames_dropped: 0,
            estimated_bytes_per_minute: Some(40_000_000),
            free_disk_bytes: Some(1 << 34),
            output_path: Some("/v/.captureone-sessions/s1".into()),
            encoder: Some("libx264".into()),
            warnings: vec![],
            error: None,
        }
    }

    #[test]
    fn every_capture_event_matches_the_contract_schema() {
        let err = CaptureError::new(CaptureErrorCode::SourceClosed, "window closed");
        let events = vec![
            CaptureEvent::StateChanged { session_id: "s1".into(), from: RecordingState::Countdown, to: RecordingState::Starting, at: "2026-11-04T10:00:00Z".into() },
            CaptureEvent::Status { status: status() },
            CaptureEvent::Countdown { session_id: "s1".into(), remaining_sec: 3 },
            CaptureEvent::Warning { session_id: Some("s1".into()), message: "dropped frames".into() },
            CaptureEvent::Warning { session_id: None, message: "shortcut taken".into() },
            CaptureEvent::Error { session_id: Some("s1".into()), error: err.clone() },
            CaptureEvent::Completed { session_id: "s1".into(), reason: StopReason::MaxDuration, media: media() },
            CaptureEvent::Screenshot { media: MediaMetadata { kind: MediaKind::Screenshot, image_format: Some(ImageFormat::Png), container: "png".into(), duration_ms: None, fps: None, video_codec: None, video_bitrate_kbps: None, ..media() } },
        ];
        for e in &events {
            assert_valid("CaptureEvent", &serde_json::to_value(e).unwrap());
        }
        let v = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(v, json!({ "type": "state-changed", "sessionId": "s1", "from": "countdown", "to": "starting", "at": "2026-11-04T10:00:00Z" }));
        assert_eq!(serde_json::to_value(&events[2]).unwrap(), json!({ "type": "countdown", "sessionId": "s1", "remainingSec": 3 }));
        assert_eq!(serde_json::to_value(&events[6]).unwrap()["reason"], "max-duration");
    }

    #[test]
    fn errors_serialize_as_contract_capture_error() {
        let e = CaptureError::unsupported("Samsung TVs have no screenshot API");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v, json!({ "code": "UNSUPPORTED", "message": "Samsung TVs have no screenshot API", "recoverable": false }));
        assert_valid("CaptureError", &v);
        let io: CaptureError = std::io::Error::from_raw_os_error(28).into();
        assert_valid("CaptureError", &serde_json::to_value(io.with_detail("path", "/v")).unwrap());
    }

    #[test]
    fn ipc_payload_shapes_match_desktop_ipc_ts() {
        let s = serde_json::to_value(StorageUsage { total_bytes: 1, video_count: 2, screenshot_count: 3, free_disk_bytes: 4, output_directory: "/v".into() }).unwrap();
        assert_eq!(s, json!({ "totalBytes": 1, "videoCount": 2, "screenshotCount": 3, "freeDiskBytes": 4, "outputDirectory": "/v" }));
        let d = serde_json::to_value(Diagnostics {
            app_version: "0.1.0".into(),
            platform: "linux".into(),
            os_version: "24.04".into(),
            ffmpeg_path: None,
            ffmpeg_version: None,
            ffprobe_path: Some("/usr/bin/ffprobe".into()),
            data_directory: "/d".into(),
            log_directory: "/l".into(),
            database_path: "/d/library.sqlite3".into(),
        })
        .unwrap();
        let keys: Vec<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 9);
        for k in ["appVersion", "platform", "osVersion", "ffmpegPath", "ffmpegVersion", "ffprobePath", "dataDirectory", "logDirectory", "databasePath"] {
            assert!(keys.contains(&k), "{k}");
        }
        assert!(d["ffmpegPath"].is_null(), "null, not omitted (contract: string | null)");
        let h = serde_json::to_value(HubStatus { enabled: false, url: None, pairing_code: None, paired_devices: vec![PairedDeviceInfo { name: "TV".into(), device_kind: "lg-webos-tv".into(), paired_at: "t".into() }] }).unwrap();
        assert_eq!(h, json!({ "enabled": false, "url": null, "pairingCode": null, "pairedDevices": [{ "name": "TV", "deviceKind": "lg-webos-tv", "pairedAt": "t" }] }));
        assert_eq!(serde_json::to_value(ShortcutPayload { action: ShortcutAction::RegionScreenshot }).unwrap(), json!({ "action": "region-screenshot" }));
        assert_eq!(serde_json::to_value(TvPairResult { paired: false, message: "declined".into() }).unwrap(), json!({ "paired": false, "message": "declined" }));
        let key: device_discovery::TvRemoteKey = serde_json::from_value(json!("VOLUME_UP")).unwrap();
        assert_eq!(key, device_discovery::TvRemoteKey::VolumeUp);
        let d = device_discovery::DiscoveredDevice {
            id: "lg:1".into(),
            name: "Living room".into(),
            kind: device_discovery::DeviceKind::LgWebosTv,
            manufacturer: Some("LG".into()),
            model: None,
            address: "192.168.1.30".into(),
            port: Some(3001),
            discovered_via: vec![device_discovery::DiscoveredVia::Ssdp],
            control_protocols: vec![device_discovery::ControlProtocol::LgSsap],
            services: vec![],
            last_seen: "2026-11-04T10:00:00Z".into(),
        };
        assert_valid("DiscoveredDevice", &serde_json::to_value(&d).unwrap());
    }

    #[test]
    fn command_args_from_the_ui_deserialize() {
        // Exactly what apps/desktop/src/lib/record-form.ts#buildRecordingConfig produces.
        let cfg: RecordingConfig = serde_json::from_value(json!({
            "target": { "kind": "region", "displayId": ":0.0", "rect": { "x": 10, "y": 20, "width": 640, "height": 360 } },
            "video": { "codec": "h264", "resolution": { "mode": "height", "height": 720 }, "fps": 30, "quality": "medium", "keyframeIntervalSec": 2, "hardwareAcceleration": "auto", "showCursor": true },
            "audio": { "microphone": { "enabled": true, "deviceId": "mic0", "gain": 1.2 }, "system": { "enabled": false, "gain": 1 }, "trackMode": "mixed", "sampleRate": 48000, "bitrateKbps": 160 },
            "output": { "directory": "/v", "fileNameTemplate": "CaptureOne {date} {time}", "container": "mp4" },
            "limits": { "maxDurationSec": 300, "minFreeDiskMb": 500 },
            "countdownSec": 3
        }))
        .unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.target, CaptureTarget::Region { display_id: ":0.0".into(), rect: Rect { x: 10, y: 20, width: 640, height: 360 } });
        assert_eq!(cfg.limits.max_duration_sec, Some(300));
        // Minimal input relies on schema defaults.
        let req: ScreenshotRequest = serde_json::from_value(json!({ "target": { "kind": "display", "displayId": "d0" } })).unwrap();
        assert_eq!((req.format, req.jpeg_quality, req.show_cursor, req.delay_sec), (ImageFormat::Png, 90, true, 0));
        let kinds: Vec<SourceKind> = serde_json::from_value(json!(["display", "window"])).unwrap();
        assert_eq!(kinds, vec![SourceKind::Display, SourceKind::Window]);
        let perm: PermissionKind = serde_json::from_value(json!("screenCapture")).unwrap();
        assert_eq!(perm, PermissionKind::ScreenCapture);
        let rect: Option<Rect> = serde_json::from_value(json!(null)).unwrap();
        assert!(rect.is_none());
    }
}
