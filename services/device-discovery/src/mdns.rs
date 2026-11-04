//! mDNS / DNS-SD browsing (`mdns-sd`).

use crate::classify::MdnsRecord;
use crate::transport::net_err;
use capture_core::CaptureResult;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub const ANDROID_TV_REMOTE: &str = "_androidtvremote2._tcp.local.";
pub const GOOGLE_CAST: &str = "_googlecast._tcp.local.";
pub const CAPTUREONE: &str = "_captureone._tcp.local.";
pub const DEFAULT_SERVICES: &[&str] = &[ANDROID_TV_REMOTE, GOOGLE_CAST, CAPTUREONE];

/// Browses `services` for `timeout` and returns every resolved instance with its IPv4 addresses.
pub fn browse(services: &[&str], timeout: Duration) -> CaptureResult<Vec<(IpAddr, MdnsRecord)>> {
    let daemon = ServiceDaemon::new().map_err(|e| net_err(format!("mDNS unavailable: {e}")))?;
    let mut receivers = Vec::new();
    for s in services {
        match daemon.browse(s) {
            Ok(rx) => receivers.push(rx),
            Err(e) => log::debug!("mdns: browse {s} failed: {e}"),
        }
    }
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    while Instant::now() < deadline && !receivers.is_empty() {
        let mut idle = true;
        for rx in &receivers {
            while let Ok(ev) = rx.try_recv() {
                idle = false;
                if let ServiceEvent::ServiceResolved(info) = ev {
                    let txt: BTreeMap<String, String> = info.get_properties().iter().map(|p| (p.key().to_string(), p.val_str().to_string())).collect();
                    let instance = info.get_fullname().strip_suffix(&format!(".{}", info.ty_domain)).unwrap_or(info.get_fullname()).to_string();
                    let record = MdnsRecord { service_type: info.ty_domain.clone(), instance, port: info.get_port(), txt };
                    for ip in info.get_addresses_v4() {
                        out.push((IpAddr::V4(ip), record.clone()));
                    }
                }
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    for s in services {
        let _ = daemon.stop_browse(s);
    }
    if let Ok(rx) = daemon.shutdown() {
        let _ = rx.recv_timeout(Duration::from_millis(500));
    }
    out.sort_by(|a, b| (a.0, &a.1.service_type, &a.1.instance).cmp(&(b.0, &b.1.service_type, &b.1.instance)));
    out.dedup();
    Ok(out)
}
