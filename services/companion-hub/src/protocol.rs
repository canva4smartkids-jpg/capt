//! Serde mirror of `packages/capture-contracts/src/companion.ts` (zod is the source of truth).
//!
//! Field names serialize as camelCase and enum spellings match the TS literals exactly.
//! `tests/contract.rs` validates serialized values against the exported JSON Schemas.

use capture_core::{CapabilityReport, MediaMetadata, Platform, RecordingStatus};
use serde::{Deserialize, Serialize};

/// `COMPANION_PROTOCOL_VERSION`.
pub const COMPANION_PROTOCOL_VERSION: u32 = 1;
/// `COMPANION_DEFAULT_PORT`.
pub const COMPANION_DEFAULT_PORT: u16 = 47800;
/// mDNS / DNS-SD service type the hub advertises.
pub const COMPANION_MDNS_SERVICE: &str = "_captureone._tcp.local.";

/// REST paths (see `COMPANION_ROUTES`).
pub mod routes {
    pub const INFO: &str = "/api/v1/info";
    pub const PAIR: &str = "/api/v1/pair";
    pub const RECORDINGS: &str = "/api/v1/recordings";
    pub const STATUS: &str = "/api/v1/status";
    pub const COMMAND: &str = "/api/v1/command";
    pub const EVENTS: &str = "/api/v1/events";

    /// `GET /api/v1/recordings/:id/file`, with the id percent-encoded.
    pub fn recording_file(id: &str) -> String {
        format!("{RECORDINGS}/{}/file", super::percent_encode(id))
    }
    /// `GET /api/v1/recordings/:id/thumbnail`, with the id percent-encoded.
    pub fn recording_thumbnail(id: &str) -> String {
        format!("{RECORDINGS}/{}/thumbnail", super::percent_encode(id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKind {
    Desktop,
    AndroidPhone,
    AndroidTablet,
    AndroidTv,
    GoogleTv,
    SamsungTizenTv,
    LgWebosTv,
    OtherTv,
    Unknown,
}

impl DeviceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceKind::Desktop => "desktop",
            DeviceKind::AndroidPhone => "android-phone",
            DeviceKind::AndroidTablet => "android-tablet",
            DeviceKind::AndroidTv => "android-tv",
            DeviceKind::GoogleTv => "google-tv",
            DeviceKind::SamsungTizenTv => "samsung-tizen-tv",
            DeviceKind::LgWebosTv => "lg-webos-tv",
            DeviceKind::OtherTv => "other-tv",
            DeviceKind::Unknown => "unknown",
        }
    }
    pub fn is_tv(&self) -> bool {
        matches!(self, DeviceKind::AndroidTv | DeviceKind::GoogleTv | DeviceKind::SamsungTizenTv | DeviceKind::LgWebosTv | DeviceKind::OtherTv)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlProtocol {
    CaptureoneCompanion,
    LgSsap,
    SamsungRemoteWs,
    AndroidTvRemote,
    Dial,
    UpnpMediaRenderer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoveredVia {
    Ssdp,
    Mdns,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub discovered_via: Vec<DiscoveredVia>,
    pub control_protocols: Vec<ControlProtocol>,
    /// Raw service identifiers (SSDP ST/USN or mDNS service types).
    pub services: Vec<String>,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubInfo {
    /// Always [`COMPANION_PROTOCOL_VERSION`].
    pub protocol_version: u32,
    pub hub_id: String,
    pub name: String,
    pub platform: Platform,
    pub app_version: String,
    pub capabilities: CapabilityReport,
}

impl HubInfo {
    pub fn new(hub_id: impl Into<String>, name: impl Into<String>, capabilities: CapabilityReport) -> Self {
        Self {
            protocol_version: COMPANION_PROTOCOL_VERSION,
            hub_id: hub_id.into(),
            name: name.into(),
            platform: capabilities.platform,
            app_version: capabilities.app_version.clone(),
            capabilities,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    /// Exactly six ASCII digits.
    pub code: String,
    /// 1..=64 characters.
    pub device_name: String,
    pub device_kind: DeviceKind,
    /// The companion's own capability report (e.g. a TV reporting it cannot capture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<CapabilityReport>,
}

impl PairRequest {
    /// Same rules as the zod schema.
    pub fn validate(&self) -> Result<(), String> {
        if self.code.len() != 6 || !self.code.bytes().all(|b| b.is_ascii_digit()) {
            return Err("code must be exactly 6 digits".into());
        }
        let n = self.device_name.chars().count();
        if n == 0 || n > 64 {
            return Err("deviceName must be 1..64 characters".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    /// Bearer token (hex, 256 bits of entropy). Shown once; the hub only stores its SHA-256.
    pub token: String,
    pub hub: HubInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StartPreset {
    /// The desktop's configured default capture target (usually the primary display).
    #[default]
    Default,
    /// Record the HDMI/USB capture device (e.g. a capture card that receives the TV's HDMI output).
    CaptureDevice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum RemoteCommand {
    StartRecording {
        #[serde(default)]
        preset: StartPreset,
    },
    PauseRecording,
    ResumeRecording,
    StopRecording,
    Screenshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCommandResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RemoteCommandResult {
    pub fn ok() -> Self {
        Self { ok: true, ..Default::default() }
    }
    pub fn err(message: impl Into<String>) -> Self {
        Self { ok: false, error: Some(message.into()), ..Default::default() }
    }
    pub fn from_error(e: &capture_core::CaptureError) -> Self {
        Self::err(format!("{:?}: {}", e.code, e.message))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum HubEvent {
    Hello {
        hub: HubInfo,
    },
    RecordingStatus {
        /// `null` when no session is active.
        status: Option<RecordingStatus>,
    },
    LibraryChanged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingListResponse {
    pub items: Vec<MediaMetadata>,
}

/// A companion that completed pairing. Serializes to the `HubStatus.pairedDevices[]`
/// shape of the desktop IPC contract (plus `id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub device_kind: DeviceKind,
    pub paired_at: String,
    /// The capability report the companion sent while pairing, when it was valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<CapabilityReport>,
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes `%XX` sequences (and `+` as space, for query strings). Invalid escapes are kept verbatim.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_roundtrip() {
        let id = "rec 1/ä?x=y";
        let enc = percent_encode(id);
        assert_eq!(enc, "rec%201%2F%C3%A4%3Fx%3Dy");
        assert_eq!(percent_decode(&enc), id);
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("abc%41"), "abcA");
    }

    #[test]
    fn pair_request_validation() {
        let mut r = PairRequest { code: "123456".into(), device_name: "TV".into(), device_kind: DeviceKind::LgWebosTv, capabilities: None };
        assert!(r.validate().is_ok());
        r.code = "12345".into();
        assert!(r.validate().is_err());
        r.code = "12345a".into();
        assert!(r.validate().is_err());
        r.code = "123456".into();
        r.device_name = "x".repeat(65);
        assert!(r.validate().is_err());
        r.device_name = String::new();
        assert!(r.validate().is_err());
    }

    #[test]
    fn remote_command_spellings() {
        let c: RemoteCommand = serde_json::from_str(r#"{"type":"start-recording"}"#).unwrap();
        assert_eq!(c, RemoteCommand::StartRecording { preset: StartPreset::Default });
        let c: RemoteCommand = serde_json::from_str(r#"{"type":"start-recording","preset":"capture-device"}"#).unwrap();
        assert_eq!(c, RemoteCommand::StartRecording { preset: StartPreset::CaptureDevice });
        assert_eq!(serde_json::to_string(&RemoteCommand::PauseRecording).unwrap(), r#"{"type":"pause-recording"}"#);
        assert!(serde_json::from_str::<RemoteCommand>(r#"{"type":"format-disk"}"#).is_err());
    }

    #[test]
    fn hub_event_null_status() {
        let v = serde_json::to_value(HubEvent::RecordingStatus { status: None }).unwrap();
        assert_eq!(v, serde_json::json!({"type":"recording-status","status":null}));
        let v = serde_json::to_value(HubEvent::LibraryChanged).unwrap();
        assert_eq!(v, serde_json::json!({"type":"library-changed"}));
    }
}
