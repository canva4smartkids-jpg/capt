//! Source id formats used by the Windows backend (platform independent).
//!
//! * Displays: the GDI device name, e.g. `\\.\DISPLAY1` (stable while the monitor stays on the
//!   same output). For convenience `DISPLAY1`, `display1` and a zero-based enumeration index
//!   (`0`) are accepted too.
//! * Windows: the HWND as a decimal string (`"132456"`); `0x`-prefixed hex is accepted.

use capture_core::{CaptureError, CaptureErrorCode, CaptureResult};

pub const DISPLAY_PREFIX: &str = r"\\.\";

/// How a display id selects a monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplaySelector {
    /// Canonical device name (`\\.\DISPLAY1`), compared case-insensitively.
    DeviceName(String),
    /// Zero-based position in `list_sources` order.
    Index(usize),
}

pub fn parse_display_id(id: &str) -> CaptureResult<DisplaySelector> {
    let id = id.trim();
    if id.is_empty() {
        return Err(CaptureError::invalid_config("display id is empty"));
    }
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        return id
            .parse::<usize>()
            .map(DisplaySelector::Index)
            .map_err(|_| CaptureError::invalid_config(format!("display index '{id}' is out of range")));
    }
    let bare = id.strip_prefix(DISPLAY_PREFIX).unwrap_or(id);
    if bare.is_empty() || bare.contains('\\') {
        return Err(CaptureError::invalid_config(format!(
            "'{id}' is not a display device name (expected e.g. \\\\.\\DISPLAY1)"
        )));
    }
    Ok(DisplaySelector::DeviceName(canonical_device_name(bare)))
}

/// `display1` / `\\.\display1` → `\\.\DISPLAY1`.
pub fn canonical_device_name(name: &str) -> String {
    let bare = name.strip_prefix(DISPLAY_PREFIX).unwrap_or(name);
    format!("{DISPLAY_PREFIX}{}", bare.to_ascii_uppercase())
}

impl DisplaySelector {
    /// True when the monitor `device_name` at list position `index` is selected.
    pub fn matches(&self, device_name: &str, index: usize) -> bool {
        match self {
            DisplaySelector::DeviceName(n) => n.eq_ignore_ascii_case(&canonical_device_name(device_name)),
            DisplaySelector::Index(i) => *i == index,
        }
    }
}

/// Number in `\\.\DISPLAY<n>`, if any (used for friendly fallback names).
pub fn display_number(device_name: &str) -> Option<u32> {
    let bare = device_name.strip_prefix(DISPLAY_PREFIX).unwrap_or(device_name);
    let upper = bare.to_ascii_uppercase();
    upper.strip_prefix("DISPLAY")?.parse().ok()
}

pub fn format_window_id(hwnd: isize) -> String {
    hwnd.to_string()
}

/// Parses a window id into the raw HWND value.
pub fn parse_window_id(id: &str) -> CaptureResult<isize> {
    let id = id.trim();
    let parsed = if let Some(hex) = id.strip_prefix("0x").or_else(|| id.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).ok().map(|v| v as isize)
    } else {
        id.parse::<isize>().ok()
    };
    match parsed {
        Some(0) | None => Err(CaptureError::new(
            CaptureErrorCode::SourceNotFound,
            format!("'{id}' is not a valid window id (expected the HWND as a decimal number)"),
        )),
        Some(v) => Ok(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_ids() {
        assert_eq!(parse_display_id(r"\\.\DISPLAY1").unwrap(), DisplaySelector::DeviceName(r"\\.\DISPLAY1".into()));
        assert_eq!(parse_display_id("display2").unwrap(), DisplaySelector::DeviceName(r"\\.\DISPLAY2".into()));
        assert_eq!(parse_display_id(" 0 ").unwrap(), DisplaySelector::Index(0));
        assert_eq!(parse_display_id("12").unwrap(), DisplaySelector::Index(12));
        assert!(parse_display_id("").is_err());
        assert!(parse_display_id(r"\\.\").is_err());
        assert!(parse_display_id(r"C:\foo").is_err());
        assert!(parse_display_id("99999999999999999999999999").is_err());
    }

    #[test]
    fn display_matching() {
        let sel = parse_display_id("DISPLAY3").unwrap();
        assert!(sel.matches(r"\\.\DISPLAY3", 7));
        assert!(sel.matches(r"\\.\display3", 0));
        assert!(!sel.matches(r"\\.\DISPLAY30", 0));
        assert!(!sel.matches(r"\\.\DISPLAY1", 0));
        let idx = parse_display_id("1").unwrap();
        assert!(idx.matches(r"\\.\DISPLAY9", 1));
        assert!(!idx.matches(r"\\.\DISPLAY9", 0));
        assert_eq!(canonical_device_name("display1"), r"\\.\DISPLAY1");
        assert_eq!(display_number(r"\\.\DISPLAY12"), Some(12));
        assert_eq!(display_number("WinDisc"), None);
    }

    #[test]
    fn window_ids() {
        assert_eq!(parse_window_id("132456").unwrap(), 132_456);
        assert_eq!(parse_window_id("0x1F").unwrap(), 31);
        assert_eq!(parse_window_id(&format_window_id(65_552)).unwrap(), 65_552);
        let err = parse_window_id("abc").unwrap_err();
        assert_eq!(err.code, CaptureErrorCode::SourceNotFound);
        assert!(parse_window_id("0").is_err());
        assert!(parse_window_id("").is_err());
    }
}
