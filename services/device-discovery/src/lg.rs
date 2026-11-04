//! LG webOS "SSAP" second-screen client (WebSocket JSON protocol used by LG's own remote apps and
//! by community tools such as lgtv2 / aiowebostv).
//!
//! Verified only against the fake SSAP server in `tests/`; **no real LG TV was available**. The
//! pairing manifest requests the permissions below without LG's signed-manifest block; firmware
//! that insists on the signed block can be served by passing a custom manifest in
//! [`LgClientConfig::manifest`].
//!
//! `ssap://tv/executeOneShot` (TV picture screenshot) is model/firmware dependent, needs the user
//! to accept pairing on the TV, and protected (HDCP/DRM) content comes back black.

use crate::keys::{lg_action, LgKeyAction, TvRemoteKey};
use crate::store::{CredentialStore, Vendor};
use crate::transport::{http_get, is_timeout, net_err, ws_connect, TvSocket};
use capture_core::*;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::Message;

pub const LG_PERMISSIONS: &[&str] = &[
    "LAUNCH",
    "APP_TO_APP",
    "CONTROL_AUDIO",
    "CONTROL_DISPLAY",
    "CONTROL_INPUT_JOYSTICK",
    "CONTROL_INPUT_MEDIA_PLAYBACK",
    "CONTROL_INPUT_TV",
    "CONTROL_MOUSE_AND_KEYBOARD",
    "CONTROL_POWER",
    "CONTROL_TV_SCREEN",
    "READ_APP_STATUS",
    "READ_COUNTRY_INFO",
    "READ_NETWORK_STATE",
    "READ_POWER_STATE",
    "READ_RUNNING_APPS",
    "READ_SETTINGS",
    "READ_TV_CURRENT_TIME",
    "READ_UPDATE_INFO",
    "WRITE_NOTIFICATION_TOAST",
];

pub fn default_manifest() -> Value {
    json!({
        "manifestVersion": 1,
        "appVersion": APP_VERSION,
        "signed": {
            "appId": "dev.captureone.desktop",
            "vendorId": "dev.captureone",
            "localizedAppNames": { "": "CaptureOne" },
            "localizedVendorNames": { "": "CaptureOne" },
            "permissions": [],
            "created": "20261104"
        },
        "permissions": LG_PERMISSIONS,
    })
}

#[derive(Debug, Clone)]
pub struct LgClientConfig {
    /// `(tls, port)` endpoints tried in order. Current firmware only serves wss on 3001;
    /// older models serve ws on 3000.
    pub endpoints: Vec<(bool, u16)>,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    /// How long to wait for the user to accept the pairing prompt on the TV.
    pub pairing_timeout: Duration,
    pub manifest: Value,
}

impl Default for LgClientConfig {
    fn default() -> Self {
        Self {
            endpoints: vec![(true, 3001), (false, 3000)],
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            pairing_timeout: Duration::from_secs(60),
            manifest: default_manifest(),
        }
    }
}

pub struct LgClient {
    ip: IpAddr,
    ws: TvSocket,
    pointer: Option<TvSocket>,
    secure: bool,
    pin: Option<String>,
    next_id: u64,
    cfg: LgClientConfig,
}

fn ssap_error(uri: &str, msg: &str) -> CaptureError {
    let lower = msg.to_ascii_lowercase();
    let code = if lower.contains("401") || lower.contains("permission") || lower.contains("403") {
        CaptureErrorCode::PermissionDenied
    } else if lower.contains("404") || lower.contains("no such service") || lower.contains("unknown uri") {
        CaptureErrorCode::Unsupported
    } else {
        CaptureErrorCode::Network
    };
    CaptureError::new(code, format!("LG TV rejected {uri}: {msg}"))
}

fn read_json(ws: &mut TvSocket) -> Result<Option<Value>, tungstenite::Error> {
    match ws.read()? {
        Message::Text(t) => Ok(serde_json::from_str(t.as_str()).ok()),
        Message::Close(_) => Err(tungstenite::Error::ConnectionClosed),
        _ => Ok(None),
    }
}

impl LgClient {
    /// Connects and registers. Without a stored client-key the TV shows a pairing prompt and this
    /// blocks until the user accepts (client-key is then persisted), rejects, or `pairing_timeout`.
    pub fn connect(ip: IpAddr, store: Arc<CredentialStore>, cfg: LgClientConfig) -> CaptureResult<Self> {
        let key = ip.to_string();
        let mut last_err = net_err(format!("no SSAP endpoint configured for {ip}"));
        for &(tls, port) in &cfg.endpoints {
            let creds = store.get(Vendor::Lg, &key);
            let url = format!("{}://{}:{port}/", if tls { "wss" } else { "ws" }, crate::transport::host_literal(ip));
            let pin = if tls { creds.cert_sha256.clone() } else { None };
            let (mut ws, fp) = match ws_connect(&url, ip, pin, cfg.connect_timeout) {
                Ok(v) => v,
                Err(e) => {
                    log::debug!("lg: {url}: {}", e.message);
                    last_err = e;
                    continue;
                }
            };
            let client_key = Self::register(&mut ws, creds.secret.as_deref(), &cfg)?;
            store.update(Vendor::Lg, &key, |c| {
                c.secret = Some(client_key.clone());
                if tls && c.cert_sha256.is_none() {
                    c.cert_sha256 = fp.clone();
                }
            })?;
            let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(500)));
            return Ok(Self { ip, ws, pointer: None, secure: tls, pin: fp, next_id: 1, cfg });
        }
        Err(last_err)
    }

    fn register(ws: &mut TvSocket, client_key: Option<&str>, cfg: &LgClientConfig) -> CaptureResult<String> {
        let mut payload = json!({ "forcePairing": false, "pairingType": "PROMPT", "manifest": cfg.manifest });
        if let Some(k) = client_key {
            payload["client-key"] = json!(k);
        }
        let msg = json!({ "type": "register", "id": "register_0", "payload": payload });
        ws.send(Message::text(msg.to_string())).map_err(|e| net_err(format!("SSAP register: {e}")))?;
        let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(500)));
        let deadline = Instant::now() + cfg.pairing_timeout;
        let mut prompted = false;
        while Instant::now() < deadline {
            let v = match read_json(ws) {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(e) if is_timeout(&e) => continue,
                Err(e) => return Err(net_err(format!("SSAP connection lost during pairing: {e}"))),
            };
            if v["id"] != "register_0" {
                continue;
            }
            match v["type"].as_str() {
                Some("registered") => {
                    return v["payload"]["client-key"]
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| net_err("LG TV registered without returning a client-key"));
                }
                Some("response") => {
                    if v["payload"]["pairingType"] == "PROMPT" && !prompted {
                        prompted = true;
                        log::info!("lg: accept the CaptureOne pairing prompt on the TV");
                    }
                }
                Some("error") => {
                    let err = v["error"].as_str().unwrap_or("pairing failed");
                    let lower = err.to_ascii_lowercase();
                    let code = if lower.contains("reject") || lower.contains("cancel") || lower.contains("403") || lower.contains("denied") {
                        CaptureErrorCode::PermissionDenied
                    } else {
                        CaptureErrorCode::Network
                    };
                    return Err(CaptureError::new(code, format!("LG TV pairing failed: {err}")));
                }
                _ => {}
            }
        }
        Err(CaptureError::new(
            CaptureErrorCode::PermissionDenied,
            if prompted { "Pairing prompt on the LG TV was not accepted in time" } else { "LG TV did not answer the pairing request" },
        ))
    }

    pub fn ip(&self) -> IpAddr {
        self.ip
    }

    /// Sends an SSAP request and waits for its response payload.
    pub fn request(&mut self, uri: &str, payload: Value) -> CaptureResult<Value> {
        let id = format!("req_{}", self.next_id);
        self.next_id += 1;
        let msg = json!({ "type": "request", "id": id, "uri": uri, "payload": payload });
        self.ws.send(Message::text(msg.to_string())).map_err(|e| net_err(format!("SSAP {uri}: {e}")))?;
        let deadline = Instant::now() + self.cfg.request_timeout;
        while Instant::now() < deadline {
            let v = match read_json(&mut self.ws) {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(e) if is_timeout(&e) => continue,
                Err(e) => return Err(net_err(format!("SSAP {uri}: {e}"))),
            };
            if v["id"] != id.as_str() {
                continue;
            }
            return match v["type"].as_str() {
                Some("response") => {
                    let p = v["payload"].clone();
                    if p["returnValue"] == false {
                        Err(ssap_error(uri, p["errorText"].as_str().or(p["errorMessage"].as_str()).unwrap_or("returnValue false")))
                    } else {
                        Ok(p)
                    }
                }
                Some("error") => Err(ssap_error(uri, v["error"].as_str().unwrap_or("error"))),
                other => Err(net_err(format!("SSAP {uri}: unexpected message type {other:?}"))),
            };
        }
        Err(net_err(format!("SSAP {uri}: no response within {:?}", self.cfg.request_timeout)))
    }

    /// `ssap://system/getSystemInfo` (modelName, features, receiverType...).
    pub fn system_info(&mut self) -> CaptureResult<Value> {
        self.request("ssap://system/getSystemInfo", json!({}))
    }

    /// `ssap://com.webos.service.update/getCurrentSWInformation` (product_name, major_ver...).
    pub fn software_info(&mut self) -> CaptureResult<Value> {
        self.request("ssap://com.webos.service.update/getCurrentSWInformation", json!({}))
    }

    fn pointer_socket(&mut self) -> CaptureResult<&mut TvSocket> {
        if self.pointer.is_none() {
            let p = self.request("ssap://com.webos.service.networkinput/getPointerInputSocket", json!({}))?;
            let path = p["socketPath"].as_str().ok_or_else(|| net_err("LG TV returned no pointer socketPath"))?;
            let pin = if path.starts_with("wss://") && self.secure { self.pin.clone() } else { None };
            let (ws, _) = ws_connect(path, self.ip, pin, self.cfg.connect_timeout)?;
            self.pointer = Some(ws);
        }
        self.pointer.as_mut().ok_or_else(|| CaptureError::internal("pointer socket missing"))
    }

    /// Sends a button press on the pointer input socket (`type:button\nname:UP\n\n`).
    pub fn press_button(&mut self, name: &str) -> CaptureResult<()> {
        let frame = format!("type:button\nname:{name}\n\n");
        let first = self.pointer_socket()?.send(Message::text(frame.clone()));
        if first.is_err() {
            // The TV closes idle pointer sockets; reopen once.
            self.pointer = None;
            self.pointer_socket()?.send(Message::text(frame)).map_err(|e| net_err(format!("LG pointer socket: {e}")))?;
        }
        Ok(())
    }

    pub fn send_key(&mut self, key: TvRemoteKey) -> CaptureResult<()> {
        match lg_action(key) {
            LgKeyAction::Button(b) => self.press_button(b),
            LgKeyAction::Request(uri) => self.request(uri, json!({})).map(|_| ()),
        }
    }

    /// Captures the TV picture with `ssap://tv/executeOneShot`, downloads the returned image from
    /// the TV and writes it into `output_dir`. The image is decoded to validate it; an all-black
    /// frame is reported as a validation warning (protected content is blanked by the TV).
    pub fn screenshot(&mut self, output_dir: &Path, tv_name: &str) -> CaptureResult<MediaMetadata> {
        let resp = self.request("ssap://tv/executeOneShot", json!({}))?;
        let uri = resp["imageUri"].as_str().ok_or_else(|| {
            CaptureError::unsupported("This LG TV did not return a screenshot (executeOneShot is model/firmware dependent)")
        })?;
        let pin = if uri.starts_with("https://") && self.secure { self.pin.clone() } else { None };
        let http = http_get(uri, Some(self.ip), pin, Duration::from_secs(20), 64 * 1024 * 1024)?;
        if http.status != 200 {
            return Err(net_err(format!("downloading the TV screenshot failed: HTTP {}", http.status)));
        }
        crate::media::save_screenshot(&http.body, output_dir, tv_name, "webos")
    }

    pub fn close(mut self) {
        if let Some(mut p) = self.pointer.take() {
            let _ = p.close(None);
            let _ = p.flush();
        }
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}
