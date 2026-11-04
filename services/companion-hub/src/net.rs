//! LAN address helpers.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// The IPv4 address of the interface that routes to the internet / default gateway.
/// Uses a connected UDP socket, which selects a route without sending any packet.
/// Falls back to the first private interface address.
pub fn primary_lan_ip() -> Option<IpAddr> {
    let routed = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok().and_then(|s| {
        s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?; // TEST-NET-1, never actually contacted
        s.local_addr().ok().map(|a| a.ip())
    });
    match routed {
        Some(ip) if !ip.is_unspecified() && !ip.is_loopback() => Some(ip),
        _ => lan_ips().into_iter().next(),
    }
}

/// Non-loopback, non-link-local IPv4 interface addresses, private ranges first.
pub fn lan_ips() -> Vec<IpAddr> {
    let mut v: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .map(|list| {
            list.into_iter()
                .filter_map(|i| match i.ip() {
                    IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() => Some(ip),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|ip| (!ip.is_private(), *ip));
    v.dedup();
    v.into_iter().map(IpAddr::V4).collect()
}

pub fn http_url(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(v4) => format!("http://{v4}:{port}"),
        IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls() {
        assert_eq!(http_url(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), 47800), "http://192.168.1.2:47800");
        assert_eq!(http_url("::1".parse().unwrap(), 1), "http://[::1]:1");
        for ip in lan_ips() {
            assert!(!ip.is_loopback());
        }
    }
}
