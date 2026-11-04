//! Turns everything learned about one IP address (SSDP, descriptions, mDNS) into a contract
//! [`DiscoveredDevice`].
//!
//! Heuristics (documented because they are heuristics):
//! * LG webOS: the `webos-second-screen` SSDP service, a `WebOS` SERVER header, or LG Electronics
//!   as manufacturer together with DIAL/MediaRenderer.
//! * Samsung Tizen: the `RemoteControlReceiver` device or `MultiScreenService`, or Samsung as
//!   manufacturer with a Tizen SERVER header or DIAL. Pre-2016 (Orsay) Samsung TVs cannot be told
//!   apart reliably and will fail the remote-control handshake.
//! * Android TV / Google TV: `_androidtvremote2._tcp`. Google TV is an Android TV OS UI and is only
//!   reported when the Cast model/name mentions Google TV or Chromecast; otherwise `android-tv`.
//! * `_captureone._tcp`: a CaptureOne desktop hub.
//! * Any other DIAL server or video-capable Cast receiver: `other-tv`.

use crate::description::DeviceDescription;
use crate::ssdp::SsdpResponse;
use companion_hub::protocol::{ControlProtocol, DeviceKind, DiscoveredDevice, DiscoveredVia};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

pub const LG_SSAP_TLS_PORT: u16 = 3001;
pub const SAMSUNG_REMOTE_TLS_PORT: u16 = 8002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsRecord {
    /// e.g. `_googlecast._tcp.local.`
    pub service_type: String,
    pub instance: String,
    pub port: u16,
    pub txt: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Evidence {
    pub ip: IpAddr,
    pub ssdp: Vec<SsdpResponse>,
    pub descriptions: Vec<DeviceDescription>,
    pub mdns: Vec<MdnsRecord>,
}

impl Evidence {
    pub fn new(ip: IpAddr) -> Self {
        Self { ip, ssdp: vec![], descriptions: vec![], mdns: vec![] }
    }
    fn has_mdns(&self, ty: &str) -> Option<&MdnsRecord> {
        self.mdns.iter().find(|m| m.service_type.trim_end_matches('.').trim_end_matches(".local") == ty)
    }
    fn identifiers(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for r in &self.ssdp {
            ids.extend(r.st.iter().cloned());
            ids.extend(r.usn.iter().cloned());
        }
        for d in &self.descriptions {
            ids.extend(d.device_type.iter().cloned());
            ids.extend(d.service_types.iter().cloned());
        }
        ids.into_iter().map(|s| s.to_ascii_lowercase()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub kind: DeviceKind,
    pub protocols: Vec<ControlProtocol>,
    pub port: Option<u16>,
}

pub fn classify(ev: &Evidence) -> Classification {
    let ids = ev.identifiers();
    let has = |needle: &str| ids.iter().any(|i| i.contains(needle));
    let manu: String = ev.descriptions.iter().filter_map(|d| d.manufacturer.clone()).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    let server: String = ev.ssdp.iter().filter_map(|r| r.server.clone()).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    let dial = has("dial-multiscreen-org:service:dial") || has("dial-multiscreen-org:device:dial");
    let renderer = has("schemas-upnp-org:device:mediarenderer");
    let captureone = ev.has_mdns("_captureone._tcp");
    let atv = ev.has_mdns("_androidtvremote2._tcp");
    let cast = ev.has_mdns("_googlecast._tcp");

    let lg = has("lge-com:service:webos-second-screen") || server.contains("webos") || (manu.contains("lg electronics") && (dial || renderer));
    let samsung = has("samsung.com:device:remotecontrolreceiver")
        || has("samsung.com:service:multiscreenservice")
        || (manu.contains("samsung") && (server.contains("tizen") || dial));

    let mut protocols = BTreeSet::new();
    if dial {
        protocols.insert(ControlProtocol::Dial);
    }
    if renderer {
        protocols.insert(ControlProtocol::UpnpMediaRenderer);
    }
    let (kind, port) = if let Some(rec) = captureone {
        protocols.clear();
        protocols.insert(ControlProtocol::CaptureoneCompanion);
        (DeviceKind::Desktop, Some(rec.port))
    } else if lg {
        protocols.insert(ControlProtocol::LgSsap);
        (DeviceKind::LgWebosTv, Some(LG_SSAP_TLS_PORT))
    } else if samsung {
        protocols.insert(ControlProtocol::SamsungRemoteWs);
        (DeviceKind::SamsungTizenTv, Some(SAMSUNG_REMOTE_TLS_PORT))
    } else if let Some(rec) = atv {
        protocols.insert(ControlProtocol::AndroidTvRemote);
        let hint = cast
            .map(|c| format!("{} {}", c.txt.get("md").cloned().unwrap_or_default(), c.txt.get("fn").cloned().unwrap_or_default()))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let model_hint: String = ev.descriptions.iter().filter_map(|d| d.model_name.clone()).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        let google = hint.contains("google tv") || hint.contains("chromecast") || model_hint.contains("google tv");
        (if google { DeviceKind::GoogleTv } else { DeviceKind::AndroidTv }, Some(rec.port))
    } else if cast.map(cast_has_video).unwrap_or(false) || dial {
        (DeviceKind::OtherTv, None)
    } else {
        (DeviceKind::Unknown, None)
    };
    Classification { kind, protocols: protocols.into_iter().collect(), port }
}

/// Cast `ca` TXT capability bitfield: bit 0 = video out.
fn cast_has_video(rec: &MdnsRecord) -> bool {
    rec.txt.get("ca").and_then(|v| v.parse::<u32>().ok()).map(|ca| ca & 1 == 1).unwrap_or(false)
}

fn clean(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Builds the contract device, or `None` for devices we cannot control and that are not TVs
/// (routers, printers, NAS...).
pub fn to_device(ev: &Evidence, now_iso: &str) -> Option<DiscoveredDevice> {
    let c = classify(ev);
    if c.kind == DeviceKind::Unknown && c.protocols.is_empty() {
        return None;
    }
    // Prefer the description that carries a manufacturer (the "real" root device).
    let mut descs: Vec<&DeviceDescription> = ev.descriptions.iter().collect();
    descs.sort_by_key(|d| (d.manufacturer.is_none(), d.udn.clone()));
    let primary = descs.first().copied();
    let cast = ev.has_mdns("_googlecast._tcp");
    let hub = ev.has_mdns("_captureone._tcp");

    let name = hub
        .and_then(|h| h.txt.get("name").and_then(|n| clean(n)))
        .or_else(|| primary.and_then(|d| d.friendly_name.as_deref().and_then(clean)))
        .or_else(|| cast.and_then(|m| m.txt.get("fn").and_then(|n| clean(n))))
        .or_else(|| ev.mdns.first().and_then(|m| clean(&m.instance)))
        .unwrap_or_else(|| format!("Device at {}", ev.ip));
    let manufacturer = primary.and_then(|d| d.manufacturer.as_deref().and_then(clean));
    let model = primary
        .and_then(|d| d.model_name.as_deref().and_then(clean))
        .or_else(|| cast.and_then(|m| m.txt.get("md").and_then(|n| clean(n))));

    let id = if let Some(h) = hub {
        format!("captureone:{}", h.txt.get("id").cloned().unwrap_or_else(|| ev.ip.to_string()))
    } else if let Some(udn) = descs.iter().filter_map(|d| d.udn.clone()).min() {
        udn.to_ascii_lowercase()
    } else if let Some(cid) = cast.and_then(|m| m.txt.get("id")) {
        format!("cast:{cid}")
    } else if let Some(uuid) = ev.ssdp.iter().filter_map(|r| r.usn.as_deref()).filter_map(|u| u.split("::").next()).filter(|u| u.starts_with("uuid:")).min() {
        uuid.to_ascii_lowercase()
    } else {
        format!("ip:{}", ev.ip)
    };

    let mut via = BTreeSet::new();
    if !ev.ssdp.is_empty() {
        via.insert(DiscoveredVia::Ssdp);
    }
    if !ev.mdns.is_empty() {
        via.insert(DiscoveredVia::Mdns);
    }
    let mut services = BTreeSet::new();
    for r in &ev.ssdp {
        if let Some(st) = &r.st {
            if !st.starts_with("uuid:") {
                services.insert(st.clone());
            }
        }
    }
    for m in &ev.mdns {
        services.insert(m.service_type.clone());
    }
    Some(DiscoveredDevice {
        id,
        name,
        kind: c.kind,
        manufacturer,
        model,
        address: ev.ip.to_string(),
        port: c.port,
        discovered_via: via.into_iter().collect(),
        control_protocols: c.protocols,
        services: services.into_iter().collect(),
        last_seen: now_iso.to_string(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn ssdp(ip: &str, st: &str, server: &str) -> SsdpResponse {
        SsdpResponse { from: ip.parse().unwrap(), location: Some(format!("http://{ip}:1234/")), st: Some(st.into()), usn: Some(format!("uuid:{ip}::{st}")), server: Some(server.into()) }
    }
    fn desc(manu: &str, model: &str, name: &str, udn: &str) -> DeviceDescription {
        DeviceDescription { friendly_name: Some(name.into()), manufacturer: Some(manu.into()), model_name: Some(model.into()), model_number: None, udn: Some(udn.into()), device_type: None, service_types: vec![] }
    }
    fn mdns(ty: &str, port: u16, txt: &[(&str, &str)]) -> MdnsRecord {
        MdnsRecord { service_type: ty.into(), instance: "inst".into(), port, txt: txt.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn lg_webos() {
        let mut ev = Evidence::new("192.168.1.50".parse().unwrap());
        ev.ssdp.push(ssdp("192.168.1.50", "urn:lge-com:service:webos-second-screen:1", "WebOS/4.1.0 UPnP/1.0"));
        ev.ssdp.push(ssdp("192.168.1.50", "urn:dial-multiscreen-org:service:dial:1", "Linux/4.4 UPnP/1.0"));
        ev.descriptions.push(desc("LG Electronics", "OLED55C1PUB", "[LG] webOS TV OLED55C1PUB", "uuid:LG-1"));
        let d = to_device(&ev, "t").unwrap();
        assert_eq!(d.kind, DeviceKind::LgWebosTv);
        assert_eq!(d.control_protocols, vec![ControlProtocol::LgSsap, ControlProtocol::Dial]);
        assert_eq!(d.id, "uuid:lg-1");
        assert_eq!(d.port, Some(3001));
        assert_eq!(d.model.as_deref(), Some("OLED55C1PUB"));
        assert_eq!(d.discovered_via, vec![DiscoveredVia::Ssdp]);
    }

    #[test]
    fn samsung_tizen() {
        let mut ev = Evidence::new("10.0.0.9".parse().unwrap());
        ev.ssdp.push(ssdp("10.0.0.9", "urn:samsung.com:device:RemoteControlReceiver:1", "SHP, UPnP/1.0, Samsung UPnP SDK/1.0"));
        ev.ssdp.push(ssdp("10.0.0.9", "urn:schemas-upnp-org:device:MediaRenderer:1", "SHP, UPnP/1.0, Samsung UPnP SDK/1.0"));
        ev.descriptions.push(desc("Samsung Electronics", "QE55Q80TATXXU", "[TV] Samsung Q80 Series (55)", "uuid:S-1"));
        let d = to_device(&ev, "t").unwrap();
        assert_eq!(d.kind, DeviceKind::SamsungTizenTv);
        assert_eq!(d.control_protocols, vec![ControlProtocol::SamsungRemoteWs, ControlProtocol::UpnpMediaRenderer]);
        assert_eq!(d.port, Some(8002));
    }

    #[test]
    fn samsung_soundbar_is_not_a_tv() {
        let mut ev = Evidence::new("10.0.0.10".parse().unwrap());
        ev.ssdp.push(ssdp("10.0.0.10", "urn:schemas-upnp-org:device:MediaRenderer:1", "Samsung UPnP SDK/1.0"));
        ev.descriptions.push(desc("Samsung Electronics", "HW-Q90R", "Soundbar", "uuid:SB"));
        let d = to_device(&ev, "t").unwrap();
        assert_eq!(d.kind, DeviceKind::Unknown);
        assert_eq!(d.control_protocols, vec![ControlProtocol::UpnpMediaRenderer]);
    }

    #[test]
    fn android_and_google_tv() {
        let mut ev = Evidence::new("10.0.0.20".parse().unwrap());
        ev.mdns.push(mdns("_androidtvremote2._tcp.local.", 6466, &[("bt", "AA:BB")]));
        ev.mdns.push(mdns("_googlecast._tcp.local.", 8009, &[("md", "BRAVIA 4K VH2"), ("fn", "Living Room TV"), ("id", "abc"), ("ca", "463365")]));
        let d = to_device(&ev, "t").unwrap();
        assert_eq!(d.kind, DeviceKind::AndroidTv);
        assert_eq!(d.name, "Living Room TV");
        assert_eq!(d.model.as_deref(), Some("BRAVIA 4K VH2"));
        assert_eq!(d.id, "cast:abc");
        assert_eq!(d.port, Some(6466));
        assert_eq!(d.control_protocols, vec![ControlProtocol::AndroidTvRemote]);

        let mut ev = Evidence::new("10.0.0.21".parse().unwrap());
        ev.mdns.push(mdns("_androidtvremote2._tcp.local.", 6466, &[]));
        ev.mdns.push(mdns("_googlecast._tcp.local.", 8009, &[("md", "Chromecast"), ("fn", "Bedroom")]));
        assert_eq!(to_device(&ev, "t").unwrap().kind, DeviceKind::GoogleTv);
    }

    #[test]
    fn captureone_hub_and_others() {
        let mut ev = Evidence::new("10.0.0.30".parse().unwrap());
        ev.mdns.push(mdns("_captureone._tcp.local.", 47800, &[("id", "hub-1"), ("name", "Studio PC")]));
        let d = to_device(&ev, "t").unwrap();
        assert_eq!((d.kind, d.id.as_str(), d.name.as_str(), d.port), (DeviceKind::Desktop, "captureone:hub-1", "Studio PC", Some(47800)));
        assert_eq!(d.control_protocols, vec![ControlProtocol::CaptureoneCompanion]);

        // Roku-like DIAL server.
        let mut ev = Evidence::new("10.0.0.31".parse().unwrap());
        ev.ssdp.push(ssdp("10.0.0.31", "urn:dial-multiscreen-org:service:dial:1", "Roku/9.4 UPnP/1.0"));
        assert_eq!(to_device(&ev, "t").unwrap().kind, DeviceKind::OtherTv);

        // Audio-only cast speaker: not a TV and not controllable -> dropped.
        let mut ev = Evidence::new("10.0.0.32".parse().unwrap());
        ev.mdns.push(mdns("_googlecast._tcp.local.", 8009, &[("md", "Google Home Mini"), ("ca", "2052")]));
        assert!(to_device(&ev, "t").is_none());

        // Router.
        let mut ev = Evidence::new("10.0.0.1".parse().unwrap());
        ev.ssdp.push(ssdp("10.0.0.1", "urn:schemas-upnp-org:device:InternetGatewayDevice:1", "Linux UPnP/1.0 miniupnpd/2.0"));
        assert!(to_device(&ev, "t").is_none());
    }
}
