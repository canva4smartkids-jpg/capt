//! `SCShareableContent` enumeration: displays and windows, and lookups by contract id.
//!
//! Ids: displays use the `CGDirectDisplayID` (stable for a given monitor across reboots),
//! windows use the `CGWindowID`, both as decimal strings.

use super::host::display_pixel_size;
use super::util::{capture_error_from_ns, wait, Shared};
use crate::geometry::{display_for_window, scale_factor, window_pixel_size, PointRect};
use block2::RcBlock;
use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, CaptureSource, SourceKind};
use objc2::rc::Retained;
use objc2_core_foundation::CGRect;
use objc2_core_graphics::{CGDisplayIsBuiltin, CGMainDisplayID};
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{SCDisplay, SCRunningApplication, SCShareableContent, SCWindow};
use std::sync::mpsc;
use std::time::Duration;

const CONTENT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn point_rect(r: CGRect) -> PointRect {
    PointRect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

pub(crate) fn cg_rect(r: PointRect) -> CGRect {
    CGRect {
        origin: objc2_core_foundation::CGPoint { x: r.x, y: r.y },
        size: objc2_core_foundation::CGSize { width: r.width, height: r.height },
    }
}

/// Fetches a snapshot of capturable content. Triggers the Screen Recording TCC check; without
/// permission this fails with `PERMISSION_DENIED` (SCStreamErrorUserDeclined).
///
/// * `on_screen_only = true` for source pickers.
/// * `false` for id lookups, so windows on other Spaces or minimised windows still resolve.
pub(crate) fn shareable_content(on_screen_only: bool) -> CaptureResult<Retained<SCShareableContent>> {
    let (tx, rx) = mpsc::sync_channel::<CaptureResult<Shared<SCShareableContent>>>(1);
    // Runs on an arbitrary ScreenCaptureKit queue. The arguments are only borrowed for the
    // duration of the call, so the content is retained before being sent to the caller.
    let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        // SAFETY: SCK passes either null or valid, live objects.
        let result = unsafe {
            if let Some(err) = error.as_ref() {
                Err(capture_error_from_ns(err))
            } else if let Some(content) = Retained::retain(content) {
                Ok(Shared(content))
            } else {
                Err(CaptureError::internal("SCShareableContent returned neither content nor an error"))
            }
        };
        let _ = tx.try_send(result);
    });
    // SAFETY: plain BOOL arguments and a heap block that SCK copies.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            on_screen_only,
            &handler,
        )
    };
    wait(&rx, CONTENT_TIMEOUT, "ScreenCaptureKit shareable content")?.map(|s| s.0)
}

/// Geometry of one display.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DisplayInfo {
    pub id: u32,
    /// Frame in global points.
    pub frame: PointRect,
    pub pixel_size: (u32, u32),
    pub scale: f64,
}

pub(crate) fn display_info(d: &SCDisplay) -> DisplayInfo {
    // SAFETY: simple property getters on a valid SCDisplay.
    let (id, frame, point_w, point_h) = unsafe { (d.displayID(), point_rect(d.frame()), d.width(), d.height()) };
    let (point_w, point_h) = (point_w.max(1) as f64, point_h.max(1) as f64);
    let pixel_size = display_pixel_size(id).unwrap_or((point_w as u32, point_h as u32));
    DisplayInfo { id, frame, pixel_size, scale: scale_factor(pixel_size.0, point_w) }
}

pub(crate) fn displays(content: &SCShareableContent) -> Vec<(Retained<SCDisplay>, DisplayInfo)> {
    // SAFETY: getter on a valid content snapshot.
    let list = unsafe { content.displays() };
    list.iter().map(|d| {
        let info = display_info(&d);
        (d, info)
    }).collect()
}

fn parse_id(kind: SourceKind, id: &str) -> CaptureResult<u32> {
    id.trim()
        .parse::<u32>()
        .map_err(|_| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("{kind:?} id '{id}' is not a macOS display/window id")))
}

pub(crate) fn find_display(content: &SCShareableContent, id: &str) -> CaptureResult<(Retained<SCDisplay>, DisplayInfo)> {
    let want = parse_id(SourceKind::Display, id)?;
    displays(content)
        .into_iter()
        .find(|(_, info)| info.id == want)
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("display '{id}' is not connected")))
}

pub(crate) fn find_window(content: &SCShareableContent, id: &str) -> CaptureResult<Retained<SCWindow>> {
    let want = parse_id(SourceKind::Window, id)?;
    // SAFETY: getters on valid objects.
    unsafe { content.windows() }
        .iter()
        .find(|w| unsafe { w.windowID() } == want)
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("window '{id}' does not exist (closed?)")))
}

/// This process's `SCRunningApplication` (present only when it owns windows, e.g. the
/// desktop app), used to keep CaptureOne's own UI out of display captures.
pub(crate) fn own_application(content: &SCShareableContent) -> Option<Retained<SCRunningApplication>> {
    let pid = std::process::id() as libc::pid_t;
    // SAFETY: getters on valid objects.
    unsafe { content.applications() }.iter().find(|a| unsafe { a.processID() } == pid)
}

/// Backing scale of the display a window is (mostly) on.
pub(crate) fn window_display_scale(window_frame: &PointRect, displays: &[DisplayInfo]) -> f64 {
    let frames: Vec<PointRect> = displays.iter().map(|d| d.frame).collect();
    let main = CGMainDisplayID();
    display_for_window(window_frame, &frames)
        .map(|i| displays[i].scale)
        .or_else(|| displays.iter().find(|d| d.id == main).map(|d| d.scale))
        .unwrap_or(1.0)
}

fn display_name(id: u32, index: usize) -> String {
    if CGDisplayIsBuiltin(id) {
        "Built-in Display".to_string()
    } else {
        format!("Display {}", index + 1)
    }
}

pub(crate) fn list_sources(kinds: &[SourceKind]) -> CaptureResult<Vec<CaptureSource>> {
    let want = |k: SourceKind| kinds.is_empty() || kinds.contains(&k);
    let (want_displays, want_windows) = (want(SourceKind::Display), want(SourceKind::Window));
    if !want_displays && !want_windows {
        // `device` sources (capture cards) are not implemented on macOS.
        return Ok(Vec::new());
    }
    let content = shareable_content(true)?;
    let displays: Vec<DisplayInfo> = displays(&content).into_iter().map(|(_, i)| i).collect();
    let main = CGMainDisplayID();
    let mut out = Vec::new();

    if want_displays {
        for (index, d) in displays.iter().enumerate() {
            out.push(CaptureSource {
                id: d.id.to_string(),
                kind: SourceKind::Display,
                name: display_name(d.id, index),
                width: d.pixel_size.0,
                height: d.pixel_size.1,
                x: Some(d.frame.x.round() as i32),
                y: Some(d.frame.y.round() as i32),
                scale_factor: Some(d.scale),
                is_primary: Some(d.id == main),
                app_name: None,
                device_descriptor: None,
            });
        }
    }

    if want_windows {
        let own_pid = std::process::id() as libc::pid_t;
        // SAFETY: getters on valid SCK objects throughout.
        for w in unsafe { content.windows() }.iter() {
            let (on_screen, layer, frame, title, app) =
                unsafe { (w.isOnScreen(), w.windowLayer(), point_rect(w.frame()), w.title(), w.owningApplication()) };
            // Normal application windows only: on screen, layer 0 (no menu bar, Dock, overlays),
            // with a title, not our own UI.
            if !on_screen || layer != 0 || frame.width < 1.0 || frame.height < 1.0 {
                continue;
            }
            let title = title.map(|t| t.to_string()).unwrap_or_default();
            if title.trim().is_empty() {
                continue;
            }
            if app.as_ref().is_some_and(|a| unsafe { a.processID() } == own_pid) {
                continue;
            }
            let app_name = app.map(|a| unsafe { a.applicationName() }.to_string()).filter(|n| !n.is_empty());
            let scale = window_display_scale(&frame, &displays);
            let (width, height) = window_pixel_size(&frame, scale);
            out.push(CaptureSource {
                id: unsafe { w.windowID() }.to_string(),
                kind: SourceKind::Window,
                name: title,
                width,
                height,
                x: Some(frame.x.round() as i32),
                y: Some(frame.y.round() as i32),
                scale_factor: Some(scale),
                is_primary: None,
                app_name,
                device_descriptor: None,
            });
        }
    }
    Ok(out)
}
