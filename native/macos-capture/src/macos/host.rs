//! Facts about the running Mac (OS version, model, displays) via Foundation, sysctl and
//! CoreGraphics. None of these calls require any TCC permission.

use crate::capabilities::{HostInfo, OsVersion};
use capture_core::Size;
use objc2_core_graphics::{CGDirectDisplayID, CGDisplayCopyDisplayMode, CGDisplayMode, CGError, CGGetActiveDisplayList, CGGetOnlineDisplayList};
use objc2_foundation::NSProcessInfo;
use std::ffi::CStr;
use std::sync::OnceLock;

const MAX_DISPLAYS: usize = 32;

/// The running macOS version (cached).
pub(crate) fn os_version() -> OsVersion {
    static OS: OnceLock<OsVersion> = OnceLock::new();
    *OS.get_or_init(|| {
        let v = NSProcessInfo::processInfo().operatingSystemVersion();
        let os = OsVersion::new(v.majorVersion.max(0) as u32, v.minorVersion.max(0) as u32, v.patchVersion.max(0) as u32);
        // Binaries linked against pre-11 SDKs see the "10.16" compatibility version; ask the
        // kernel for the real product version in that case.
        if os.major == 10 && os.minor >= 16 {
            if let Some(real) = sysctl_string(c"kern.osproductversion").and_then(|s| OsVersion::parse(&s)) {
                return real;
            }
        }
        os
    })
}

/// `sysctlbyname` for string values (e.g. `hw.model` → "Mac14,2").
pub(crate) fn sysctl_string(name: &CStr) -> Option<String> {
    let mut len: libc::size_t = 0;
    // SAFETY: querying the size with a null output buffer is the documented sysctl pattern.
    let rc = unsafe { libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) };
    if rc != 0 || len == 0 || len > 4096 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` has room for `len` bytes and `len` tells sysctl its size.
    let rc = unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    while buf.last() == Some(&0) {
        buf.pop();
    }
    String::from_utf8(buf).ok().filter(|s| !s.is_empty())
}

fn display_list(online: bool) -> Vec<CGDirectDisplayID> {
    let mut ids = [0 as CGDirectDisplayID; MAX_DISPLAYS];
    let mut count: u32 = 0;
    // SAFETY: `ids` holds MAX_DISPLAYS entries and that is the capacity we pass.
    let err = unsafe {
        if online {
            CGGetOnlineDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count)
        } else {
            CGGetActiveDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count)
        }
    };
    if err != CGError::Success {
        return Vec::new();
    }
    ids[..(count as usize).min(MAX_DISPLAYS)].to_vec()
}

/// Physical pixel size of a display's current mode (`CGDisplayModeGetPixelWidth/Height`).
pub(crate) fn display_pixel_size(id: CGDirectDisplayID) -> Option<(u32, u32)> {
    let mode = CGDisplayCopyDisplayMode(id)?;
    let w = CGDisplayMode::pixel_width(Some(&mode));
    let h = CGDisplayMode::pixel_height(Some(&mode));
    (w > 0 && h > 0).then(|| (w.min(u32::MAX as usize) as u32, h.min(u32::MAX as usize) as u32))
}

/// Whether a display is still connected (used to detect unplugged displays while recording).
pub(crate) fn display_is_online(id: CGDirectDisplayID) -> bool {
    display_list(true).contains(&id)
}

pub(crate) fn host_info(os: OsVersion) -> HostInfo {
    let active = display_list(false);
    let largest_display = active
        .iter()
        .filter_map(|&id| display_pixel_size(id))
        .max_by_key(|(w, h)| *w as u64 * *h as u64)
        .map(|(width, height)| Size { width, height });
    HostInfo { os_version: os, device_model: sysctl_string(c"hw.model"), largest_display, display_count: active.len() }
}
