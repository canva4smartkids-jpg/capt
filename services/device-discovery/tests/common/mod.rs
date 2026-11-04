//! Shared test helpers: schema validation and tiny fake servers.
#![allow(dead_code)]

use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn assert_valid(name: &str, value: &Value) {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../packages/capture-contracts/schema/{name}.schema.json"));
    let s: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&s).expect("schema compiles");
    let errors: Vec<String> = validator.iter_errors(value).map(|e| format!("{} at {}", e, e.instance_path())).collect();
    assert!(errors.is_empty(), "{name} invalid: {errors:#?}\nvalue: {}", serde_json::to_string_pretty(value).unwrap());
}

/// Serves fixed bodies by path over HTTP/1.x; records request paths.
pub struct FakeHttp {
    pub addr: SocketAddr,
    pub requests: Arc<Mutex<Vec<String>>>,
}

pub fn fake_http(bind: &str, routes: Vec<(String, &'static str, Vec<u8>)>) -> FakeHttp {
    let listener = TcpListener::bind(bind).unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let reqs = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let text = String::from_utf8_lossy(&buf);
            let path = text.split_whitespace().nth(1).unwrap_or("").to_string();
            reqs.lock().unwrap().push(path.clone());
            let resp = match routes.iter().find(|(p, _, _)| *p == path) {
                Some((_, ct, body)) => {
                    let mut r = format!("HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                    r.extend_from_slice(body);
                    r
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            };
            let _ = s.write_all(&resp);
        }
    });
    FakeHttp { addr, requests }
}

pub fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}
