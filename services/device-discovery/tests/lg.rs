//! LG SSAP client against a fake webOS SSAP server that checks the exact messages sent.
#![allow(clippy::result_large_err)] // tungstenite handshake callback signature

mod common;

use capture_core::CaptureErrorCode;
use device_discovery::lg::{LgClient, LgClientConfig};
use device_discovery::*;
use serde_json::{json, Value};
use std::net::{IpAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::handshake::server::{Request, Response};
use tungstenite::Message;

#[derive(Default)]
struct Seen {
    registers: Vec<Value>,
    requests: Vec<Value>,
    pointer_frames: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Accept,
    Reject,
}

fn fake_lg(mode: Mode, image_url: String, pointer_host: &'static str) -> (u16, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let seen = seen2.clone();
            let image_url = image_url.clone();
            std::thread::spawn(move || {
                let path = Arc::new(Mutex::new(String::new()));
                let p2 = path.clone();
                let Ok(mut ws) = tungstenite::accept_hdr(stream, move |req: &Request, resp: Response| {
                    *p2.lock().unwrap() = req.uri().path().to_string();
                    Ok(resp)
                }) else {
                    return;
                };
                let path = path.lock().unwrap().clone();
                if path.ends_with("netinput.pointer.sock") {
                    while let Ok(msg) = ws.read() {
                        if let Message::Text(t) = msg {
                            seen.lock().unwrap().pointer_frames.push(t.to_string());
                        }
                    }
                    return;
                }
                while let Ok(msg) = ws.read() {
                    let Message::Text(t) = msg else { continue };
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    let id = v["id"].clone();
                    match v["type"].as_str() {
                        Some("register") => {
                            seen.lock().unwrap().registers.push(v.clone());
                            ws.send(Message::text(json!({"type":"response","id":id,"payload":{"pairingType":"PROMPT","returnValue":true}}).to_string())).unwrap();
                            if mode == Mode::Reject {
                                ws.send(Message::text(json!({"type":"error","id":id,"error":"403 User rejected pairing","payload":{}}).to_string())).unwrap();
                            } else {
                                // An unrelated event first: the client must ignore it.
                                ws.send(Message::text(json!({"type":"response","id":"other","payload":{}}).to_string())).unwrap();
                                ws.send(Message::text(json!({"type":"registered","id":id,"payload":{"client-key":"KEY123"}}).to_string())).unwrap();
                            }
                        }
                        Some("request") => {
                            seen.lock().unwrap().requests.push(v.clone());
                            let payload = match v["uri"].as_str().unwrap() {
                                "ssap://system/getSystemInfo" => json!({"returnValue":true,"modelName":"OLED55C1PUB","receiverType":"dvb"}),
                                "ssap://com.webos.service.update/getCurrentSWInformation" => json!({"returnValue":true,"product_name":"webOSTV 6.0","major_ver":"03","minor_ver":"20.50"}),
                                "ssap://tv/executeOneShot" => json!({"returnValue":true,"imageUri":image_url}),
                                "ssap://com.webos.service.networkinput/getPointerInputSocket" => {
                                    json!({"returnValue":true,"socketPath":format!("ws://{pointer_host}:{port}/resources/abc/netinput.pointer.sock")})
                                }
                                "ssap://system/turnOff" => {
                                    ws.send(Message::text(json!({"type":"error","id":id,"error":"401 insufficient permissions","payload":{}}).to_string())).unwrap();
                                    continue;
                                }
                                "ssap://audio/volumeUp" => json!({"returnValue":true}),
                                _ => json!({"returnValue":false,"errorText":"no such service"}),
                            };
                            ws.send(Message::text(json!({"type":"response","id":id,"payload":payload}).to_string())).unwrap();
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    (port, seen)
}

fn cfg(port: u16) -> LgClientConfig {
    LgClientConfig { endpoints: vec![(false, port)], pairing_timeout: Duration::from_secs(5), request_timeout: Duration::from_secs(5), ..Default::default() }
}

const LOCAL: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

#[test]
fn pairing_requests_keys_and_screenshot() {
    let jpeg = common::jpeg(320, 180);
    let http = common::fake_http("127.0.0.1:0", vec![("/resources/xyz/capture.jpg".into(), "image/jpeg", jpeg.clone())]);
    let (port, seen) = fake_lg(Mode::Accept, format!("http://{}/resources/xyz/capture.jpg", http.addr), "127.0.0.1");
    let dir = tempfile::tempdir().unwrap();
    let store = CredentialStore::open(dir.path()).unwrap();

    let mut c = LgClient::connect(LOCAL, store.clone(), cfg(port)).unwrap();
    {
        let s = seen.lock().unwrap();
        let reg = &s.registers[0];
        assert_eq!(reg["type"], "register");
        assert_eq!(reg["payload"]["pairingType"], "PROMPT");
        assert!(reg["payload"].get("client-key").is_none(), "first pairing sends no client-key");
        let perms: Vec<&str> = reg["payload"]["manifest"]["permissions"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
        for needed in ["CONTROL_TV_SCREEN", "CONTROL_AUDIO", "CONTROL_INPUT_MEDIA_PLAYBACK", "CONTROL_POWER", "READ_UPDATE_INFO"] {
            assert!(perms.contains(&needed), "manifest lacks {needed}");
        }
    }
    assert_eq!(store.get(store::Vendor::Lg, "127.0.0.1").secret.as_deref(), Some("KEY123"), "client-key persisted");

    assert_eq!(c.system_info().unwrap()["modelName"], "OLED55C1PUB");
    assert_eq!(c.software_info().unwrap()["product_name"], "webOSTV 6.0");

    c.send_key(TvRemoteKey::Up).unwrap();
    c.send_key(TvRemoteKey::Enter).unwrap();
    c.send_key(TvRemoteKey::VolumeUp).unwrap();
    let err = c.send_key(TvRemoteKey::Power).unwrap_err();
    assert_eq!(err.code, CaptureErrorCode::PermissionDenied);
    let err = c.request("ssap://does/not/exist", json!({})).unwrap_err();
    assert_eq!(err.code, CaptureErrorCode::Unsupported);

    let out = tempfile::tempdir().unwrap();
    let m = c.screenshot(out.path(), "LG OLED55C1").unwrap();
    assert_eq!((m.width, m.height), (320, 180));
    assert_eq!(std::fs::read(&m.path).unwrap(), jpeg);
    assert_eq!(m.platform.as_deref(), Some("webos"));
    common::assert_valid("MediaMetadata", &serde_json::to_value(&m).unwrap());

    std::thread::sleep(Duration::from_millis(200));
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.pointer_frames, vec!["type:button\nname:UP\n\n".to_string(), "type:button\nname:ENTER\n\n".to_string()]);
        let uris: Vec<&str> = s.requests.iter().map(|r| r["uri"].as_str().unwrap()).collect();
        assert_eq!(
            uris,
            vec![
                "ssap://system/getSystemInfo",
                "ssap://com.webos.service.update/getCurrentSWInformation",
                "ssap://com.webos.service.networkinput/getPointerInputSocket",
                "ssap://audio/volumeUp",
                "ssap://system/turnOff",
                "ssap://does/not/exist",
                "ssap://tv/executeOneShot",
            ]
        );
        for r in &s.requests {
            assert_eq!(r["type"], "request");
            assert!(r["id"].as_str().unwrap().starts_with("req_"));
        }
    }
    c.close();

    // Second connection reuses the stored client-key (no new prompt needed on a real TV).
    let c2 = LgClient::connect(LOCAL, store, cfg(port)).unwrap();
    assert_eq!(seen.lock().unwrap().registers[1]["payload"]["client-key"], "KEY123");
    c2.close();
}

#[test]
fn rejected_pairing_is_permission_denied() {
    let (port, _seen) = fake_lg(Mode::Reject, String::new(), "127.0.0.1");
    let err = LgClient::connect(LOCAL, CredentialStore::in_memory(), cfg(port)).err().unwrap();
    assert_eq!(err.code, CaptureErrorCode::PermissionDenied);
    assert!(err.message.contains("rejected"));
}

#[test]
fn pointer_socket_on_foreign_host_is_refused() {
    let (port, _seen) = fake_lg(Mode::Accept, String::new(), "127.0.0.9");
    let mut c = LgClient::connect(LOCAL, CredentialStore::in_memory(), cfg(port)).unwrap();
    let err = c.send_key(TvRemoteKey::Down).unwrap_err();
    assert!(err.message.contains("refusing"), "{}", err.message);
}

#[test]
fn screenshot_from_foreign_host_is_refused() {
    let (port, _seen) = fake_lg(Mode::Accept, "http://127.0.0.9:1/x.jpg".into(), "127.0.0.1");
    let mut c = LgClient::connect(LOCAL, CredentialStore::in_memory(), cfg(port)).unwrap();
    let err = c.screenshot(std::path::Path::new("/nonexistent"), "x").unwrap_err();
    assert!(err.message.contains("refusing"), "{}", err.message);
}

#[test]
fn unreachable_tv_is_network_error() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let err = LgClient::connect(LOCAL, CredentialStore::in_memory(), LgClientConfig { endpoints: vec![(true, port), (false, port)], ..cfg(port) }).err().unwrap();
    assert_eq!(err.code, CaptureErrorCode::Network);
}
