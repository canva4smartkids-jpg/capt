//! Small pure helpers (file names, URL encoding, paths).

use std::path::{Path, PathBuf};

/// Mirrors `sanitizeFileName` in packages/settings: removes characters invalid on
/// Windows/macOS/Linux, collapses whitespace, avoids reserved names, max 120 chars.
pub fn sanitize_file_name(name: &str) -> String {
    let replaced: String = name.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20 { '_' } else { c }).collect();
    let mut s = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() {
        s = "CaptureOne".to_string();
    }
    let lower = s.to_ascii_lowercase();
    let reserved = matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || ((lower.starts_with("com") || lower.starts_with("lpt")) && lower.len() == 4 && lower.as_bytes()[3].is_ascii_digit());
    if reserved {
        s = format!("_{s}");
    }
    s.chars().take(120).collect()
}

/// First path `dir/stem.ext`, `dir/stem (2).ext`, ... that does not exist (or equals `current`).
pub fn unique_path(dir: &Path, stem: &str, ext: &str, current: Option<&Path>) -> PathBuf {
    let make = |n: u32| {
        let name = if n <= 1 { stem.to_string() } else { format!("{stem} ({n})") };
        if ext.is_empty() {
            dir.join(name)
        } else {
            dir.join(format!("{name}.{ext}"))
        }
    };
    for n in 1..10_000 {
        let p = make(n);
        if Some(p.as_path()) == current || !p.exists() {
            return p;
        }
    }
    make(10_000)
}

/// Percent-encodes a query component (RFC 3986 unreserved characters are kept).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn now_iso() -> String {
    capture_core::now_iso8601()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_matches_ts_rules() {
        assert_eq!(sanitize_file_name("a/b:c*?"), "a_b_c__");
        assert_eq!(sanitize_file_name("  hello   world.. "), "hello world");
        assert_eq!(sanitize_file_name(""), "CaptureOne");
        assert_eq!(sanitize_file_name("con"), "_con");
        assert_eq!(sanitize_file_name("COM1"), "_COM1");
        assert_eq!(sanitize_file_name("Display 1: take 1"), "Display 1_ take 1");
        assert_eq!(sanitize_file_name(&"x".repeat(200)).len(), 120);
    }

    #[test]
    fn unique_paths_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_path(dir.path(), "Demo", "mp4", None);
        assert_eq!(first.file_name().unwrap(), "Demo.mp4");
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(unique_path(dir.path(), "Demo", "mp4", None).file_name().unwrap(), "Demo (2).mp4");
        assert_eq!(unique_path(dir.path(), "Demo", "mp4", Some(&first)), first);
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode(":0.0/HDMI-1 a"), "%3A0.0%2FHDMI-1%20a");
    }
}
