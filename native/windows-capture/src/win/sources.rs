//! Display and window enumeration (physical pixels regardless of the process DPI awareness).

use super::os::{from_wide, wide};
use crate::ids::{self, DisplaySelector};
use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, CaptureSource, SourceKind};
use std::mem::size_of;
use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE, GW_OWNER, MONITORINFOF_PRIMARY, WS_CHILD, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

/// Makes the calling thread per-monitor DPI aware for its lifetime so Win32 geometry is in
/// physical pixels even when the host process is DPI unaware (e.g. an unmanifested CLI).
pub(crate) struct DpiAwarenessGuard(Option<DPI_AWARENESS_CONTEXT>);

impl DpiAwarenessGuard {
    pub(crate) fn per_monitor() -> Self {
        for ctx in [DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE] {
            // SAFETY: plain Win32 call; returns the previous context or NULL on failure.
            let prev = unsafe { SetThreadDpiAwarenessContext(ctx) };
            if !prev.0.is_null() {
                return Self(Some(prev));
            }
        }
        Self(None)
    }
}

impl Drop for DpiAwarenessGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            // SAFETY: restores the context returned by the earlier call on this thread.
            unsafe { SetThreadDpiAwarenessContext(prev) };
        }
    }
}

/// A display plus the handle needed to capture it.
#[derive(Debug, Clone)]
pub(crate) struct MonitorEntry {
    pub hmonitor: HMONITOR,
    pub source: CaptureSource,
}

unsafe extern "system" fn collect_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<HMONITOR>` passed by `monitors()` for this call only.
    let v = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    v.push(h);
    TRUE
}

pub(crate) fn monitors() -> CaptureResult<Vec<MonitorEntry>> {
    let _dpi = DpiAwarenessGuard::per_monitor();
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only touches `handles`, which outlives the call.
    let ok = unsafe { EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM(&mut handles as *mut Vec<HMONITOR> as isize)) };
    if !ok.as_bool() {
        return Err(CaptureError::internal(format!(
            "EnumDisplayMonitors failed: {}",
            windows::core::Error::from_thread()
        )));
    }
    let mut out = Vec::new();
    for h in handles {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: MONITORINFOEXW starts with MONITORINFO and cbSize announces the extended size.
        if !unsafe { GetMonitorInfoW(h, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool() {
            continue;
        }
        let device_name = from_wide(&info.szDevice);
        let rc = info.monitorInfo.rcMonitor;
        let (mut x, mut y, mut width, mut height) = (rc.left, rc.top, (rc.right - rc.left).max(0) as u32, (rc.bottom - rc.top).max(0) as u32);
        // The current display mode is always in physical pixels, independent of DPI awareness.
        let mut dm = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let name_w = wide(&device_name);
        // SAFETY: valid device name and DEVMODEW with dmSize set.
        if unsafe { EnumDisplaySettingsW(PCWSTR(name_w.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm) }.as_bool() && dm.dmPelsWidth > 0 && dm.dmPelsHeight > 0 {
            width = dm.dmPelsWidth;
            height = dm.dmPelsHeight;
            // SAFETY: for display devices the union holds the display variant (dmPosition).
            let pos = unsafe { dm.Anonymous1.Anonymous2.dmPosition };
            x = pos.x;
            y = pos.y;
        }
        if width == 0 || height == 0 {
            continue;
        }
        let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
        // SAFETY: valid out pointers.
        let scale = match unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
            Ok(()) if dpi_x > 0 => Some(dpi_x as f64 / 96.0),
            _ => None,
        };
        let is_primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;
        let friendly = windows_capture::monitor::Monitor::from_raw_hmonitor(h.0)
            .name()
            .ok()
            .filter(|n| !n.trim().is_empty());
        let number = ids::display_number(&device_name);
        let label = match number {
            Some(n) => format!("Display {n}"),
            None => device_name.clone(),
        };
        let name = match friendly {
            Some(f) => format!("{label}: {f}"),
            None => label,
        };
        out.push(MonitorEntry {
            hmonitor: h,
            source: CaptureSource {
                id: ids::canonical_device_name(&device_name),
                kind: SourceKind::Display,
                name,
                width,
                height,
                x: Some(x),
                y: Some(y),
                scale_factor: scale,
                is_primary: Some(is_primary),
                app_name: None,
                device_descriptor: None,
            },
        });
    }
    // Primary first, then by DISPLAY number, so index 0 is always the primary display.
    out.sort_by_key(|m| (!m.source.is_primary.unwrap_or(false), ids::display_number(&m.source.id).unwrap_or(u32::MAX)));
    Ok(out)
}

pub(crate) fn find_monitor(display_id: &str) -> CaptureResult<MonitorEntry> {
    let selector: DisplaySelector = ids::parse_display_id(display_id)?;
    monitors()?
        .into_iter()
        .enumerate()
        .find(|(i, m)| selector.matches(&m.source.id, *i))
        .map(|(_, m)| m)
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("display '{display_id}' not found")))
}

unsafe extern "system" fn collect_window(h: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<HWND>` passed by `windows()` for this call only.
    let v = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    v.push(h);
    TRUE
}

/// Shell surfaces that are visible and titled but are not application windows.
const SHELL_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow"];

pub(crate) fn windows() -> CaptureResult<Vec<CaptureSource>> {
    let _dpi = DpiAwarenessGuard::per_monitor();
    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: the callback only touches `handles`, which outlives the call.
    unsafe { EnumWindows(Some(collect_window), LPARAM(&mut handles as *mut Vec<HWND> as isize)) }
        .map_err(|e| CaptureError::internal(format!("EnumWindows failed: {e}")))?;
    // SAFETY: trivial Win32 call.
    let own_pid = unsafe { GetCurrentProcessId() };
    Ok(handles.into_iter().filter_map(|h| describe_window(h, own_pid)).collect())
}

fn window_text(h: HWND) -> Option<String> {
    // SAFETY: plain Win32 calls with a correctly sized buffer.
    let len = unsafe { GetWindowTextLengthW(h) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    if n <= 0 {
        return None;
    }
    let title = String::from_utf16_lossy(&buf[..n as usize]).trim().to_string();
    (!title.is_empty()).then_some(title)
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: buffer length is passed by the binding.
    let n = unsafe { GetClassNameW(h, &mut buf) };
    if n <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

pub(crate) fn is_cloaked(h: HWND) -> bool {
    let mut cloaked: u32 = 0;
    // SAFETY: `cloaked` is a writable DWORD as DWMWA_CLOAKED requires.
    let r = unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), size_of::<u32>() as u32) };
    r.is_ok() && cloaked != 0
}

/// Window bounds in physical screen pixels without the invisible resize borders.
pub(crate) fn window_bounds(h: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a writable RECT.
    let dwm = unsafe { DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut rect as *mut RECT).cast(), size_of::<RECT>() as u32) };
    if dwm.is_err() {
        // SAFETY: `rect` is a writable RECT.
        unsafe { GetWindowRect(h, &mut rect) }.ok()?;
    }
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

pub(crate) fn process_image_stem(pid: u32) -> Option<String> {
    // SAFETY: handle is closed below; buffer size is passed in/out.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        let r = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        r.ok()?;
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned())
    }
}

fn describe_window(h: HWND, own_pid: u32) -> Option<CaptureSource> {
    // SAFETY (whole function): read-only Win32 queries on a window handle; a handle that dies
    // in the middle just makes the calls fail.
    unsafe {
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return None;
        }
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        if style & WS_CHILD.0 != 0 || ex & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        // Owned popups/dialogs are skipped unless they explicitly ask for a taskbar button.
        if let Ok(owner) = GetWindow(h, GW_OWNER) {
            if !owner.0.is_null() && ex & WS_EX_APPWINDOW.0 == 0 {
                return None;
            }
        }
        if is_cloaked(h) {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == own_pid {
            return None;
        }
        let class = class_name(h);
        if SHELL_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(&class)) {
            return None;
        }
        let title = window_text(h)?;
        let rect = window_bounds(h)?;
        let dpi = GetDpiForWindow(h);
        Some(CaptureSource {
            id: ids::format_window_id(h.0 as isize),
            kind: SourceKind::Window,
            name: title,
            width: (rect.right - rect.left) as u32,
            height: (rect.bottom - rect.top) as u32,
            x: Some(rect.left),
            y: Some(rect.top),
            scale_factor: (dpi > 0).then(|| dpi as f64 / 96.0),
            is_primary: None,
            app_name: process_image_stem(pid),
            device_descriptor: None,
        })
    }
}

/// Validates a window id and returns its HWND.
pub(crate) fn find_window(window_id: &str) -> CaptureResult<HWND> {
    let raw = ids::parse_window_id(window_id)?;
    let h = HWND(raw as *mut core::ffi::c_void);
    // SAFETY: IsWindow accepts any value.
    if !unsafe { IsWindow(Some(h)) }.as_bool() {
        return Err(CaptureError::new(
            CaptureErrorCode::SourceNotFound,
            format!("window {window_id} does not exist (it may have been closed)"),
        ));
    }
    Ok(h)
}

pub(crate) fn is_minimized(h: HWND) -> bool {
    // SAFETY: plain query.
    unsafe { IsIconic(h) }.as_bool()
}
