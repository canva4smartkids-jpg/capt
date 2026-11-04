//! The Rust companion types must serialize to JSON accepted by the schemas exported from
//! packages/capture-contracts, and must accept the JSON the TypeScript companions send.

use capture_core::*;
use companion_hub::*;
mod common;
use common::{assert_valid, schema};
use serde_json::json;
use std::collections::BTreeMap;

fn report() -> CapabilityReport {
    let mut features = BTreeMap::new();
    features.insert(FeatureKey::RemoteControl, FeatureSupport::native());
    features.insert(FeatureKey::DisplayCapture, FeatureSupport::with(SupportLevel::ExternalHardware, "HDMI capture card"));
    CapabilityReport {
        schema_version: 1,
        platform: Platform::Linux,
        os_name: "Linux".into(),
        os_version: "6.8".into(),
        device_model: None,
        app_version: "0.1.0".into(),
        generated_at: now_iso8601(),
        features,
        encoders: vec![],
        frame_rates: vec![30],
        screenshot_formats: vec![ImageFormat::Png],
        max_recording_resolution: None,
        notes: vec![],
    }
}

fn media() -> MediaMetadata {
    MediaMetadata {
        id: "r1".into(),
        kind: MediaKind::Video,
        path: routes::recording_file("r1"),
        file_name: "a.mp4".into(),
        title: "A".into(),
        created_at: now_iso8601(),
        size_bytes: 10,
        width: 1920,
        height: 1080,
        duration_ms: Some(1000.0),
        fps: Some(30.0),
        video_codec: Some(VideoCodec::H264),
        video_bitrate_kbps: None,
        audio_tracks: vec![],
        image_format: None,
        container: "mp4".into(),
        source_name: None,
        platform: None,
        validation: None,
        thumbnail_path: Some(routes::recording_thumbnail("r1")),
    }
}

fn status() -> RecordingStatus {
    RecordingStatus {
        session_id: "s".into(),
        state: RecordingState::Recording,
        elapsed_ms: 5,
        countdown_remaining_sec: None,
        bytes_written: 1,
        frames_encoded: 2,
        frames_dropped: 0,
        estimated_bytes_per_minute: None,
        free_disk_bytes: None,
        output_path: None,
        encoder: Some("libx264".into()),
        warnings: vec![],
        error: None,
    }
}

#[test]
fn hub_info_pair_response() {
    let hub = HubInfo::new("hub-1", "Studio", report());
    assert_valid("HubInfo", &serde_json::to_value(&hub).unwrap());
    let resp = PairResponse { token: "a".repeat(64), hub };
    assert_valid("PairResponse", &serde_json::to_value(&resp).unwrap());
}

#[test]
fn hub_events() {
    let hub = HubInfo::new("hub-1", "Studio", report());
    for ev in [
        HubEvent::Hello { hub },
        HubEvent::RecordingStatus { status: None },
        HubEvent::RecordingStatus { status: Some(status()) },
        HubEvent::LibraryChanged,
    ] {
        let v = serde_json::to_value(&ev).unwrap();
        assert_valid("HubEvent", &v);
        assert_eq!(serde_json::from_value::<HubEvent>(v).unwrap(), ev);
    }
}

#[test]
fn remote_commands() {
    for c in [
        RemoteCommand::StartRecording { preset: StartPreset::Default },
        RemoteCommand::StartRecording { preset: StartPreset::CaptureDevice },
        RemoteCommand::PauseRecording,
        RemoteCommand::ResumeRecording,
        RemoteCommand::StopRecording,
        RemoteCommand::Screenshot,
    ] {
        let v = serde_json::to_value(c).unwrap();
        assert_valid("RemoteCommand", &v);
        assert_eq!(serde_json::from_value::<RemoteCommand>(v).unwrap(), c);
    }
}

#[test]
fn list_items_and_command_result_media() {
    let m = media();
    assert_valid("MediaMetadata", &serde_json::to_value(&m).unwrap());
    let list = serde_json::to_value(RecordingListResponse { items: vec![m.clone()] }).unwrap();
    for item in list["items"].as_array().unwrap() {
        assert_valid("MediaMetadata", item);
    }
    let r = RemoteCommandResult { ok: true, session_id: Some("s".into()), media: Some(m), error: None };
    let v = serde_json::to_value(&r).unwrap();
    assert_valid("MediaMetadata", &v["media"]);
    assert_eq!(v["ok"], json!(true));
    assert_valid("RecordingStatus", &serde_json::to_value(status()).unwrap());
}

#[test]
fn pair_request_from_typescript_companion() {
    // What apps/tv-web sends (zod output: `notes` may be omitted because it has a default).
    let ts = json!({
        "code": "123456",
        "deviceName": "Samsung TV (QE55Q80)",
        "deviceKind": "samsung-tizen-tv",
        "capabilities": {
            "schemaVersion": 1, "platform": "tizen", "osName": "Tizen", "osVersion": "6.5",
            "appVersion": "0.1.0", "generatedAt": "2026-11-04T10:00:00Z",
            "features": {
                "displayCapture": { "level": "external-hardware", "reason": "HDMI capture card on the desktop" },
                "videoRecording": { "level": "companion-only", "reason": "control the desktop" },
                "remoteControl": { "level": "native" }
            },
            "encoders": [], "frameRates": [], "screenshotFormats": []
        }
    });
    assert_valid("PairRequest", &ts);
    let req: PairRequest = serde_json::from_value(ts).unwrap();
    assert!(req.validate().is_ok());
    assert_eq!(req.device_kind, DeviceKind::SamsungTizenTv);
    let caps = req.capabilities.as_ref().unwrap();
    assert_eq!(caps.level(FeatureKey::DisplayCapture), SupportLevel::ExternalHardware);
    assert_valid("PairRequest", &serde_json::to_value(&req).unwrap());
}

#[test]
fn discovered_device_shape() {
    let d = DiscoveredDevice {
        id: "lg-1".into(),
        name: "LG OLED".into(),
        kind: DeviceKind::LgWebosTv,
        manufacturer: Some("LG Electronics".into()),
        model: None,
        address: "192.168.1.5".into(),
        port: Some(3000),
        discovered_via: vec![DiscoveredVia::Ssdp, DiscoveredVia::Mdns],
        control_protocols: vec![ControlProtocol::LgSsap, ControlProtocol::Dial],
        services: vec!["urn:lge-com:service:webos-second-screen:1".into()],
        last_seen: now_iso8601(),
    };
    assert_valid("DiscoveredDevice", &serde_json::to_value(&d).unwrap());
}

#[test]
fn enum_spellings_match_contract_enums() {
    // Every DeviceKind in the schema must deserialize, and serialize back identically.
    let s = schema("DiscoveredDevice");
    for k in s["properties"]["kind"]["enum"].as_array().unwrap() {
        let kind: DeviceKind = serde_json::from_value(k.clone()).unwrap();
        assert_eq!(&serde_json::to_value(kind).unwrap(), k);
        assert_eq!(kind.as_str(), k.as_str().unwrap());
    }
    for p in s["properties"]["controlProtocols"]["items"]["enum"].as_array().unwrap() {
        let proto: ControlProtocol = serde_json::from_value(p.clone()).unwrap();
        assert_eq!(&serde_json::to_value(proto).unwrap(), p);
    }
}
