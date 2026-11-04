//! Companion hub integration: `HubBackend` over the desktop library, engine and the same
//! recording/screenshot code paths the UI uses; lifecycle of the `HubHandle`.

use crate::events::{HubStatus, PairedDeviceInfo};
use crate::state::Shared;
use crate::{actions, recording};
use capture_core::*;
use companion_hub::pairing::{Pairing, PairingPolicy};
use companion_hub::{HubBackend, HubConfig, HubHandle, HubInfo, HubOptions, RemoteCommand, RemoteCommandResult, StartPreset, COMPANION_DEFAULT_PORT};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub struct DesktopHub {
    core: Shared,
    /// None in tests: commands that need windows/events (start, screenshot) are refused.
    app: Option<AppHandle>,
}

pub fn hub_dir(core: &Shared) -> PathBuf {
    core.paths.data_dir.join("hub")
}

/// Stable hub id persisted in the hub state dir.
pub fn hub_id(dir: &Path) -> String {
    let file = dir.join("hub-id");
    if let Ok(id) = std::fs::read_to_string(&file) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return id;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&file, &id);
    id
}

/// Human-readable computer name for companions ("CaptureOne on <host>").
pub fn computer_name() -> String {
    let from_env = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok();
    let from_file = || ["/proc/sys/kernel/hostname", "/etc/hostname"].iter().find_map(|p| std::fs::read_to_string(p).ok());
    let host = from_env.or_else(from_file).map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    match host {
        Some(h) => format!("CaptureOne on {h}"),
        None => "CaptureOne Desktop".to_string(),
    }
}

/// Capability report for companions; a minimal honest report when the engine is unavailable.
pub fn hub_capabilities(core: &Shared) -> CapabilityReport {
    match recording::capabilities(core) {
        Ok(r) => r,
        Err(e) => crate::capabilities::with_app_features(
            CapabilityReport {
                schema_version: 1,
                platform: Platform::current(),
                os_name: std::env::consts::OS.to_string(),
                os_version: String::new(),
                device_model: None,
                app_version: core.app_version.clone(),
                generated_at: now_iso8601(),
                features: BTreeMap::new(),
                encoders: vec![],
                frame_rates: vec![],
                screenshot_formats: vec![],
                max_recording_resolution: None,
                notes: vec![format!("Capture engine unavailable: {}", e.message)],
            },
            &core.app_version,
        ),
    }
}

fn ok_session(id: Option<String>) -> RemoteCommandResult {
    RemoteCommandResult { ok: true, session_id: id, ..Default::default() }
}

impl HubBackend for DesktopHub {
    fn hub_info(&self) -> HubInfo {
        HubInfo::new(hub_id(&hub_dir(&self.core)), computer_name(), hub_capabilities(&self.core))
    }

    fn list_recordings(&self) -> Vec<MediaMetadata> {
        match self.core.library.lock().list() {
            Ok(items) => items.into_iter().filter(|m| Path::new(&m.path).is_file()).collect(),
            Err(e) => {
                log::warn!("hub: library unavailable: {e}");
                vec![]
            }
        }
    }

    fn recording_path(&self, id: &str) -> Option<PathBuf> {
        let m = self.core.library.lock().get(id).ok().flatten()?;
        let p = PathBuf::from(m.path);
        p.is_file().then_some(p)
    }

    fn thumbnail_path(&self, id: &str) -> Option<PathBuf> {
        let m = self.core.library.lock().get(id).ok().flatten()?;
        // Screenshots are their own thumbnail.
        let p = PathBuf::from(m.thumbnail_path.or_else(|| (m.kind == MediaKind::Screenshot).then(|| m.path.clone()))?);
        p.is_file().then_some(p)
    }

    fn status(&self) -> Option<RecordingStatus> {
        self.core.active().map(|c| c.status())
    }

    fn command(&self, cmd: RemoteCommand) -> RemoteCommandResult {
        let core = &self.core;
        let session = || core.active().map(|c| c.id.clone());
        let result = match cmd {
            RemoteCommand::StartRecording { preset } => match &self.app {
                Some(app) => actions::start_default(app, core, preset == StartPreset::CaptureDevice).map(|id| ok_session(Some(id))),
                None => Err(CaptureError::unsupported("recording control is not available")),
            },
            RemoteCommand::PauseRecording => actions::pause_active(core).map(|_| ok_session(session())),
            RemoteCommand::ResumeRecording => actions::resume_active(core).map(|_| ok_session(session())),
            RemoteCommand::StopRecording => {
                let id = session();
                actions::stop_active(core).map(|m| RemoteCommandResult { ok: true, session_id: id, media: Some(m), error: None })
            }
            RemoteCommand::Screenshot => match &self.app {
                Some(app) => actions::screenshot_default(app, core).map(|m| RemoteCommandResult { ok: true, media: Some(m), ..Default::default() }),
                None => Err(CaptureError::unsupported("screenshots are not available")),
            },
        };
        result.unwrap_or_else(|e| {
            log::info!("hub command {cmd:?} refused: {e}");
            RemoteCommandResult::from_error(&e)
        })
    }
}

pub struct StartConfig {
    pub bind: IpAddr,
    pub port: u16,
    pub mdns: bool,
}

/// LAN-facing configuration used by the app (all interfaces, default port, mDNS advertisement).
pub fn lan_config(_core: &Shared) -> StartConfig {
    StartConfig { bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED), port: COMPANION_DEFAULT_PORT, mdns: true }
}

/// Starts the hub if it is not running (blocking: binds and spawns the server thread).
pub fn start(core: &Shared, app: Option<AppHandle>, cfg: StartConfig) -> CaptureResult<()> {
    if core.hub.lock().is_some() {
        return Ok(());
    }
    let backend = std::sync::Arc::new(DesktopHub { core: core.clone(), app });
    let handle = HubHandle::start_with_options(
        backend,
        HubConfig { bind: cfg.bind, port: cfg.port, state_dir: hub_dir(core) },
        HubOptions { mdns: cfg.mdns, ..HubOptions::default() },
    )?;
    let mut slot = core.hub.lock();
    if slot.is_none() {
        *slot = Some(handle);
    } else {
        drop(slot);
        handle.stop(); // lost a start race
    }
    Ok(())
}

/// Stops the hub (blocking: joins the server thread, up to a few seconds).
pub fn stop(core: &Shared) {
    let handle = core.hub.lock().take();
    if let Some(h) = handle {
        h.stop();
    }
}

fn device_info(d: companion_hub::PairedDevice) -> PairedDeviceInfo {
    PairedDeviceInfo { name: d.name, device_kind: d.device_kind.as_str().to_string(), paired_at: d.paired_at }
}

pub fn status(core: &Shared) -> HubStatus {
    if let Some(h) = core.hub.lock().as_ref() {
        return HubStatus { enabled: true, url: Some(h.url()), pairing_code: Some(h.pairing_code()), paired_devices: h.paired_devices().into_iter().map(device_info).collect() };
    }
    // Not running: still show who is paired (tokens persist across restarts).
    let paired = Pairing::load(&hub_dir(core), PairingPolicy::default()).map(|p| p.devices()).unwrap_or_default();
    HubStatus { enabled: false, url: None, pairing_code: None, paired_devices: paired.into_iter().map(device_info).collect() }
}

pub fn regenerate_code(core: &Shared) -> CaptureResult<HubStatus> {
    {
        let hub = core.hub.lock();
        let h = hub.as_ref().ok_or_else(|| CaptureError::invalid_state("Turn on the companion hub first."))?;
        h.regenerate_code();
    }
    Ok(status(core))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::settings::Settings;
    use crate::state::{AppCore, AppPaths};
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::Arc;

    fn http(addr: SocketAddr, method: &str, path: &str, token: Option<&str>, body: Option<&Value>) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        let req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}", payload.len());
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("http header end");
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let code: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut body = raw[split + 4..].to_vec();
        if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            let mut out = Vec::new();
            let mut rest = &body[..];
            loop {
                let eol = rest.windows(2).position(|w| w == b"\r\n").unwrap();
                let n = usize::from_str_radix(String::from_utf8_lossy(&rest[..eol]).trim(), 16).unwrap();
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&rest[eol + 2..eol + 2 + n]);
                rest = &rest[eol + 4 + n..];
            }
            body = out;
        }
        (code, body)
    }

    fn json_of(b: &[u8]) -> Value {
        serde_json::from_slice(b).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(b)))
    }

    fn core_with_library(dir: &Path) -> Shared {
        let out = dir.join("Videos");
        std::fs::create_dir_all(&out).unwrap();
        let file = out.join("Demo.mp4");
        std::fs::write(&file, b"not really an mp4, served as bytes").unwrap();
        let mut lib = Library::open(&dir.join("data/library.sqlite3")).unwrap();
        lib.upsert(&MediaMetadata {
            id: "rec-1".into(),
            kind: MediaKind::Video,
            path: file.to_string_lossy().to_string(),
            file_name: "Demo.mp4".into(),
            title: "Demo".into(),
            created_at: "2026-11-04T10:00:00Z".into(),
            size_bytes: 34,
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
            platform: Some("linux".into()),
            validation: None,
            thumbnail_path: None,
        })
        .unwrap();
        let paths = AppPaths { config_dir: dir.join("config"), data_dir: dir.join("data"), log_dir: dir.join("logs"), db_path: dir.join("data/library.sqlite3"), default_output_dir: out.clone() };
        Arc::new(AppCore::new(paths, "0.1.0".into(), Err(CaptureError::unsupported("no capture engine in unit tests")), Settings::defaults(&out.to_string_lossy()), lib))
    }

    #[test]
    fn hub_serves_the_library_to_a_paired_companion() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_library(dir.path());
        start(&core, None, StartConfig { bind: IpAddr::V4(Ipv4Addr::LOCALHOST), port: 0, mdns: false }).unwrap();
        let (addr, code) = {
            let h = core.hub.lock();
            let h = h.as_ref().unwrap();
            (h.local_addr(), h.pairing_code())
        };
        let st = status(&core);
        assert!(st.enabled && st.url.as_deref().unwrap().ends_with(&format!(":{}", addr.port())));
        assert_eq!(st.pairing_code.as_deref(), Some(code.as_str()));

        let (c, info) = http(addr, "GET", "/api/v1/info", None, None);
        assert_eq!(c, 200);
        let info = json_of(&info);
        assert_eq!(info["protocolVersion"], 1);
        assert!(info["name"].as_str().unwrap().starts_with("CaptureOne"));
        assert_eq!(info["capabilities"]["features"]["videoRecording"], Value::Null, "no engine: nothing claimed");

        let (c, _) = http(addr, "GET", "/api/v1/recordings", None, None);
        assert_eq!(c, 401, "library requires a pairing token");
        let (c, bad) = http(addr, "POST", "/api/v1/pair", None, Some(&json!({ "code": "000000", "deviceName": "TV", "deviceKind": "lg-webos-tv" })));
        assert!(c == 401 || c == 403 || code == "000000", "wrong code rejected ({c}: {})", String::from_utf8_lossy(&bad));

        let (c, pair) = http(addr, "POST", "/api/v1/pair", None, Some(&json!({ "code": code, "deviceName": "Living room TV", "deviceKind": "lg-webos-tv" })));
        assert_eq!(c, 200, "{}", String::from_utf8_lossy(&pair));
        let token = json_of(&pair)["token"].as_str().unwrap().to_string();

        let (c, list) = http(addr, "GET", "/api/v1/recordings", Some(&token), None);
        assert_eq!(c, 200);
        let list = json_of(&list);
        assert_eq!(list["items"][0]["id"], "rec-1");
        assert_eq!(list["items"][0]["path"], "/api/v1/recordings/rec-1/file", "local paths are never exposed");

        let (c, bytes) = http(addr, "GET", "/api/v1/recordings/rec-1/file", Some(&token), None);
        assert_eq!(c, 200);
        assert_eq!(bytes, b"not really an mp4, served as bytes");

        let (c, st) = http(addr, "GET", "/api/v1/status", Some(&token), None);
        assert_eq!(c, 200, "{}", String::from_utf8_lossy(&st));

        let (c, res) = http(addr, "POST", "/api/v1/command", Some(&token), Some(&json!({ "type": "pause-recording" })));
        let res = json_of(&res);
        assert!(c == 200 || c == 409, "{c} {res}");
        assert_eq!(res["ok"], false);
        assert!(res["error"].as_str().unwrap().contains("InvalidState"), "{res}");
        let (_, res) = http(addr, "POST", "/api/v1/command", Some(&token), Some(&json!({ "type": "start-recording", "preset": "capture-device" })));
        assert_eq!(json_of(&res)["ok"], false);

        assert_eq!(status(&core).paired_devices.iter().map(|d| (d.name.as_str(), d.device_kind.as_str())).collect::<Vec<_>>(), vec![("Living room TV", "lg-webos-tv")]);
        let old = code.clone();
        let st = regenerate_code(&core).unwrap();
        assert_ne!(st.pairing_code.unwrap(), old);

        stop(&core);
        let st = status(&core);
        assert!(!st.enabled && st.url.is_none());
        assert_eq!(st.paired_devices.len(), 1, "pairings persist while the hub is off");
        assert_eq!(regenerate_code(&core).unwrap_err().code, CaptureErrorCode::InvalidState);
        // Restart keeps the hub id and accepts the existing token.
        let id = hub_id(&hub_dir(&core));
        start(&core, None, StartConfig { bind: IpAddr::V4(Ipv4Addr::LOCALHOST), port: 0, mdns: false }).unwrap();
        let addr = core.hub.lock().as_ref().unwrap().local_addr();
        assert_eq!(http(addr, "GET", "/api/v1/recordings", Some(&token), None).0, 200);
        assert_eq!(hub_id(&hub_dir(&core)), id);
        stop(&core);
    }

    #[test]
    fn thumbnails_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_library(dir.path());
        let hub = DesktopHub { core: core.clone(), app: None };
        assert!(hub.recording_path("rec-1").is_some());
        assert!(hub.thumbnail_path("rec-1").is_none(), "video without a thumbnail");
        assert!(hub.recording_path("nope").is_none());
        std::fs::remove_file(dir.path().join("Videos/Demo.mp4")).unwrap();
        assert!(hub.recording_path("rec-1").is_none());
        assert!(hub.list_recordings().is_empty(), "missing files are not offered");
        assert!(hub.status().is_none());
        assert!(!hub.command(RemoteCommand::Screenshot).ok);
    }
}
