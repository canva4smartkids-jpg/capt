//! Samsung Tizen TV remote-control WebSocket client (`samsung.remote.control` channel).
//!
//! 2018+ models: `wss://<ip>:8002/api/v2/channels/samsung.remote.control?name=<base64>&token=<t>`
//! (self-signed certificate; the TV shows an "Allow" prompt on first connect and returns a token
//! in `ms.channel.connect`). 2016-2017 models: `ws://<ip>:8001/...` without a token.
//! The API only sends remote keys. **It cannot take screenshots**: `screenshot` always returns
//! UNSUPPORTED (Samsung's Smart TV FAQ states screen capture is not supported in Samsung TV apps). Verified only against the fake server in `tests/`; no real Samsung TV was available.

use crate::keys::{samsung_key, TvRemoteKey};
use crate::store::{CredentialStore, Vendor};
use crate::transport::{base64, host_literal, http_get, is_timeout, net_err, ws_connect, TvSocket};
use capture_core::*;
use companion_hub::protocol::percent_encode;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::Message;

#[derive(Debug, Clone)]
pub struct SamsungClientConfig {
    /// Shown on the TV's "allow" prompt and in its device list.
    pub app_name: String,
    /// `(tls, port)` endpoints tried in order.
    pub endpoints: Vec<(bool, u16)>,
    pub connect_timeout: Duration,
    /// How long to wait for the user to answer the "Allow" prompt.
    pub pairing_timeout: Duration,
}

impl Default for SamsungClientConfig {
    fn default() -> Self {
        Self { app_name: "CaptureOne".into(), endpoints: vec![(true, 8002), (false, 8001)], connect_timeout: Duration::from_secs(5), pairing_timeout: Duration::from_secs(30) }
    }
}

pub struct SamsungClient {
    ip: IpAddr,
    ws: TvSocket,
}

/// The channel URL (token only on TLS endpoints, where the TV issues one).
pub fn channel_url(ip: IpAddr, tls: bool, port: u16, app_name: &str, token: Option<&str>) -> String {
    let mut url = format!(
        "{}://{}:{port}/api/v2/channels/samsung.remote.control?name={}",
        if tls { "wss" } else { "ws" },
        host_literal(ip),
        percent_encode(&base64(app_name.as_bytes()))
    );
    if let (true, Some(t)) = (tls, token) {
        url.push_str("&token=");
        url.push_str(&percent_encode(t));
    }
    url
}

/// The exact `ms.remote.control` click message for a key code.
pub fn key_message(key_code: &str) -> Value {
    json!({
        "method": "ms.remote.control",
        "params": { "Cmd": "Click", "DataOfCmd": key_code, "Option": "false", "TypeOfRemote": "SendRemoteKey" }
    })
}

impl SamsungClient {
    pub fn connect(ip: IpAddr, store: Arc<CredentialStore>, cfg: SamsungClientConfig) -> CaptureResult<Self> {
        let key = ip.to_string();
        let mut last_err = net_err(format!("no Samsung endpoint configured for {ip}"));
        for &(tls, port) in &cfg.endpoints {
            let creds = store.get(Vendor::Samsung, &key);
            let url = channel_url(ip, tls, port, &cfg.app_name, creds.secret.as_deref());
            let pin = if tls { creds.cert_sha256.clone() } else { None };
            let (mut ws, fp) = match ws_connect(&url, ip, pin, cfg.connect_timeout) {
                Ok(v) => v,
                Err(e) => {
                    log::debug!("samsung: {}: {}", url.split('?').next().unwrap_or(""), e.message);
                    last_err = e;
                    continue;
                }
            };
            let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(500)));
            let token = Self::await_connect(&mut ws, cfg.pairing_timeout)?;
            store.update(Vendor::Samsung, &key, |c| {
                if let Some(t) = &token {
                    c.secret = Some(t.clone());
                }
                if tls && c.cert_sha256.is_none() {
                    c.cert_sha256 = fp.clone();
                }
            })?;
            return Ok(Self { ip, ws });
        }
        Err(last_err)
    }

    fn await_connect(ws: &mut TvSocket, timeout: Duration) -> CaptureResult<Option<String>> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let msg = match ws.read() {
                Ok(m) => m,
                Err(e) if is_timeout(&e) => continue,
                Err(e) => return Err(net_err(format!("Samsung TV closed the connection: {e}"))),
            };
            let Message::Text(t) = msg else { continue };
            let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
            match v["event"].as_str() {
                Some("ms.channel.connect") => return Ok(v["data"]["token"].as_str().map(str::to_string).filter(|t| !t.is_empty())),
                Some("ms.channel.unauthorized") => {
                    return Err(CaptureError::new(CaptureErrorCode::PermissionDenied, "The Samsung TV denied access. Allow CaptureOne in the TV prompt (or under Settings > General > External Device Manager > Device Connection Manager)."))
                }
                Some("ms.channel.timeOut") => return Err(CaptureError::new(CaptureErrorCode::PermissionDenied, "Nobody answered the access prompt on the Samsung TV")),
                _ => {}
            }
        }
        Err(CaptureError::new(CaptureErrorCode::PermissionDenied, "The Samsung TV did not confirm the connection in time (answer the prompt on the TV)"))
    }

    pub fn ip(&self) -> IpAddr {
        self.ip
    }

    pub fn send_key_code(&mut self, key_code: &str) -> CaptureResult<()> {
        self.ws.send(Message::text(key_message(key_code).to_string())).map_err(|e| net_err(format!("Samsung remote: {e}")))
    }

    pub fn send_key(&mut self, key: TvRemoteKey) -> CaptureResult<()> {
        self.send_key_code(samsung_key(key))
    }

    /// Samsung's remote API has no screenshot function.
    pub fn screenshot(&mut self) -> CaptureResult<MediaMetadata> {
        Err(CaptureError::unsupported(
            "Samsung's Smart TV FAQ states screen capture is not supported in Samsung TV apps, and the remote-control API has no screenshot function; record the HDMI source with a capture card instead",
        ))
    }

    pub fn close(mut self) {
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}

/// `GET http://<ip>:8001/api/v2/` — model, OS and feature flags the TV reports (no pairing needed).
pub fn device_info(ip: IpAddr, timeout: Duration) -> CaptureResult<Value> {
    let resp = http_get(&format!("http://{}:8001/api/v2/", host_literal(ip)), Some(ip), None, timeout, 256 * 1024)?;
    if resp.status != 200 {
        return Err(net_err(format!("Samsung device info: HTTP {}", resp.status)));
    }
    serde_json::from_slice(&resp.body).map_err(|e| net_err(format!("Samsung device info: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_and_message() {
        let ip: IpAddr = "192.168.1.9".parse().unwrap();
        assert_eq!(channel_url(ip, true, 8002, "CaptureOne", Some("123")), "wss://192.168.1.9:8002/api/v2/channels/samsung.remote.control?name=Q2FwdHVyZU9uZQ%3D%3D&token=123");
        assert_eq!(channel_url(ip, false, 8001, "CaptureOne", Some("123")), "ws://192.168.1.9:8001/api/v2/channels/samsung.remote.control?name=Q2FwdHVyZU9uZQ%3D%3D");
        assert_eq!(
            key_message("KEY_HOME").to_string(),
            r#"{"method":"ms.remote.control","params":{"Cmd":"Click","DataOfCmd":"KEY_HOME","Option":"false","TypeOfRemote":"SendRemoteKey"}}"#
        );
    }
}
