//! QueryPerformanceCounter based timestamps → `Instant`.

use crate::timing::{instant_from_hns, qpc_to_hns};
use std::sync::OnceLock;
use std::time::Instant;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn frequency() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        // SAFETY: valid out pointer; cannot fail on Windows XP and later.
        let _ = unsafe { QueryPerformanceFrequency(&mut f) };
        f
    })
}

/// Current QPC time in 100 ns units.
pub(crate) fn qpc_now_hns() -> i64 {
    let mut c = 0i64;
    // SAFETY: valid out pointer.
    let _ = unsafe { QueryPerformanceCounter(&mut c) };
    qpc_to_hns(c, frequency())
}

/// Maps a 100 ns QPC timestamp (WGC `SystemRelativeTime`, WASAPI `pu64QPCPosition`) to `Instant`.
pub(crate) fn instant_from_qpc_hns(event_hns: i64) -> Instant {
    let now = Instant::now();
    let now_hns = qpc_now_hns();
    instant_from_hns(now, now_hns, event_hns)
}
