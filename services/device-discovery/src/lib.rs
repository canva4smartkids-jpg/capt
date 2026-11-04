//! LAN discovery and control of TVs for the CaptureOne desktop app.
//!
//! * [`scan`]: SSDP M-SEARCH + UPnP description + mDNS browse, merged per IP into contract
//!   [`DiscoveredDevice`]s (`packages/capture-contracts/src/companion.ts`).
//! * [`lg::LgClient`]: LG webOS SSAP (pairing, system info, remote keys and an **experimental,
//!   undocumented** `executeOneShot` screenshot of the TV picture — `partial`: not an official LG
//!   API, model/firmware dependent, DRM content blank).
//! * [`samsung::SamsungClient`]: Samsung Tizen remote-control WebSocket (keys only).
//! * [`TvClient`]: one entry point for the desktop's `tv_pair` / `tv_send_key` / `tv_screenshot`.
//!
//! No real TV was available while this was written; all protocol code is tested against fake
//! SSDP/HTTP/WebSocket servers that check the exact bytes sent.

pub mod classify;
pub mod description;
pub mod keys;
pub mod lg;
pub mod mdns;
pub mod media;
pub mod samsung;
pub mod ssdp;
pub mod store;
pub mod transport;

pub use companion_hub::protocol::{ControlProtocol, DeviceKind, DiscoveredDevice, DiscoveredVia};
pub use keys::TvRemoteKey;
pub use store::CredentialStore;

use capture_core::*;
use classify::Evidence;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// SSDP/mDNS listening time. Description fetches add up to `description_timeout`.
    pub timeout: Duration,
    /// Where M-SEARCH is sent (the SSDP multicast group; tests use unicast fakes).
    pub ssdp_targets: Vec<SocketAddr>,
    pub search_targets: Vec<String>,
    /// mDNS service types to browse; empty disables mDNS.
    pub mdns_services: Vec<String>,
    pub fetch_descriptions: bool,
    pub description_timeout: Duration,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3),
            ssdp_targets: vec![ssdp::SSDP_MULTICAST],
            search_targets: ssdp::DEFAULT_SEARCH_TARGETS.iter().map(|s| s.to_string()).collect(),
            mdns_services: mdns::DEFAULT_SERVICES.iter().map(|s| s.to_string()).collect(),
            fetch_descriptions: true,
            description_timeout: Duration::from_secs(2),
        }
    }
}

/// Scans the LAN for TVs and CaptureOne hubs for `timeout` (plus up to 2 s of description fetches).
pub fn scan(timeout: Duration) -> CaptureResult<Vec<DiscoveredDevice>> {
    scan_with(&ScanOptions { timeout, ..Default::default() })
}

pub fn scan_with(opts: &ScanOptions) -> CaptureResult<Vec<DiscoveredDevice>> {
    let sts: Vec<&str> = opts.search_targets.iter().map(String::as_str).collect();
    let services: Vec<&str> = opts.mdns_services.iter().map(String::as_str).collect();
    let (ssdp_res, mdns_res) = std::thread::scope(|s| {
        let ssdp_h = s.spawn(|| if opts.ssdp_targets.is_empty() { Ok(vec![]) } else { ssdp::search(&opts.ssdp_targets, &sts, opts.timeout) });
        let mdns_h = s.spawn(|| if services.is_empty() { Ok(vec![]) } else { mdns::browse(&services, opts.timeout) });
        (
            ssdp_h.join().unwrap_or_else(|_| Err(CaptureError::internal("ssdp thread panicked"))),
            mdns_h.join().unwrap_or_else(|_| Err(CaptureError::internal("mdns thread panicked"))),
        )
    });
    let (responses, records) = match (ssdp_res, mdns_res) {
        (Err(a), Err(b)) => return Err(CaptureError::new(CaptureErrorCode::Network, format!("discovery failed: SSDP: {}; mDNS: {}", a.message, b.message))),
        (a, b) => {
            if let Err(e) = &a {
                log::warn!("SSDP discovery failed: {}", e.message);
            }
            if let Err(e) = &b {
                log::warn!("mDNS discovery failed: {}", e.message);
            }
            (a.unwrap_or_default(), b.unwrap_or_default())
        }
    };

    let mut by_ip: HashMap<IpAddr, Evidence> = HashMap::new();
    for r in responses {
        by_ip.entry(r.from).or_insert_with(|| Evidence::new(r.from)).ssdp.push(r);
    }
    for (ip, rec) in records {
        by_ip.entry(ip).or_insert_with(|| Evidence::new(ip)).mdns.push(rec);
    }

    if opts.fetch_descriptions {
        // One fetch per distinct (LOCATION, responder) pair, in parallel. The responder is part
        // of the key because a LOCATION is only fetched from the device that announced it.
        let mut locations: BTreeSet<(String, IpAddr)> = BTreeSet::new();
        for ev in by_ip.values() {
            for r in &ev.ssdp {
                if let Some(l) = &r.location {
                    locations.insert((l.clone(), r.from));
                }
            }
        }
        let fetched: Vec<(IpAddr, description::DeviceDescription)> = std::thread::scope(|s| {
            let handles: Vec<_> = locations
                .iter()
                .take(64)
                .map(|(loc, ip)| {
                    let (loc, ip) = (loc.clone(), *ip);
                    s.spawn(move || description::fetch(&loc, ip, opts.description_timeout).map(|d| (ip, d)).map_err(|e| log::debug!("description {loc}: {}", e.message)).ok())
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
        });
        for (ip, d) in fetched {
            if let Some(ev) = by_ip.get_mut(&ip) {
                if !ev.descriptions.contains(&d) {
                    ev.descriptions.push(d);
                }
            }
        }
    }

    let now = now_iso8601();
    let devices: Vec<DiscoveredDevice> = by_ip.values().filter_map(|ev| classify::to_device(ev, &now)).collect();
    Ok(dedupe_by_id(devices))
}

/// Preference for the address to keep when one device answers on several addresses
/// (multi-homed hosts, mDNS records that include loopback): LAN-private first, loopback last.
fn address_rank(addr: &str) -> (u8, String) {
    let rank = match addr.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) if v4.is_loopback() => 3,
        Ok(IpAddr::V6(v6)) if v6.is_loopback() => 3,
        Ok(IpAddr::V4(v4)) if v4.is_private() => 0,
        Ok(IpAddr::V4(v4)) if v4.is_link_local() => 2,
        Ok(_) => 1,
        Err(_) => 4,
    };
    (rank, addr.to_string())
}

fn union<T: PartialEq + Ord>(a: &mut Vec<T>, b: Vec<T>) {
    for x in b {
        if !a.contains(&x) {
            a.push(x);
        }
    }
    a.sort();
}

/// Device ids are unique in the result: one entry per id, merged (via/protocols/services) and
/// keeping the best address. Sorted by kind, name, address.
pub fn dedupe_by_id(devices: Vec<DiscoveredDevice>) -> Vec<DiscoveredDevice> {
    let mut by_id: BTreeMap<String, DiscoveredDevice> = BTreeMap::new();
    for d in devices {
        match by_id.get_mut(&d.id) {
            None => {
                by_id.insert(d.id.clone(), d);
            }
            Some(existing) => {
                let mut merged = if address_rank(&d.address) < address_rank(&existing.address) { d.clone() } else { existing.clone() };
                let other = if merged.address == d.address { existing.clone() } else { d };
                union(&mut merged.discovered_via, other.discovered_via);
                union(&mut merged.control_protocols, other.control_protocols);
                union(&mut merged.services, other.services);
                merged.manufacturer = merged.manufacturer.or(other.manufacturer);
                merged.model = merged.model.or(other.model);
                *existing = merged;
            }
        }
    }
    let mut out: Vec<DiscoveredDevice> = by_id.into_values().collect();
    out.sort_by(|a, b| (a.kind.as_str(), &a.name, &a.address).cmp(&(b.kind.as_str(), &b.name, &b.address)));
    out
}

/// A paired connection to a controllable TV.
pub enum TvClient {
    Lg(Box<lg::LgClient>),
    Samsung(Box<samsung::SamsungClient>),
}

impl TvClient {
    /// Connects (pairing on first use; the user must accept a prompt on the TV) using the best
    /// protocol the device advertises.
    pub fn connect(device: &DiscoveredDevice, store: Arc<CredentialStore>) -> CaptureResult<Self> {
        let ip: IpAddr = device.address.parse().map_err(|_| CaptureError::invalid_config(format!("invalid TV address {}", device.address)))?;
        if device.control_protocols.contains(&ControlProtocol::LgSsap) {
            return lg::LgClient::connect(ip, store, lg::LgClientConfig::default()).map(|c| TvClient::Lg(Box::new(c)));
        }
        if device.control_protocols.contains(&ControlProtocol::SamsungRemoteWs) {
            return samsung::SamsungClient::connect(ip, store, samsung::SamsungClientConfig::default()).map(|c| TvClient::Samsung(Box::new(c)));
        }
        if device.control_protocols.contains(&ControlProtocol::AndroidTvRemote) {
            return Err(CaptureError::unsupported(
                "Android TV / Google TV remote control (Android TV Remote protocol v2, TLS pairing with an on-screen code) is not implemented; use the CaptureOne Android TV app",
            ));
        }
        Err(CaptureError::unsupported(format!("{} cannot be controlled by CaptureOne", device.name)))
    }

    pub fn send_key(&mut self, key: TvRemoteKey) -> CaptureResult<()> {
        match self {
            TvClient::Lg(c) => c.send_key(key),
            TvClient::Samsung(c) => c.send_key(key),
        }
    }

    /// LG only (experimental, undocumented `ssap://tv/executeOneShot`); UNSUPPORTED on Samsung
    /// (Samsung's Smart TV FAQ: screen capture is not supported in Samsung TV apps).
    pub fn screenshot(&mut self, output_dir: &Path, tv_name: &str) -> CaptureResult<MediaMetadata> {
        match self {
            TvClient::Lg(c) => c.screenshot(output_dir, tv_name),
            TvClient::Samsung(c) => c.screenshot(),
        }
    }

    pub fn close(self) {
        match self {
            TvClient::Lg(c) => (*c).close(),
            TvClient::Samsung(c) => (*c).close(),
        }
    }
}

/// Honest support levels for the desktop app's capability report (TV-related features it gains
/// from this crate). `tvNativeScreenshot` is `partial` and experimental/undocumented: LG only, needs
/// on-TV pairing, firmware dependent, DRM content blank, and unverified on real hardware.
pub fn desktop_tv_features() -> BTreeMap<FeatureKey, FeatureSupport> {
    let mut m = BTreeMap::new();
    m.insert(
        FeatureKey::DeviceDiscovery,
        FeatureSupport::with(SupportLevel::Partial, "SSDP + mDNS on the local network; blocked by client isolation / multicast filtering on some Wi-Fi networks")
            .requires(vec![PermissionKind::LocalNetwork]),
    );
    m.insert(
        FeatureKey::RemoteControl,
        FeatureSupport::with(SupportLevel::Partial, "LG webOS (SSAP) and Samsung Tizen (remote WebSocket) after accepting a prompt on the TV; Android TV not implemented")
            .requires(vec![PermissionKind::LocalNetwork]),
    );
    m.insert(
        FeatureKey::TvNativeScreenshot,
        FeatureSupport::with(
            SupportLevel::Partial,
            "Experimental, undocumented: LG webOS only, via the undocumented ssap://tv/executeOneShot call (not an official LG API; LG's developer forum states webOS provides no screen-capture API) after accepting pairing on the TV; model/firmware dependent and may break with updates; protected (HDCP/DRM) content is blank; not verified on a real TV. Samsung: unsupported (Samsung's Smart TV FAQ states screen capture is not supported in Samsung TV apps)",
        )
        .requires(vec![PermissionKind::LocalNetwork]),
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, addr: &str, via: DiscoveredVia) -> DiscoveredDevice {
        DiscoveredDevice {
            id: id.into(),
            name: "Hub".into(),
            kind: DeviceKind::Desktop,
            manufacturer: None,
            model: None,
            address: addr.into(),
            port: Some(47800),
            discovered_via: vec![via],
            control_protocols: vec![ControlProtocol::CaptureoneCompanion],
            services: vec!["_captureone._tcp.local.".into()],
            last_seen: String::new(),
        }
    }

    #[test]
    fn dedupe_prefers_lan_address_and_merges() {
        let out = dedupe_by_id(vec![
            dev("captureone:a", "127.0.0.1", DiscoveredVia::Mdns),
            dev("captureone:a", "192.168.1.20", DiscoveredVia::Mdns),
            dev("captureone:a", "203.0.113.5", DiscoveredVia::Ssdp),
            dev("captureone:b", "10.0.0.2", DiscoveredVia::Mdns),
        ]);
        assert_eq!(out.len(), 2);
        let a = out.iter().find(|d| d.id == "captureone:a").unwrap();
        assert_eq!(a.address, "192.168.1.20");
        assert_eq!(a.discovered_via, vec![DiscoveredVia::Ssdp, DiscoveredVia::Mdns]);
    }
}
