//! End-to-end tests of the hub over real sockets on 127.0.0.1 with a fake backend.

use capture_core::*;
use companion_hub::pairing::PairingPolicy;
use companion_hub::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod common;
use common as contract;

struct Fake {
    dir: PathBuf,
    status: Mutex<Option<RecordingStatus>>,
    commands: Mutex<Vec<RemoteCommand>>,
}

fn caps() -> CapabilityReport {
    let mut features = BTreeMap::new();
    features.insert(FeatureKey::RemoteControl, FeatureSupport::native());
    CapabilityReport {
        schema_version: 1,
        platform: Platform::Linux,
        os_name: "Linux".into(),
        os_version: "test".into(),
        device_model: None,
        app_version: "0.1.0".into(),
        generated_at: now_iso8601(),
        features,
        encoders: vec![],
        frame_rates: vec![30],
        screenshot_formats: vec![],
        max_recording_resolution: None,
        notes: vec![],
    }
}

fn video_bytes() -> Vec<u8> {
    (0..200_000u32).map(|i| (i % 251) as u8).collect()
}

impl HubBackend for Fake {
    fn hub_info(&self) -> HubInfo {
        HubInfo::new("hub-test", "Test hub", caps())
    }
    fn list_recordings(&self) -> Vec<MediaMetadata> {
        vec![MediaMetadata {
            id: "rec 1".into(),
            kind: MediaKind::Video,
            path: self.dir.join("clip.mp4").display().to_string(),
            file_name: "clip.mp4".into(),
            title: "Clip".into(),
            created_at: now_iso8601(),
            size_bytes: 200_000,
            width: 1280,
            height: 720,
            duration_ms: Some(2000.0),
            fps: Some(30.0),
            video_codec: Some(VideoCodec::H264),
            video_bitrate_kbps: None,
            audio_tracks: vec![],
            image_format: None,
            container: "mp4".into(),
            source_name: None,
            platform: None,
            validation: None,
            thumbnail_path: Some(self.dir.join("thumb.jpg").display().to_string()),
        }]
    }
    fn recording_path(&self, id: &str) -> Option<PathBuf> {
        (id == "rec 1").then(|| self.dir.join("clip.mp4"))
    }
    fn thumbnail_path(&self, id: &str) -> Option<PathBuf> {
        (id == "rec 1").then(|| self.dir.join("thumb.jpg"))
    }
    fn status(&self) -> Option<RecordingStatus> {
        self.status.lock().unwrap().clone()
    }
    fn command(&self, cmd: RemoteCommand) -> RemoteCommandResult {
        self.commands.lock().unwrap().push(cmd);
        match cmd {
            RemoteCommand::StartRecording { .. } => RemoteCommandResult { ok: true, session_id: Some("sess-1".into()), ..Default::default() },
            RemoteCommand::Screenshot => RemoteCommandResult::err("UNSUPPORTED: no display in test"),
            _ => RemoteCommandResult::ok(),
        }
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    state_dir: PathBuf,
    backend: Arc<Fake>,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let media = tmp.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("clip.mp4"), video_bytes()).unwrap();
    std::fs::write(media.join("thumb.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();
    let state_dir = tmp.path().join("state");
    let backend = Arc::new(Fake { dir: media, status: Mutex::new(None), commands: Mutex::new(vec![]) });
    Env { _tmp: tmp, state_dir, backend }
}

fn start(e: &Env, policy: PairingPolicy) -> HubHandle {
    HubHandle::start_with_options(
        e.backend.clone(),
        HubConfig { bind: IpAddr::from([127, 0, 0, 1]), port: 0, state_dir: e.state_dir.clone() },
        HubOptions { mdns: false, pairing: policy, ..Default::default() },
    )
    .expect("hub starts")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).proxy(None).timeout_global(Some(Duration::from_secs(10))).build().into()
}

fn pair(hub: &HubHandle, code: &str) -> (u16, Value) {
    let mut r = agent()
        .post(format!("{}/api/v1/pair", hub.url()))
        .send_json(json!({ "code": code, "deviceName": "Test TV", "deviceKind": "lg-webos-tv" }))
        .unwrap();
    (r.status().as_u16(), r.body_mut().read_json().unwrap())
}

fn wrong(hub: &HubHandle) -> String {
    if hub.pairing_code() == "000000" { "111111".into() } else { "000000".into() }
}

#[test]
fn info_is_public_and_matches_schema() {
    let e = env();
    let hub = start(&e, PairingPolicy::default());
    assert!(hub.url().starts_with("http://127.0.0.1:"));
    let mut r = agent().get(format!("{}/api/v1/info", hub.url())).call().unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.body_mut().read_json().unwrap();
    contract::assert_valid("HubInfo", &v);
    assert_eq!(v["hubId"], "hub-test");
    hub.stop();
}

#[test]
fn pairing_good_bad_and_persisted() {
    let e = env();
    let hub = start(&e, PairingPolicy::default());
    let (status, body) = pair(&hub, &wrong(&hub));
    assert_eq!(status, 401);
    assert_eq!(body["remainingAttempts"], 4);
    let (status, body) = pair(&hub, "12x456");
    assert_eq!(status, 400, "{body}");
    let (status, body) = pair(&hub, &hub.pairing_code());
    assert_eq!(status, 200, "{body}");
    contract::assert_valid("PairResponse", &body);
    let token = body["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);
    let devices = hub.paired_devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "Test TV");
    assert_eq!(devices[0].device_kind, DeviceKind::LgWebosTv);

    // Unauthenticated / bad token.
    let r = agent().get(format!("{}/api/v1/recordings", hub.url())).call().unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let r = agent().get(format!("{}/api/v1/recordings", hub.url())).header("Authorization", format!("Bearer {}", "f".repeat(64))).call().unwrap();
    assert_eq!(r.status().as_u16(), 401);

    // regenerate: old code stops working, token stays valid.
    let old = hub.pairing_code();
    let new = hub.regenerate_code();
    if new != old {
        assert_eq!(pair(&hub, &old).0, 401);
    }
    hub.stop();

    // Restart with the same state dir: token still valid.
    let hub = start(&e, PairingPolicy::default());
    let r = agent().get(format!("{}/api/v1/status", hub.url())).header("Authorization", format!("Bearer {token}")).call().unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let id = hub.paired_devices()[0].id.clone();
    assert!(hub.unpair(&id).unwrap());
    let r = agent().get(format!("{}/api/v1/status", hub.url())).header("Authorization", format!("Bearer {token}")).call().unwrap();
    assert_eq!(r.status().as_u16(), 401);
}

#[test]
fn pairing_lockout() {
    let e = env();
    let hub = start(&e, PairingPolicy { max_failures: 3, lockout: Duration::from_secs(1), max_lockout: Duration::from_secs(4), rotate_after_failures: 1000 });
    assert_eq!(pair(&hub, &wrong(&hub)).0, 401);
    assert_eq!(pair(&hub, &wrong(&hub)).0, 401);
    let mut r = agent()
        .post(format!("{}/api/v1/pair", hub.url()))
        .send_json(json!({ "code": wrong(&hub), "deviceName": "Attacker", "deviceKind": "unknown" }))
        .unwrap();
    assert_eq!(r.status().as_u16(), 429);
    assert_eq!(r.headers().get("retry-after").unwrap().to_str().unwrap(), "1");
    let body: Value = r.body_mut().read_json().unwrap();
    assert_eq!(body["retryAfterSec"], 1);
    // Correct code refused while locked out.
    assert_eq!(pair(&hub, &hub.pairing_code()).0, 429);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(pair(&hub, &hub.pairing_code()).0, 200);
    assert!(hub.paired_devices().iter().all(|d| d.name != "Attacker"));
}

fn paired(e: &Env) -> (HubHandle, String) {
    let hub = start(e, PairingPolicy::default());
    let (s, body) = pair(&hub, &hub.pairing_code());
    assert_eq!(s, 200);
    let token = body["token"].as_str().unwrap().to_string();
    (hub, token)
}

#[test]
fn list_and_range_fetch() {
    let e = env();
    let (hub, token) = paired(&e);
    let auth = format!("Bearer {token}");
    let mut r = agent().get(format!("{}/api/v1/recordings", hub.url())).header("Authorization", &auth).call().unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let list: Value = r.body_mut().read_json().unwrap();
    let item = &list["items"][0];
    contract::assert_valid("MediaMetadata", item);
    assert_eq!(item["path"], "/api/v1/recordings/rec%201/file", "local paths are not exposed");
    assert_eq!(item["thumbnailPath"], "/api/v1/recordings/rec%201/thumbnail");

    let file_url = format!("{}{}", hub.url(), item["path"].as_str().unwrap());
    let full = video_bytes();
    // Range via header auth.
    let mut r = agent().get(&file_url).header("Authorization", &auth).header("Range", "bytes=100-199").call().unwrap();
    assert_eq!(r.status().as_u16(), 206);
    assert_eq!(r.headers().get("content-type").unwrap(), "video/mp4");
    assert_eq!(r.headers().get("content-range").unwrap(), "bytes 100-199/200000");
    assert_eq!(r.body_mut().read_to_vec().unwrap(), &full[100..200]);
    // Range via ?token= (as used by <video src> on TVs), suffix range.
    let mut r = agent().get(format!("{file_url}?token={token}")).header("Range", "bytes=-10").call().unwrap();
    assert_eq!(r.status().as_u16(), 206);
    assert_eq!(r.body_mut().read_to_vec().unwrap(), &full[full.len() - 10..]);
    // Open-ended range.
    let mut r = agent().get(format!("{file_url}?token={token}")).header("Range", "bytes=199990-").call().unwrap();
    assert_eq!(r.status().as_u16(), 206);
    assert_eq!(r.body_mut().read_to_vec().unwrap().len(), 10);
    // Whole file.
    let mut r = agent().get(format!("{file_url}?token={token}")).call().unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers().get("accept-ranges").unwrap(), "bytes");
    let mut body = Vec::new();
    r.body_mut().as_reader().read_to_end(&mut body).unwrap();
    assert_eq!(body, full);
    // Unsatisfiable range.
    let r = agent().get(format!("{file_url}?token={token}")).header("Range", "bytes=300000-300010").call().unwrap();
    assert_eq!(r.status().as_u16(), 416);
    // No token on the file route.
    let r = agent().get(&file_url).call().unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // Thumbnail and unknown ids.
    let r = agent().get(format!("{}/api/v1/recordings/rec%201/thumbnail?token={token}", hub.url())).call().unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers().get("content-type").unwrap(), "image/jpeg");
    let r = agent().get(format!("{}/api/v1/recordings/nope/file?token={token}", hub.url())).call().unwrap();
    assert_eq!(r.status().as_u16(), 404);
}

#[test]
fn status_and_command_round_trip() {
    let e = env();
    let (hub, token) = paired(&e);
    let auth = format!("Bearer {token}");
    let mut r = agent().get(format!("{}/api/v1/status", hub.url())).header("Authorization", &auth).call().unwrap();
    assert_eq!(r.body_mut().read_to_string().unwrap(), "null");

    let mut r = agent()
        .post(format!("{}/api/v1/command", hub.url()))
        .header("Authorization", &auth)
        .send_json(json!({ "type": "start-recording", "preset": "capture-device" }))
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.body_mut().read_json().unwrap();
    assert_eq!(v, json!({ "ok": true, "sessionId": "sess-1" }));
    assert_eq!(e.backend.commands.lock().unwrap().last().copied(), Some(RemoteCommand::StartRecording { preset: StartPreset::CaptureDevice }));

    let mut r = agent().post(format!("{}/api/v1/command", hub.url())).header("Authorization", &auth).send_json(json!({ "type": "screenshot" })).unwrap();
    let v: Value = r.body_mut().read_json().unwrap();
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("UNSUPPORTED"));

    let r = agent().post(format!("{}/api/v1/command", hub.url())).header("Authorization", &auth).send_json(json!({ "type": "rm-rf" })).unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = agent().post(format!("{}/api/v1/command", hub.url())).send_json(json!({ "type": "stop-recording" })).unwrap();
    assert_eq!(r.status().as_u16(), 401);
    assert_eq!(e.backend.commands.lock().unwrap().len(), 2, "unauthenticated/invalid commands never reach the backend");

    *e.backend.status.lock().unwrap() = Some(RecordingStatus {
        session_id: "sess-1".into(),
        state: RecordingState::Recording,
        elapsed_ms: 1500,
        countdown_remaining_sec: None,
        bytes_written: 1000,
        frames_encoded: 45,
        frames_dropped: 0,
        estimated_bytes_per_minute: None,
        free_disk_bytes: None,
        output_path: None,
        encoder: Some("libx264".into()),
        warnings: vec![],
        error: None,
    });
    let mut r = agent().get(format!("{}/api/v1/status", hub.url())).header("Authorization", &auth).call().unwrap();
    let v: Value = r.body_mut().read_json().unwrap();
    contract::assert_valid("RecordingStatus", &v);
    assert_eq!(v["state"], "recording");
}

fn ws_read(ws: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>) -> Value {
    loop {
        match ws.read().expect("websocket message") {
            tungstenite::Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn websocket_events() {
    let e = env();
    let (hub, token) = paired(&e);
    let ws_url = hub.url().replace("http://", "ws://");
    // Without a token the upgrade is refused.
    match tungstenite::connect(format!("{ws_url}/api/v1/events")) {
        Err(tungstenite::Error::Http(resp)) => assert_eq!(resp.status().as_u16(), 401),
        other => panic!("expected 401, got {:?}", other.map(|_| ())),
    }
    let (mut ws, _) = tungstenite::connect(format!("{ws_url}/api/v1/events?token={token}")).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    }
    let hello = ws_read(&mut ws);
    contract::assert_valid("HubEvent", &hello);
    assert_eq!(hello["type"], "hello");
    let st = ws_read(&mut ws);
    assert_eq!(st, json!({ "type": "recording-status", "status": null }));

    hub.broadcast(HubEvent::LibraryChanged);
    let ev = ws_read(&mut ws);
    assert_eq!(ev, json!({ "type": "library-changed" }));
    contract::assert_valid("HubEvent", &ev);

    // stop() closes the socket and the listener.
    let url = hub.url();
    hub.stop();
    let closed = loop {
        match ws.read() {
            Ok(tungstenite::Message::Close(_)) | Err(_) => break true,
            Ok(_) => continue,
        }
    };
    assert!(closed);
    assert!(agent().get(format!("{url}/api/v1/info")).call().is_err(), "listener is closed after stop");
}

#[test]
fn cors_preflight_for_tv_origins() {
    let e = env();
    let hub = start(&e, PairingPolicy::default());
    let r = agent()
        .options(format!("{}/api/v1/recordings", hub.url()))
        .header("Origin", "null")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .call()
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert_eq!(r.headers().get("access-control-allow-origin").unwrap(), "*");
    assert!(r.headers().get("access-control-allow-credentials").is_none(), "never cookie-based");
    let allow = r.headers().get("access-control-allow-headers").unwrap().to_str().unwrap().to_ascii_lowercase();
    assert!(allow.contains("authorization"));
    let r = agent().get(format!("{}/api/v1/info", hub.url())).header("Origin", "file://").call().unwrap();
    assert_eq!(r.headers().get("access-control-allow-origin").unwrap(), "*");
}

#[test]
fn port_in_use_is_reported() {
    let e = env();
    let hub = start(&e, PairingPolicy::default());
    let port = hub.local_addr().port();
    let err = HubHandle::start_with_options(
        e.backend.clone(),
        HubConfig { bind: IpAddr::from([127, 0, 0, 1]), port, state_dir: e.state_dir.clone() },
        HubOptions { mdns: false, ..Default::default() },
    )
    .err()
    .expect("second bind fails");
    assert_eq!(err.code, CaptureErrorCode::DeviceBusy);
}

#[test]
fn mdns_advertisement_never_blocks_start() {
    // Multicast may be unavailable in CI containers; start must still succeed.
    let e = env();
    let hub = HubHandle::start(e.backend.clone(), HubConfig { bind: IpAddr::from([127, 0, 0, 1]), port: 0, state_dir: e.state_dir.clone() }).unwrap();
    let r = agent().get(format!("{}/api/v1/info", hub.url())).call().unwrap();
    assert_eq!(r.status().as_u16(), 200);
    hub.stop();
}
