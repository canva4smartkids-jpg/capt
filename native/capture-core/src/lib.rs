//! capture-core: the native half of the CaptureOne shared capture contract.
pub mod backend;
pub mod frame;
pub mod presets;
pub mod state;
pub mod types;

pub use backend::*;
pub use frame::*;
pub use types::*;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// ISO-8601 UTC timestamp without external deps (seconds precision).
pub fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    iso8601_from_unix(secs)
}

pub fn iso8601_from_unix(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil-from-days (Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iso_dates() {
        assert_eq!(iso8601_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_unix(1_762_261_200), "2025-11-04T13:00:00Z");
        assert_eq!(iso8601_from_unix(951_782_400), "2000-02-29T00:00:00Z");
    }
    #[test]
    fn crop_and_rgba() {
        let mut data = vec![0u8; 4 * 4 * 4];
        for (i, px) in data.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[i as u8, 10, 20, 0]);
        }
        let f = VideoFrame::new_bgra(4, 4, 16, data).unwrap();
        let c = f.crop(Rect { x: 1, y: 1, width: 2, height: 2 }).unwrap();
        assert_eq!((c.width, c.height, c.stride), (2, 2, 8));
        assert_eq!(c.data[0], 5);
        assert_eq!(&c.to_rgba()[0..4], &[20, 10, 5, 255]);
        assert!(f.crop(Rect { x: 10, y: 10, width: 2, height: 2 }).is_err());
        let clamped = f.crop(Rect { x: -1, y: 2, width: 100, height: 100 }).unwrap();
        assert_eq!((clamped.width, clamped.height), (4, 2));
    }
    #[test]
    fn storage_full_mapping() {
        let e: CaptureError = std::io::Error::from_raw_os_error(28).into();
        assert_eq!(e.code, CaptureErrorCode::StorageFull);
    }
}
