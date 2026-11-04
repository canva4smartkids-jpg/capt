//! Contract tests: Rust mirrors must serialize to JSON accepted by the schemas generated
//! from packages/capture-contracts, and shared logic must match the TS fixtures.

use capture_core::presets::*;
use capture_core::state::allowed_transitions;
use capture_core::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn schema(name: &str) -> Value {
    let p = repo().join(format!("packages/capture-contracts/schema/{name}.schema.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p:?}: {e} (run `pnpm schema`)"))).unwrap()
}

fn assert_valid(name: &str, value: &Value) {
    let s = schema(name);
    let validator = jsonschema::validator_for(&s).expect("schema compiles");
    let errors: Vec<String> = validator.iter_errors(value).map(|e| format!("{} at {}", e, e.instance_path())).collect();
    assert!(errors.is_empty(), "{name} invalid: {errors:#?}\nvalue: {}", serde_json::to_string_pretty(value).unwrap());
}

fn sample_report() -> CapabilityReport {
    let mut features = BTreeMap::new();
    features.insert(FeatureKey::DisplayCapture, FeatureSupport::native());
    features.insert(FeatureKey::SystemAudio, FeatureSupport::with(SupportLevel::Partial, "loopback via PulseAudio monitor").requires(vec![PermissionKind::SystemAudio]));
    features.insert(FeatureKey::TvNativeScreenshot, FeatureSupport::unsupported("desktop"));
    CapabilityReport {
        schema_version: 1,
        platform: Platform::Linux,
        os_name: "Linux".into(),
        os_version: "6.8".into(),
        device_model: None,
        app_version: APP_VERSION.into(),
        generated_at: now_iso8601(),
        features,
        encoders: vec![EncoderInfo { codec: VideoCodec::H264, name: "libx264".into(), hardware: false, verified: true }],
        frame_rates: vec![24, 30, 60],
        screenshot_formats: vec![ImageFormat::Png, ImageFormat::Jpeg],
        max_recording_resolution: Some(Size { width: 3840, height: 2160 }),
        notes: vec![],
    }
}

fn sample_config() -> RecordingConfig {
    RecordingConfig {
        target: CaptureTarget::Region { display_id: "0".into(), rect: Rect { x: 10, y: 20, width: 640, height: 480 } },
        video: VideoSettings {
            codec: VideoCodec::H265,
            resolution: ResolutionSetting::Height { height: 1080 },
            fps: 60,
            quality: QualityPreset::Custom,
            bitrate_kbps: Some(8000),
            keyframe_interval_sec: 2.0,
            hardware_acceleration: HardwareAcceleration::Prefer,
            show_cursor: false,
        },
        audio: AudioSettings {
            microphone: AudioSourceSettings { enabled: true, device_id: Some("hw:0".into()), gain: 1.5 },
            system: AudioSourceSettings::disabled(),
            track_mode: TrackMode::Separate,
            sample_rate: 48000,
            bitrate_kbps: 192,
        },
        output: OutputSettings { directory: "/tmp".into(), file_name_template: "x {n}".into(), container: "mp4".into() },
        limits: RecordingLimits { max_duration_sec: Some(60), max_file_size_mb: None, min_free_disk_mb: 100 },
        countdown_sec: 5,
    }
}

#[test]
fn capability_report_matches_schema() {
    assert_valid("CapabilityReport", &serde_json::to_value(sample_report()).unwrap());
}

#[test]
fn recording_config_roundtrip_and_schema() {
    let cfg = sample_config();
    cfg.validate().unwrap();
    let v = serde_json::to_value(&cfg).unwrap();
    assert_valid("RecordingConfig", &v);
    let back: RecordingConfig = serde_json::from_value(v).unwrap();
    assert_eq!(back, cfg);
}

#[test]
fn accepts_minimal_ts_config_with_defaults() {
    // Shape produced by the UI before zod defaults are applied.
    let v = json!({
        "target": {"kind": "display", "displayId": "0"},
        "video": {"codec": "h264", "resolution": {"mode": "source"}, "fps": 30, "quality": "medium"},
        "audio": {"microphone": {"enabled": false}, "system": {"enabled": true}},
        "output": {"directory": "/tmp"}
    });
    assert_valid("RecordingConfig", &v);
    let cfg: RecordingConfig = serde_json::from_value(v).unwrap();
    assert_eq!(cfg.countdown_sec, 3);
    assert_eq!(cfg.audio.sample_rate, 48000);
    assert_eq!(cfg.audio.bitrate_kbps, 160);
    assert_eq!(cfg.limits.min_free_disk_mb, 500);
    assert!(cfg.video.show_cursor);
    cfg.validate().unwrap();
}

#[test]
fn validate_rejects_bad_config() {
    let mut c = sample_config();
    c.video.fps = 25;
    assert_eq!(c.validate().unwrap_err().code, CaptureErrorCode::InvalidConfig);
    let mut c = sample_config();
    c.video.bitrate_kbps = None;
    assert!(c.validate().is_err());
    let mut c = sample_config();
    c.target = CaptureTarget::Region { display_id: "0".into(), rect: Rect { x: 0, y: 0, width: 8, height: 8 } };
    assert!(c.validate().is_err());
}

#[test]
fn other_types_match_schema() {
    let status = RecordingStatus {
        session_id: "s1".into(),
        state: RecordingState::Paused,
        elapsed_ms: 1234,
        countdown_remaining_sec: None,
        bytes_written: 99,
        frames_encoded: 10,
        frames_dropped: 1,
        estimated_bytes_per_minute: Some(1000),
        free_disk_bytes: Some(5),
        output_path: Some("/tmp/a.mp4".into()),
        encoder: Some("libx264".into()),
        warnings: vec!["w".into()],
        error: Some(CaptureError::new(CaptureErrorCode::StorageFull, "full").with_detail("free", 0)),
    };
    assert_valid("RecordingStatus", &serde_json::to_value(&status).unwrap());

    let media = MediaMetadata {
        id: "m1".into(),
        kind: MediaKind::Video,
        path: "/tmp/a.mp4".into(),
        file_name: "a.mp4".into(),
        title: "a".into(),
        created_at: now_iso8601(),
        size_bytes: 1000,
        width: 1920,
        height: 1080,
        duration_ms: Some(5000.0),
        fps: Some(30.0),
        video_codec: Some(VideoCodec::H264),
        video_bitrate_kbps: Some(4000.0),
        audio_tracks: vec![AudioTrackInfo { index: 1, codec: "aac".into(), channels: 2, sample_rate: 48000, label: Some("mixed".into()), duration_ms: Some(5000.0) }],
        image_format: None,
        container: "mp4".into(),
        source_name: Some("Display 1".into()),
        platform: Some("linux".into()),
        validation: Some(MediaValidation { valid: true, checked_at: now_iso8601(), tool: "ffprobe".into(), issues: vec![ValidationIssue { severity: IssueSeverity::Warning, code: "x".into(), message: "y".into() }] }),
        thumbnail_path: None,
    };
    assert_valid("MediaMetadata", &serde_json::to_value(&media).unwrap());

    let src = CaptureSource { id: "1".into(), kind: SourceKind::Window, name: "Term".into(), width: 800, height: 600, x: Some(-5), y: Some(0), scale_factor: Some(2.0), is_primary: None, app_name: Some("xterm".into()), device_descriptor: None };
    assert_valid("CaptureSource", &serde_json::to_value(&src).unwrap());

    let perm = PermissionStatus::new(PermissionKind::ScreenCapture, PermissionState::NotApplicable);
    assert_valid("PermissionStatus", &serde_json::to_value(&perm).unwrap());

    let req = ScreenshotRequest { target: CaptureTarget::Window { window_id: "9".into() }, format: ImageFormat::Jpeg, jpeg_quality: 80, output_directory: None, file_name_template: None, show_cursor: true, delay_sec: 0 };
    assert_valid("ScreenshotRequest", &serde_json::to_value(&req).unwrap());

    for t in [
        CaptureTarget::Display { display_id: "a".into() },
        CaptureTarget::Device { device_id: "/dev/video0".into() },
    ] {
        assert_valid("CaptureTarget", &serde_json::to_value(&t).unwrap());
    }
}

#[test]
fn enum_spellings_match_ts() {
    let enums: Value = serde_json::from_str(&std::fs::read_to_string(repo().join("packages/capture-contracts/schema/enums.json")).unwrap()).unwrap();
    fn names<T: serde::Serialize>(items: &[T]) -> Vec<String> {
        items.iter().map(|i| serde_json::to_value(i).unwrap().as_str().unwrap().to_string()).collect()
    }
    let check = |key: &str, rust: Vec<String>| {
        let ts: Vec<String> = enums[key].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(rust, ts, "enum {key} differs between Rust and TS");
    };
    use FeatureKey as F;
    check("Platform", names(&[Platform::Windows, Platform::Macos, Platform::Linux, Platform::Android, Platform::AndroidTv, Platform::GoogleTv, Platform::Tizen, Platform::Webos, Platform::OtherTv]));
    check("SupportLevel", names(&[SupportLevel::Native, SupportLevel::Partial, SupportLevel::CompanionOnly, SupportLevel::ExternalHardware, SupportLevel::Unsupported]));
    check(
        "FeatureKey",
        names(&[
            F::Screenshot, F::DisplayCapture, F::WindowCapture, F::RegionCapture, F::MultiDisplay, F::VideoRecording, F::PauseResume, F::CursorCapture, F::MicrophoneAudio, F::SystemAudio, F::SeparateAudioTracks, F::H264, F::H265,
            F::HardwareEncoding, F::GlobalShortcuts, F::RecordingHistory, F::Share, F::HdmiCaptureDevice, F::RemoteControl, F::DeviceDiscovery, F::TvNativeScreenshot, F::FileTransfer,
        ]),
    );
    check("PermissionKind", names(&[PermissionKind::ScreenCapture, PermissionKind::Microphone, PermissionKind::SystemAudio, PermissionKind::Storage, PermissionKind::Notifications, PermissionKind::LocalNetwork]));
    check("PermissionState", names(&[PermissionState::Granted, PermissionState::Denied, PermissionState::Prompt, PermissionState::Restricted, PermissionState::NotApplicable, PermissionState::PerSession]));
    use CaptureErrorCode as E;
    check(
        "CaptureErrorCode",
        names(&[E::PermissionDenied, E::Unsupported, E::SourceNotFound, E::SourceClosed, E::EncoderUnavailable, E::EncoderFailed, E::StorageFull, E::IoError, E::InvalidConfig, E::InvalidState, E::DrmProtected, E::Interrupted, E::DeviceBusy, E::Network, E::Internal]),
    );
    use RecordingState as R;
    check("RecordingState", names(&[R::Idle, R::Countdown, R::Starting, R::Recording, R::Paused, R::Stopping, R::Finalizing, R::Completed, R::Failed]));
}

#[test]
fn transitions_match_ts() {
    let table: BTreeMap<String, Vec<String>> = serde_json::from_str(&std::fs::read_to_string(repo().join("packages/capture-contracts/schema/recording-transitions.json")).unwrap()).unwrap();
    for (from, tos) in table {
        let from: RecordingState = serde_json::from_value(Value::String(from)).unwrap();
        let rust: Vec<String> = allowed_transitions(from).iter().map(|s| serde_json::to_value(s).unwrap().as_str().unwrap().to_string()).collect();
        assert_eq!(rust, tos, "transitions from {from:?}");
    }
}

#[test]
fn presets_match_ts_fixtures() {
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(repo().join("tests/contract/fixtures/presets-cases.json")).unwrap()).unwrap();
    let mut n = 0;
    for c in fx["resolution"].as_array().unwrap() {
        let src: Size = serde_json::from_value(c["source"].clone()).unwrap();
        let setting: ResolutionSetting = serde_json::from_value(c["setting"].clone()).unwrap();
        let exp: Size = serde_json::from_value(c["expected"].clone()).unwrap();
        assert_eq!(resolve_output_size(src, setting), exp, "resolution case {c}");
        n += 1;
    }
    for c in fx["bitrate"].as_array().unwrap() {
        let size: Size = serde_json::from_value(c["size"].clone()).unwrap();
        let codec: VideoCodec = serde_json::from_value(c["codec"].clone()).unwrap();
        let q: QualityPreset = serde_json::from_value(c["quality"].clone()).unwrap();
        let got = video_bitrate_kbps(size, c["fps"].as_u64().unwrap() as u32, codec, q, None);
        assert_eq!(got as u64, c["expectedKbps"].as_u64().unwrap(), "bitrate case {c}");
        n += 1;
    }
    for c in fx["fileSize"].as_array().unwrap() {
        let got = estimate_file_size_bytes(c["videoKbps"].as_u64().unwrap() as u32, c["audioKbps"].as_u64().unwrap() as u32, c["durationSec"].as_f64().unwrap());
        assert_eq!(got, c["expectedBytes"].as_u64().unwrap(), "file size case {c}");
        n += 1;
    }
    assert!(n > 100);
}
