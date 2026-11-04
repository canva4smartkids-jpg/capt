//! Best-effort DNS-SD advertisement of the hub as `_captureone._tcp`.
//! Failure (no multicast, sandboxed network, firewall) is logged and never fails `start`.

use crate::protocol::{HubInfo, COMPANION_MDNS_SERVICE, COMPANION_PROTOCOL_VERSION};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::IpAddr;

pub(crate) struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertisement {
    pub fn stop(self) {
        if let Ok(rx) = self.daemon.unregister(&self.fullname) {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
        }
        if let Ok(rx) = self.daemon.shutdown() {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
        }
    }
}

/// DNS label safe instance name (max 63 bytes).
fn instance_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || c == '.' { ' ' } else { c }).collect();
    let mut s = cleaned.trim().to_string();
    if s.is_empty() {
        s = "CaptureOne".into();
    }
    while s.len() > 63 {
        s.pop();
    }
    s
}

fn host_label(hub_id: &str) -> String {
    let id: String = hub_id.chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect();
    format!("captureone-{}.local.", if id.is_empty() { "hub".to_string() } else { id.to_lowercase() })
}

pub(crate) fn advertise(info: &HubInfo, bind: IpAddr, port: u16) -> Option<Advertisement> {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("companion hub: mDNS unavailable ({e}); companions must enter the address manually");
            return None;
        }
    };
    let pv = COMPANION_PROTOCOL_VERSION.to_string();
    let props = [("id", info.hub_id.as_str()), ("pv", pv.as_str()), ("api", "/api/v1"), ("name", info.name.as_str())];
    let built = if bind.is_unspecified() {
        ServiceInfo::new(COMPANION_MDNS_SERVICE, &instance_name(&info.name), &host_label(&info.hub_id), "", port, &props[..]).map(|s| s.enable_addr_auto())
    } else {
        ServiceInfo::new(COMPANION_MDNS_SERVICE, &instance_name(&info.name), &host_label(&info.hub_id), bind, port, &props[..])
    };
    let service = match built {
        Ok(s) => s,
        Err(e) => {
            log::warn!("companion hub: invalid mDNS service info: {e}");
            let _ = daemon.shutdown();
            return None;
        }
    };
    let fullname = service.get_fullname().to_string();
    if let Err(e) = daemon.register(service) {
        log::warn!("companion hub: mDNS registration failed: {e}");
        let _ = daemon.shutdown();
        return None;
    }
    Some(Advertisement { daemon, fullname })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        assert_eq!(instance_name("  Studio.PC\n "), "Studio PC");
        assert_eq!(instance_name(""), "CaptureOne");
        assert_eq!(instance_name(&"x".repeat(100)).len(), 63);
        assert_eq!(host_label("AB-12-cd"), "captureone-ab12cd.local.");
    }
}
