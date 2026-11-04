//! Discovery against fake SSDP responders (unicast on 127.0.0.x) and fake description servers.
//! Linux routes all of 127.0.0.0/8 to loopback, so each fake "TV" gets its own address.

mod common;

use device_discovery::*;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct FakeTv {
    addr: SocketAddr,
    searches: Arc<Mutex<Vec<String>>>,
}

/// Answers M-SEARCH like a TV: one response per matching ST (all of them for `ssdp:all`).
fn fake_ssdp(ip: &str, sts: Vec<String>, location: String, server: &'static str, uuid: &'static str) -> FakeTv {
    let sock = UdpSocket::bind(format!("{ip}:0")).unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    let addr = sock.local_addr().unwrap();
    let searches = Arc::new(Mutex::new(Vec::new()));
    let log = searches.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
            let msg = String::from_utf8_lossy(&buf[..n]).to_string();
            log.lock().unwrap().push(msg.clone());
            if !msg.starts_with("M-SEARCH * HTTP/1.1\r\n") || !msg.contains("MAN: \"ssdp:discover\"") {
                continue;
            }
            let st = msg.lines().find_map(|l| l.strip_prefix("ST: ")).unwrap_or("").trim().to_string();
            for s in &sts {
                if st == "ssdp:all" || st == *s {
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nEXT:\r\nLOCATION: {location}\r\nSERVER: {server}\r\nST: {s}\r\nUSN: uuid:{uuid}::{s}\r\n\r\n"
                    );
                    let _ = sock.send_to(resp.as_bytes(), from);
                }
            }
        }
    });
    FakeTv { addr, searches }
}

const LG_XML: &str = r#"<?xml version="1.0"?><root xmlns="urn:schemas-upnp-org:device-1-0"><device>
<deviceType>urn:schemas-upnp-org:device:Basic:1</deviceType><friendlyName>[LG] webOS TV OLED55C1PUB</friendlyName>
<manufacturer>LG Electronics</manufacturer><modelName>OLED55C1PUB</modelName><modelNumber>55C1</modelNumber>
<UDN>uuid:lg-0001</UDN><serviceList><service><serviceType>urn:lge-com:service:webos-second-screen:1</serviceType></service></serviceList>
</device></root>"#;

const SAMSUNG_XML: &str = r#"<?xml version="1.0"?><root xmlns="urn:schemas-upnp-org:device-1-0"><device>
<deviceType>urn:samsung.com:device:RemoteControlReceiver:1</deviceType><friendlyName>[TV] Samsung Q80 Series (55)</friendlyName>
<manufacturer>Samsung Electronics</manufacturer><modelName>QE55Q80TATXXU</modelName><UDN>uuid:sam-0001</UDN>
</device></root>"#;

const ROUTER_XML: &str = r#"<?xml version="1.0"?><root xmlns="urn:schemas-upnp-org:device-1-0"><device>
<deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType><friendlyName>Router</friendlyName>
<manufacturer>Acme</manufacturer><modelName>R1</modelName><UDN>uuid:router</UDN></device></root>"#;

#[test]
fn scan_classifies_fake_tvs() {
    let lg_http = common::fake_http("127.0.0.2:0", vec![("/desc.xml".into(), "text/xml", LG_XML.as_bytes().to_vec())]);
    let lg = fake_ssdp(
        "127.0.0.2",
        vec!["urn:lge-com:service:webos-second-screen:1".into(), "urn:dial-multiscreen-org:service:dial:1".into()],
        format!("http://{}/desc.xml", lg_http.addr),
        "WebOS/4.1.0 UPnP/1.0",
        "lg-0001",
    );
    let sam_http = common::fake_http("127.0.0.3:0", vec![("/rcr/".into(), "text/xml", SAMSUNG_XML.as_bytes().to_vec())]);
    let sam = fake_ssdp(
        "127.0.0.3",
        vec!["urn:samsung.com:device:RemoteControlReceiver:1".into(), "upnp:rootdevice".into()],
        format!("http://{}/rcr/", sam_http.addr),
        "SHP, UPnP/1.0, Samsung UPnP SDK/1.0",
        "sam-0001",
    );
    let router_http = common::fake_http("127.0.0.4:0", vec![("/igd.xml".into(), "text/xml", ROUTER_XML.as_bytes().to_vec())]);
    let router = fake_ssdp(
        "127.0.0.4",
        vec!["urn:schemas-upnp-org:device:InternetGatewayDevice:1".into()],
        format!("http://{}/igd.xml", router_http.addr),
        "Linux UPnP/1.0 miniupnpd/2.0",
        "router",
    );
    // A responder whose LOCATION points at another host must not make us fetch it.
    let sneaky = fake_ssdp(
        "127.0.0.5",
        vec!["urn:dial-multiscreen-org:service:dial:1".into()],
        format!("http://{}/desc.xml", lg_http.addr),
        "Roku/9.4 UPnP/1.0",
        "roku-1",
    );

    let opts = ScanOptions {
        timeout: Duration::from_millis(1200),
        ssdp_targets: vec![lg.addr, sam.addr, router.addr, sneaky.addr],
        mdns_services: vec![],
        ..Default::default()
    };
    let devices = scan_with(&opts).unwrap();

    // The exact searches we send.
    let searches = lg.searches.lock().unwrap().clone();
    for st in ["ssdp:all", "urn:dial-multiscreen-org:service:dial:1", "urn:samsung.com:device:RemoteControlReceiver:1", "urn:lge-com:service:webos-second-screen:1"] {
        assert!(searches.iter().any(|m| m.contains(&format!("\r\nST: {st}\r\n"))), "missing ST {st}");
    }
    assert!(searches.iter().all(|m| m.contains("\r\nMX: 1\r\n")));

    for d in &devices {
        common::assert_valid("DiscoveredDevice", &serde_json::to_value(d).unwrap());
    }
    let find = |addr: &str| devices.iter().find(|d| d.address == addr);
    let lg_dev = find("127.0.0.2").expect("LG found");
    assert_eq!(lg_dev.kind, DeviceKind::LgWebosTv);
    assert_eq!(lg_dev.name, "[LG] webOS TV OLED55C1PUB");
    assert_eq!(lg_dev.manufacturer.as_deref(), Some("LG Electronics"));
    assert_eq!(lg_dev.model.as_deref(), Some("OLED55C1PUB"));
    assert_eq!(lg_dev.id, "uuid:lg-0001");
    assert_eq!(lg_dev.control_protocols, vec![ControlProtocol::LgSsap, ControlProtocol::Dial]);
    assert_eq!(lg_dev.discovered_via, vec![DiscoveredVia::Ssdp]);
    assert!(lg_dev.services.contains(&"urn:lge-com:service:webos-second-screen:1".to_string()));

    let sam_dev = find("127.0.0.3").expect("Samsung found");
    assert_eq!(sam_dev.kind, DeviceKind::SamsungTizenTv);
    assert_eq!(sam_dev.model.as_deref(), Some("QE55Q80TATXXU"));
    assert_eq!(sam_dev.control_protocols, vec![ControlProtocol::SamsungRemoteWs]);
    assert_eq!(sam_dev.port, Some(8002));

    assert!(find("127.0.0.4").is_none(), "routers are filtered out");

    let roku = find("127.0.0.5").expect("DIAL device found");
    assert_eq!(roku.kind, DeviceKind::OtherTv);
    assert_eq!(roku.manufacturer, None, "description on a foreign host was not fetched");
    assert_eq!(lg_http.requests.lock().unwrap().iter().filter(|p| *p == "/desc.xml").count(), 1, "only the LG responder's own LOCATION was fetched");
}

#[test]
fn scan_with_nothing_answering_is_empty_not_error() {
    let silent = UdpSocket::bind("127.0.0.6:0").unwrap();
    let opts = ScanOptions { timeout: Duration::from_millis(300), ssdp_targets: vec![silent.local_addr().unwrap()], mdns_services: vec![], ..Default::default() };
    assert!(scan_with(&opts).unwrap().is_empty());
}
