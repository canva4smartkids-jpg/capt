//! SSDP (UPnP discovery): M-SEARCH on 239.255.255.250:1900 and response parsing.

use crate::transport::net_err;
use capture_core::CaptureResult;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

pub const SSDP_MULTICAST: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900));

pub const ST_ALL: &str = "ssdp:all";
pub const ST_DIAL: &str = "urn:dial-multiscreen-org:service:dial:1";
pub const ST_SAMSUNG_REMOTE: &str = "urn:samsung.com:device:RemoteControlReceiver:1";
pub const ST_LG_SECOND_SCREEN: &str = "urn:lge-com:service:webos-second-screen:1";
pub const DEFAULT_SEARCH_TARGETS: &[&str] = &[ST_ALL, ST_DIAL, ST_SAMSUNG_REMOTE, ST_LG_SECOND_SCREEN];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsdpResponse {
    /// Address the response came from (the device).
    pub from: IpAddr,
    pub location: Option<String>,
    pub st: Option<String>,
    pub usn: Option<String>,
    pub server: Option<String>,
}

pub fn m_search(st: &str, mx: u32, host: SocketAddr) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: {host}\r\nMAN: \"ssdp:discover\"\r\nMX: {mx}\r\nST: {st}\r\nUSER-AGENT: CaptureOne/{} UPnP/1.1\r\n\r\n",
        capture_core::APP_VERSION
    )
}

/// Parses a unicast search response (`HTTP/1.1 200 OK`) or a multicast `NOTIFY` (alive).
pub fn parse_response(data: &[u8], from: IpAddr) -> Option<SsdpResponse> {
    let text = std::str::from_utf8(data).ok()?;
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let first = lines.next()?.trim().to_ascii_uppercase();
    let is_response = first.starts_with("HTTP/1.1 200") || first.starts_with("HTTP/1.0 200");
    let is_notify = first.starts_with("NOTIFY ");
    if !is_response && !is_notify {
        return None;
    }
    let mut r = SsdpResponse { from, location: None, st: None, usn: None, server: None };
    let mut nts_alive = !is_notify;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim().to_string();
        match k.trim().to_ascii_uppercase().as_str() {
            "LOCATION" => r.location = Some(v),
            "ST" | "NT" => r.st = Some(v),
            "USN" => r.usn = Some(v),
            "SERVER" => r.server = Some(v),
            "NTS" => nts_alive = v.eq_ignore_ascii_case("ssdp:alive"),
            _ => {}
        }
    }
    (nts_alive && (r.st.is_some() || r.usn.is_some())).then_some(r)
}

fn multicast_socket(iface: Option<Ipv4Addr>) -> std::io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    let bind_ip = iface.unwrap_or(Ipv4Addr::UNSPECIFIED);
    s.bind(&SocketAddr::V4(SocketAddrV4::new(bind_ip, 0)).into())?;
    if let Some(ip) = iface {
        s.set_multicast_if_v4(&ip)?;
    }
    // TTL 2 reaches TVs behind one extra hop (some mesh/AP setups) without flooding further.
    s.set_multicast_ttl_v4(2)?;
    s.set_multicast_loop_v4(true)?;
    Ok(s.into())
}

fn lan_ipv4s() -> Vec<Ipv4Addr> {
    let mut v: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .map(|l| {
            l.into_iter()
                .filter_map(|i| match i.ip() {
                    IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_link_local() => Some(ip),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.dedup();
    v
}

/// Sends M-SEARCH for every `st` to every `target` (the multicast group in production, unicast
/// fake responders in tests) and collects responses until `timeout`. Searches are repeated once
/// after 300 ms because UDP is lossy.
pub fn search(targets: &[SocketAddr], sts: &[&str], timeout: Duration) -> CaptureResult<Vec<SsdpResponse>> {
    let multicast = targets.iter().any(|t| t.ip().is_multicast());
    let mut sockets: Vec<UdpSocket> = Vec::new();
    if multicast {
        for ip in lan_ipv4s() {
            match multicast_socket(Some(ip)) {
                Ok(s) => sockets.push(s),
                Err(e) => log::debug!("ssdp: cannot use interface {ip}: {e}"),
            }
        }
    }
    if sockets.is_empty() {
        sockets.push(multicast_socket(None).map_err(|e| net_err(format!("SSDP socket: {e}")))?);
    }
    for s in &sockets {
        s.set_read_timeout(Some(Duration::from_millis(40))).map_err(|e| net_err(e.to_string()))?;
    }
    let mx = timeout.as_secs().clamp(1, 3) as u32;
    let send_all = |sockets: &[UdpSocket]| -> usize {
        let mut sent = 0;
        for s in sockets {
            for t in targets {
                for st in sts {
                    match s.send_to(m_search(st, mx, *t).as_bytes(), t) {
                        Ok(_) => sent += 1,
                        Err(e) => log::debug!("ssdp: send to {t} failed: {e}"),
                    }
                }
            }
        }
        sent
    };
    if send_all(&sockets) == 0 {
        return Err(net_err("SSDP: could not send M-SEARCH on any interface (no network?)"));
    }
    let start = Instant::now();
    let deadline = start + timeout;
    let mut resent = false;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    while Instant::now() < deadline {
        if !resent && start.elapsed() >= Duration::from_millis(300) {
            send_all(&sockets);
            resent = true;
        }
        for s in &sockets {
            match s.recv_from(&mut buf) {
                Ok((n, from)) => {
                    if let Some(r) = parse_response(&buf[..n], from.ip()) {
                        let key = (r.from, r.usn.clone(), r.st.clone(), r.location.clone());
                        if seen.insert(key) {
                            out.push(r);
                        }
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) => log::debug!("ssdp: recv failed: {e}"),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m_search_format() {
        let m = m_search(ST_DIAL, 2, SSDP_MULTICAST);
        assert!(m.starts_with("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:dial-multiscreen-org:service:dial:1\r\n"));
        assert!(m.ends_with("\r\n\r\n"));
    }

    #[test]
    fn parses_lg_response() {
        let raw = b"HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nDATE: Tue, 04 Nov 2026 10:00:00 GMT\r\nEXT:\r\nLOCATION: http://192.168.1.50:1691/\r\nSERVER: WebOS/4.1.0 UPnP/1.0\r\nST: urn:lge-com:service:webos-second-screen:1\r\nUSN: uuid:0a7a1e0d-xxxx::urn:lge-com:service:webos-second-screen:1\r\n\r\n";
        let r = parse_response(raw, "192.168.1.50".parse().unwrap()).unwrap();
        assert_eq!(r.location.as_deref(), Some("http://192.168.1.50:1691/"));
        assert_eq!(r.st.as_deref(), Some(ST_LG_SECOND_SCREEN));
        assert_eq!(r.server.as_deref(), Some("WebOS/4.1.0 UPnP/1.0"));
    }

    #[test]
    fn parses_notify_and_rejects_byebye_and_searches() {
        let alive = b"NOTIFY * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nNT: urn:samsung.com:device:RemoteControlReceiver:1\r\nNTS: ssdp:alive\r\nLOCATION: http://10.0.0.9:7676/rcr/\r\nUSN: uuid:abc::urn:samsung.com:device:RemoteControlReceiver:1\r\n\r\n";
        let r = parse_response(alive, "10.0.0.9".parse().unwrap()).unwrap();
        assert_eq!(r.st.as_deref(), Some(ST_SAMSUNG_REMOTE));
        let byebye = b"NOTIFY * HTTP/1.1\r\nNT: upnp:rootdevice\r\nNTS: ssdp:byebye\r\nUSN: uuid:abc\r\n\r\n";
        assert!(parse_response(byebye, "10.0.0.9".parse().unwrap()).is_none());
        let search = m_search(ST_ALL, 1, SSDP_MULTICAST);
        assert!(parse_response(search.as_bytes(), "10.0.0.9".parse().unwrap()).is_none());
        assert!(parse_response(&[0xff, 0xfe], "10.0.0.9".parse().unwrap()).is_none());
    }
}
