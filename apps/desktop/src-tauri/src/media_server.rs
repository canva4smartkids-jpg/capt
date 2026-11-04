//! Loopback media endpoint for `<video>` previews on Linux.
//!
//! WebKitGTK's GStreamer player only loads http(s)/file/blob URLs, so recordings cannot be
//! played through Tauri's `asset://` scheme there (images can). This serves *library items
//! only* (looked up by id, never arbitrary paths) on 127.0.0.1 with a random per-launch token
//! in the path, supporting HTTP Range requests for seeking.

use crate::state::Shared;
use capture_core::*;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaServerInfo {
    pub port: u16,
    pub token: String,
}

const MAX_CONNECTIONS: usize = 32;

/// Starts the server on first use; returns its address info.
pub fn ensure(core: &Shared) -> CaptureResult<MediaServerInfo> {
    let mut slot = core.media_server.lock();
    if let Some(info) = slot.as_ref() {
        return Ok(info.clone());
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| CaptureError::new(CaptureErrorCode::Network, format!("media preview server: {e}")))?;
    let port = listener.local_addr().map_err(|e| CaptureError::new(CaptureErrorCode::Network, e.to_string()))?.port();
    let info = MediaServerInfo { port, token: uuid::Uuid::new_v4().simple().to_string() };
    let (core2, token) = (core.clone(), info.token.clone());
    std::thread::Builder::new()
        .name("media-preview".into())
        .spawn(move || {
            let live = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            for stream in listener.incoming().flatten() {
                if live.load(std::sync::atomic::Ordering::SeqCst) >= MAX_CONNECTIONS {
                    continue; // drop the connection; the <video> element retries
                }
                live.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let live2 = live.clone();
                let (core3, token3) = (core2.clone(), token.clone());
                let _ = std::thread::Builder::new().name("media-preview-conn".into()).spawn(move || {
                    if let Err(e) = handle(stream, &core3, &token3) {
                        log::debug!("media preview connection: {e}");
                    }
                    live2.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                });
            }
        })
        .map_err(|e| CaptureError::internal(format!("media preview thread: {e}")))?;
    *slot = Some(info.clone());
    Ok(info)
}

pub fn url_for(info: &MediaServerInfo, id: &str) -> String {
    format!("http://127.0.0.1:{}/{}/media/{}", info.port, info.token, crate::util::percent_encode(id))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// Parses a single `bytes=` range against a file length. Returns (start, end inclusive).
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let (start, end) = if a.is_empty() {
        let suffix: u64 = b.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        (len.saturating_sub(suffix), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
        (start, end)
    };
    (start <= end && start < len).then_some((start, end))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn respond(stream: &mut TcpStream, status: &str, extra: &str) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n{extra}\r\n")
}

fn handle(mut stream: TcpStream, core: &Shared, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    // Bound everything read from the socket (header bytes) so a local peer cannot balloon memory.
    let mut reader = BufReader::new(stream.try_clone()?.take(16 * 1024));
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut range = None;
    for _ in 0..100 {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method != "GET" && method != "HEAD" {
        return respond(&mut stream, "405 Method Not Allowed", "Allow: GET, HEAD\r\n");
    }
    let prefix = format!("/{token}/media/");
    let Some(id) = target.strip_prefix(&prefix).map(percent_decode) else {
        return respond(&mut stream, "404 Not Found", "");
    };
    let item = core.library.lock().get(&id).ok().flatten();
    let Some(item) = item else { return respond(&mut stream, "404 Not Found", "") };
    let path = Path::new(&item.path);
    let Ok(mut file) = File::open(path) else { return respond(&mut stream, "404 Not Found", "") };
    let len = file.metadata()?.len();
    let ctype = content_type(path);
    let (status, start, end) = match range.as_deref() {
        Some(r) => match parse_range(r, len) {
            Some((s, e)) => ("206 Partial Content", s, e),
            None => return respond(&mut stream, "416 Range Not Satisfiable", &format!("Content-Range: bytes */{len}\r\n")),
        },
        None => ("200 OK", 0, len.saturating_sub(1)),
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    let content_range = if status.starts_with("206") { format!("Content-Range: bytes {start}-{end}/{len}\r\n") } else { String::new() };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {count}\r\nAccept-Ranges: bytes\r\n{content_range}Cache-Control: no-store\r\nConnection: close\r\n\r\n"
    )?;
    if method == "GET" && count > 0 {
        file.seek(SeekFrom::Start(start))?;
        std::io::copy(&mut (&mut file).take(count), &mut stream)?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::settings::Settings;
    use crate::state::{AppCore, AppPaths};
    use std::sync::Arc;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-1", 100), None);
        assert_eq!(parse_range("bytes=0-1,4-5", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }

    fn get(port: u16, path: &str, range: Option<&str>) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let r = range.map(|r| format!("Range: {r}\r\n")).unwrap_or_default();
        write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n{r}\r\n").unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&raw[..split]).to_string(), raw[split + 4..].to_vec())
    }

    #[test]
    fn serves_library_items_with_ranges_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a b.mp4");
        std::fs::write(&file, (0u8..=99).collect::<Vec<_>>()).unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert(&MediaMetadata {
            id: "rec 1".into(),
            kind: MediaKind::Video,
            path: file.to_string_lossy().to_string(),
            file_name: "a b.mp4".into(),
            title: String::new(),
            created_at: "2026-11-04T00:00:00Z".into(),
            size_bytes: 100,
            width: 16,
            height: 16,
            duration_ms: None,
            fps: None,
            video_codec: None,
            video_bitrate_kbps: None,
            audio_tracks: vec![],
            image_format: None,
            container: "mp4".into(),
            source_name: None,
            platform: None,
            validation: None,
            thumbnail_path: None,
        })
        .unwrap();
        let paths = AppPaths { config_dir: dir.path().into(), data_dir: dir.path().into(), log_dir: dir.path().into(), db_path: dir.path().join("x"), default_output_dir: dir.path().into() };
        let core: Shared = Arc::new(AppCore::new(paths, "0".into(), Err(CaptureError::unsupported("test")), Settings::defaults("/v"), lib));
        let info = ensure(&core).unwrap();
        assert_eq!(ensure(&core).unwrap(), info, "started once");
        let url = url_for(&info, "rec 1");
        let path = url.trim_start_matches(&format!("http://127.0.0.1:{}", info.port)).to_string();

        let (head, body) = get(info.port, &path, None);
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Content-Type: video/mp4") && head.contains("Accept-Ranges: bytes"));
        assert_eq!(body.len(), 100);

        let (head, body) = get(info.port, &path, Some("bytes=10-19"));
        assert!(head.starts_with("HTTP/1.1 206") && head.contains("Content-Range: bytes 10-19/100"), "{head}");
        assert_eq!(body, (10u8..=19).collect::<Vec<_>>());

        assert!(get(info.port, &path, Some("bytes=200-")).0.starts_with("HTTP/1.1 416"));
        assert!(get(info.port, "/wrong-token/media/rec%201", None).0.starts_with("HTTP/1.1 404"));
        assert!(get(info.port, &format!("/{}/media/unknown", info.token), None).0.starts_with("HTTP/1.1 404"));
        assert!(get(info.port, &format!("/{}/media/..%2F..%2Fetc%2Fpasswd", info.token), None).0.starts_with("HTTP/1.1 404"), "ids only, never paths");

        // Loopback only, random 128-bit token, and an oversized request line cannot exhaust memory.
        assert!(TcpStream::connect(("127.0.0.1", info.port)).unwrap().local_addr().unwrap().ip().is_loopback());
        assert_eq!(info.token.len(), 32);
        let mut s = TcpStream::connect(("127.0.0.1", info.port)).unwrap();
        let _ = s.write_all(format!("GET /{}/media/{} HTTP/1.1\r\n\r\n", info.token, "a".repeat(200_000)).as_bytes());
        let mut sink = Vec::new();
        let _ = s.read_to_end(&mut sink);
        assert!(!String::from_utf8_lossy(&sink).contains("200 OK"));
    }
}
