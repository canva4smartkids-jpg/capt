//! Samsung remote client against fake TVs: plain ws (8001 style) and TLS with a self-signed
//! certificate (8002 style), checking the exact URL and messages and certificate pinning.
#![allow(clippy::result_large_err)] // tungstenite handshake callback signature

use capture_core::CaptureErrorCode;
use device_discovery::samsung::{SamsungClient, SamsungClientConfig};
use device_discovery::store::Vendor;
use device_discovery::*;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::handshake::server::{Request, Response};
use tungstenite::Message;

#[derive(Default)]
struct Seen {
    urls: Vec<String>,
    messages: Vec<Value>,
}

#[derive(Clone)]
enum Reply {
    Connect(Option<&'static str>),
    Unauthorized,
}

fn serve<S: Read + Write>(stream: S, seen: Arc<Mutex<Seen>>, reply: Reply) {
    let s2 = seen.clone();
    let Ok(mut ws) = tungstenite::accept_hdr(stream, move |req: &Request, resp: Response| {
        s2.lock().unwrap().urls.push(req.uri().to_string());
        Ok(resp)
    }) else {
        return;
    };
    let ev = match reply {
        Reply::Connect(token) => {
            let mut data = json!({"clients":[], "id":"abc"});
            if let Some(t) = token {
                data["token"] = json!(t);
            }
            json!({"event":"ms.channel.connect","data":data})
        }
        Reply::Unauthorized => json!({"event":"ms.channel.unauthorized"}),
    };
    // Something unrelated first; the client must wait for the channel event.
    let _ = ws.send(Message::text(json!({"event":"ed.edenTV.update","data":{}}).to_string()));
    let _ = ws.send(Message::text(ev.to_string()));
    while let Ok(m) = ws.read() {
        if let Message::Text(t) = m {
            seen.lock().unwrap().messages.push(serde_json::from_str(t.as_str()).unwrap());
        }
    }
}

fn plain_server(reply: Reply) -> (u16, Arc<Mutex<Seen>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s = seen.clone();
    std::thread::spawn(move || {
        for st in l.incoming().flatten() {
            let (s, r) = (s.clone(), reply.clone());
            std::thread::spawn(move || serve(st, s, r));
        }
    });
    (port, seen)
}

fn tls_server(reply: Reply) -> (u16, Arc<Mutex<Seen>>) {
    let ck = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert = CertificateDer::from(ck.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let cfg = Arc::new(cfg);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s = seen.clone();
    std::thread::spawn(move || {
        for st in l.incoming().flatten() {
            let (s, r, cfg) = (s.clone(), reply.clone(), cfg.clone());
            std::thread::spawn(move || {
                let Ok(conn) = rustls::ServerConnection::new(cfg) else { return };
                let tls: rustls::StreamOwned<rustls::ServerConnection, TcpStream> = rustls::StreamOwned::new(conn, st);
                serve(tls, s, r);
            });
        }
    });
    (port, seen)
}

const LOCAL: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

fn cfg(tls: bool, port: u16) -> SamsungClientConfig {
    SamsungClientConfig { endpoints: vec![(tls, port)], pairing_timeout: Duration::from_secs(5), ..Default::default() }
}

fn wait_messages(seen: &Arc<Mutex<Seen>>, n: usize) -> Vec<Value> {
    for _ in 0..50 {
        let m = seen.lock().unwrap().messages.clone();
        if m.len() >= n {
            return m;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    seen.lock().unwrap().messages.clone()
}

#[test]
fn plain_ws_keys() {
    let (port, seen) = plain_server(Reply::Connect(None));
    let mut c = SamsungClient::connect(LOCAL, CredentialStore::in_memory(), cfg(false, port)).unwrap();
    c.send_key(TvRemoteKey::Up).unwrap();
    c.send_key(TvRemoteKey::Back).unwrap();
    c.send_key(TvRemoteKey::VolumeDown).unwrap();
    let msgs = wait_messages(&seen, 3);
    assert_eq!(seen.lock().unwrap().urls[0], "/api/v2/channels/samsung.remote.control?name=Q2FwdHVyZU9uZQ%3D%3D");
    let keys: Vec<&str> = msgs.iter().map(|m| m["params"]["DataOfCmd"].as_str().unwrap()).collect();
    assert_eq!(keys, vec!["KEY_UP", "KEY_RETURN", "KEY_VOLDOWN"]);
    assert_eq!(msgs[0], json!({"method":"ms.remote.control","params":{"Cmd":"Click","DataOfCmd":"KEY_UP","Option":"false","TypeOfRemote":"SendRemoteKey"}}));
    let err = c.screenshot().unwrap_err();
    assert_eq!(err.code, CaptureErrorCode::Unsupported);
    c.close();
}

#[test]
fn tls_token_and_certificate_pinning() {
    let (port, seen) = tls_server(Reply::Connect(Some("TOKEN42")));
    let dir = tempfile::tempdir().unwrap();
    let store = CredentialStore::open(dir.path()).unwrap();
    let mut c = SamsungClient::connect(LOCAL, store.clone(), cfg(true, port)).unwrap();
    c.send_key(TvRemoteKey::Home).unwrap();
    assert_eq!(wait_messages(&seen, 1)[0]["params"]["DataOfCmd"], "KEY_HOME");
    c.close();
    let creds = store.get(Vendor::Samsung, "127.0.0.1");
    assert_eq!(creds.secret.as_deref(), Some("TOKEN42"));
    let pin = creds.cert_sha256.clone().expect("certificate pinned on first use");
    assert_eq!(pin.len(), 64);
    assert!(!seen.lock().unwrap().urls[0].contains("token="));

    // Reconnect: token is sent, same certificate accepted.
    let c = SamsungClient::connect(LOCAL, store.clone(), cfg(true, port)).unwrap();
    assert!(seen.lock().unwrap().urls[1].ends_with("&token=TOKEN42"), "{}", seen.lock().unwrap().urls[1]);
    c.close();

    // A different certificate on the same IP is rejected.
    let (other_port, _) = tls_server(Reply::Connect(Some("X")));
    let err = SamsungClient::connect(LOCAL, store.clone(), cfg(true, other_port)).err().unwrap();
    assert_eq!(err.code, CaptureErrorCode::Network);
    assert!(err.message.contains("certificate changed"), "{}", err.message);
    assert_eq!(store.get(Vendor::Samsung, "127.0.0.1").cert_sha256.as_deref(), Some(pin.as_str()));
}

#[test]
fn unauthorized_is_permission_denied() {
    let (port, _) = plain_server(Reply::Unauthorized);
    let err = SamsungClient::connect(LOCAL, CredentialStore::in_memory(), cfg(false, port)).err().unwrap();
    assert_eq!(err.code, CaptureErrorCode::PermissionDenied);
}

#[test]
fn tv_client_dispatch() {
    let (port, _) = plain_server(Reply::Connect(None));
    let _ = port;
    let android = DiscoveredDevice {
        id: "x".into(),
        name: "Bravia".into(),
        kind: DeviceKind::AndroidTv,
        manufacturer: None,
        model: None,
        address: "127.0.0.1".into(),
        port: Some(6466),
        discovered_via: vec![DiscoveredVia::Mdns],
        control_protocols: vec![ControlProtocol::AndroidTvRemote],
        services: vec![],
        last_seen: String::new(),
    };
    let err = TvClient::connect(&android, CredentialStore::in_memory()).err().unwrap();
    assert_eq!(err.code, CaptureErrorCode::Unsupported);
    let features = desktop_tv_features();
    assert_eq!(features[&capture_core::FeatureKey::TvNativeScreenshot].level, capture_core::SupportLevel::Partial);
}
