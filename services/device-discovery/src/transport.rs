//! TCP/TLS transport to a TV: trust-on-first-use certificate pinning, a WebSocket connector and
//! a minimal HTTP GET client (TVs serve device descriptions and screenshots over HTTP/1.x).
//!
//! LG (wss :3001) and Samsung (wss :8002) TVs present self-signed certificates. We never use a
//! generally permissive verifier: a [`PinnedVerifier`] is created per connection, only for the
//! TV's own IP address, and pins the certificate's SHA-256 on first use. A different certificate
//! later fails the connection (re-pair to accept a replaced certificate).

use capture_core::{CaptureError, CaptureErrorCode, CaptureResult};
use parking_lot::Mutex;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) fn net_err(msg: impl Into<String>) -> CaptureError {
    CaptureError::new(CaptureErrorCode::Network, msg)
}

#[derive(Debug)]
pub struct PinnedVerifier {
    expected: Option<String>,
    seen: Mutex<Option<String>>,
    provider: Arc<CryptoProvider>,
}

impl PinnedVerifier {
    pub fn new(expected_sha256_hex: Option<String>) -> Arc<Self> {
        Arc::new(Self { expected: expected_sha256_hex, seen: Mutex::new(None), provider: Arc::new(rustls::crypto::ring::default_provider()) })
    }
    /// SHA-256 (hex) of the certificate presented in the last handshake.
    pub fn seen(&self) -> Option<String> {
        self.seen.lock().clone()
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _i: &[CertificateDer<'_>], _n: &ServerName<'_>, _o: &[u8], _now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let fp = hex::encode(Sha256::digest(end_entity.as_ref()));
        *self.seen.lock() = Some(fp.clone());
        match &self.expected {
            Some(exp) if !exp.eq_ignore_ascii_case(&fp) => Err(rustls::Error::General(format!("TV certificate changed (pinned {exp}, got {fp}); re-pair the TV if this is expected"))),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

pub enum TvStream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl TvStream {
    pub fn tcp(&self) -> &TcpStream {
        match self {
            TvStream::Plain(s) => s,
            TvStream::Tls(s) => s.get_ref(),
        }
    }
    pub fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        self.tcp().set_read_timeout(d)
    }
}

impl Read for TvStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            TvStream::Plain(s) => s.read(buf),
            TvStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for TvStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            TvStream::Plain(s) => s.write(buf),
            TvStream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            TvStream::Plain(s) => s.flush(),
            TvStream::Tls(s) => s.flush(),
        }
    }
}

/// Opens TCP (and TLS with a pinned verifier when `tls`) to `ip:port`.
/// Returns the stream and, for TLS, the presented certificate fingerprint.
pub fn connect(ip: IpAddr, port: u16, tls: bool, pin: Option<String>, timeout: Duration) -> CaptureResult<(TvStream, Option<String>)> {
    let addr = SocketAddr::new(ip, port);
    let tcp = TcpStream::connect_timeout(&addr, timeout).map_err(|e| net_err(format!("cannot connect to {addr}: {e}")))?;
    tcp.set_read_timeout(Some(timeout)).map_err(|e| net_err(e.to_string()))?;
    tcp.set_write_timeout(Some(timeout)).map_err(|e| net_err(e.to_string()))?;
    let _ = tcp.set_nodelay(true);
    if !tls {
        return Ok((TvStream::Plain(tcp), None));
    }
    let verifier = PinnedVerifier::new(pin);
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| CaptureError::internal(format!("TLS config: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let name = ServerName::IpAddress(ip.into());
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| net_err(format!("TLS setup: {e}")))?;
    let mut stream = rustls::StreamOwned::new(conn, tcp);
    // Drive the handshake now so certificate errors surface here.
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock).map_err(|e| net_err(format!("TLS handshake with {addr} failed: {e}")))?;
    }
    Ok((TvStream::Tls(Box::new(stream)), verifier.seen()))
}

pub type TvSocket = tungstenite::WebSocket<TvStream>;

/// `1.2.3.4` or `[fe80::1]` for use in URLs.
pub fn host_literal(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

/// Parsed `ws://` / `wss://` / `http://` / `https://` URL with an IP-literal or host.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleUrl {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

pub fn parse_url(url: &str) -> CaptureResult<SimpleUrl> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| net_err(format!("invalid URL {url}")))?;
    let (secure, default_port) = match scheme.to_ascii_lowercase().as_str() {
        "ws" => (false, 80),
        "http" => (false, 80),
        "wss" => (true, 443),
        "https" => (true, 443),
        other => return Err(net_err(format!("unsupported URL scheme {other}"))),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped.find(']').ok_or_else(|| net_err("invalid IPv6 URL"))?;
        let host = &stripped[..end];
        let port = stripped[end + 1..].strip_prefix(':').map(|p| p.parse::<u16>()).transpose().map_err(|_| net_err("invalid port"))?;
        (host.to_string(), port.unwrap_or(default_port))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| net_err(format!("invalid port in {url}")))?),
            None => (authority.to_string(), default_port),
        }
    };
    Ok(SimpleUrl { secure, host, port, path })
}

/// Connects a WebSocket to `url`, which must point at `expected_ip` (URLs handed out by a TV must
/// never redirect us to another host).
pub fn ws_connect(url: &str, expected_ip: IpAddr, pin: Option<String>, timeout: Duration) -> CaptureResult<(TvSocket, Option<String>)> {
    let u = parse_url(url)?;
    let ip: IpAddr = u.host.parse().map_err(|_| net_err(format!("TV returned a non-IP host {:?}", u.host)))?;
    if ip != expected_ip {
        return Err(net_err(format!("refusing to connect to {ip}: expected the TV at {expected_ip}")));
    }
    let (stream, fp) = connect(ip, u.port, u.secure, pin, timeout)?;
    let (ws, _resp) = tungstenite::client::client(url, stream).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(tungstenite::Error::Http(resp)) => net_err(format!("WebSocket upgrade to {url} refused: HTTP {}", resp.status())),
        other => net_err(format!("WebSocket handshake with {url} failed: {other}")),
    })?;
    Ok((ws, fp))
}

/// True for read errors caused by the socket read timeout.
pub fn is_timeout(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Minimal HTTP/1.0 GET (no keep-alive, so the body ends at EOF; chunked replies are decoded
/// anyway because some embedded servers ignore the request version).
pub fn http_get(url: &str, expected_ip: Option<IpAddr>, pin: Option<String>, timeout: Duration, max_bytes: usize) -> CaptureResult<HttpResponse> {
    let u = parse_url(url)?;
    let ip: IpAddr = u.host.parse().map_err(|_| net_err(format!("non-IP host in {url}")))?;
    if let Some(exp) = expected_ip {
        if exp != ip {
            return Err(net_err(format!("refusing to fetch {url}: expected host {exp}")));
        }
    }
    let deadline = Instant::now() + timeout;
    let (mut stream, _) = connect(ip, u.port, u.secure, pin, timeout)?;
    let host = match ip {
        IpAddr::V6(v6) => format!("[{v6}]:{}", u.port),
        IpAddr::V4(v4) => format!("{v4}:{}", u.port),
    };
    let req = format!("GET {} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: CaptureOne/{}\r\nAccept: */*\r\nConnection: close\r\n\r\n", u.path, capture_core::APP_VERSION);
    stream.write_all(req.as_bytes()).map_err(|e| net_err(format!("{url}: {e}")))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        if Instant::now() > deadline {
            return Err(net_err(format!("{url}: timed out")));
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > max_bytes + 64 * 1024 {
                    return Err(net_err(format!("{url}: response larger than {max_bytes} bytes")));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // rustls reports a peer that closes without close_notify as UnexpectedEof.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(net_err(format!("{url}: {e}"))),
        }
    }
    parse_http_response(&raw).ok_or_else(|| net_err(format!("{url}: malformed HTTP response")))
}

pub fn parse_http_response(raw: &[u8]) -> Option<HttpResponse> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next()?;
    let status: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let mut body = raw[split + 4..].to_vec();
    let resp_hdr = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    if resp_hdr("transfer-encoding").map(|v| v.to_ascii_lowercase().contains("chunked")).unwrap_or(false) {
        body = decode_chunked(&body)?;
    } else if let Some(len) = resp_hdr("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if body.len() < len {
            return None; // truncated
        }
        body.truncate(len);
    }
    Some(HttpResponse { status, headers, body })
}

fn decode_chunked(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_str = std::str::from_utf8(&data[..line_end]).ok()?;
        let size = usize::from_str_radix(size_str.split(';').next()?.trim(), 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        if data.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

/// RFC 4648 base64 (standard alphabet, padded).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let u = parse_url("ws://192.168.1.5:3000/resources/abc/netinput.pointer.sock").unwrap();
        assert_eq!(u, SimpleUrl { secure: false, host: "192.168.1.5".into(), port: 3000, path: "/resources/abc/netinput.pointer.sock".into() });
        let u = parse_url("https://10.0.0.2").unwrap();
        assert_eq!((u.secure, u.port, u.path.as_str()), (true, 443, "/"));
        let u = parse_url("http://[fe80::1]:8080/x?y").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("fe80::1", 8080, "/x?y"));
        assert!(parse_url("ftp://x").is_err());
        assert!(parse_url("nope").is_err());
    }

    #[test]
    fn http_parsing() {
        let r = parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nX: y\r\n\r\nabcdef").unwrap();
        assert_eq!((r.status, r.body.as_slice(), r.header("x")), (200, &b"abc"[..], Some("y")));
        let r = parse_http_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;ext=1\r\nde\r\n0\r\n\r\n").unwrap();
        assert_eq!(r.body, b"abcde");
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc").is_none());
        assert!(parse_http_response(b"garbage").is_none());
    }

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"CaptureOne"), "Q2FwdHVyZU9uZQ==");
    }
}
