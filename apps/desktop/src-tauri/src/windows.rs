//! Secondary windows: the recording indicator bar and the region selector overlay.

use crate::util::percent_encode;
use capture_core::*;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";
pub const INDICATOR: &str = "indicator";
pub const REGION: &str = "region";
/// Window titles; used to keep our own overlays out of the window source list.
pub const INDICATOR_TITLE: &str = "CaptureOne Indicator";
pub const REGION_TITLE: &str = "CaptureOne Region Selector";

fn tauri_err(e: tauri::Error) -> CaptureError {
    CaptureError::internal(format!("window: {e}"))
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Hides the main window if it is visible; returns whether it was.
pub fn hide_main(app: &AppHandle) -> bool {
    match app.get_webview_window(MAIN) {
        Some(w) if w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) => w.hide().is_ok(),
        _ => false,
    }
}

pub fn minimize_main(app: &AppHandle) -> bool {
    match app.get_webview_window(MAIN) {
        Some(w) if w.is_visible().unwrap_or(false) => w.minimize().is_ok(),
        _ => false,
    }
}

/// Small always-on-top bar (timer, pause/resume, stop) at the top centre of the primary display.
pub fn open_indicator(app: &AppHandle) -> CaptureResult<()> {
    if let Some(w) = app.get_webview_window(INDICATOR) {
        return w.show().map_err(tauri_err);
    }
    let (w, h) = (288.0, 50.0);
    let win = WebviewWindowBuilder::new(app, INDICATOR, WebviewUrl::App("index.html#/indicator".into()))
        .title(INDICATOR_TITLE)
        .inner_size(w, h)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        // Best effort: keeps the bar out of captures where the OS supports it (Windows/macOS).
        .content_protected(true)
        .build()
        .map_err(tauri_err)?;
    if let Ok(Some(m)) = win.primary_monitor() {
        let scale = m.scale_factor();
        let x = m.position().x as f64 + (m.size().width as f64 - w * scale) / 2.0;
        let y = m.position().y as f64 + 16.0 * scale;
        let _ = win.set_position(Position::Physical(PhysicalPosition { x: x.round() as i32, y: y.round() as i32 }));
    }
    win.show().map_err(tauri_err)
}

pub fn close_indicator(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(INDICATOR) {
        let _ = w.close();
    }
}

/// Opens the (initially hidden) full-screen region selector on the monitor showing `display`.
/// The page captures a frozen frame of the display and then shows itself.
pub fn open_region(app: &AppHandle, display: &CaptureSource) -> CaptureResult<()> {
    close_region(app);
    let url = format!("index.html#/region?displayId={}&w={}&h={}", percent_encode(&display.id), display.width, display.height);
    let builder = WebviewWindowBuilder::new(app, REGION, WebviewUrl::App(url.into()))
        .title(REGION_TITLE)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .focused(true)
        .visible(false);
    // macOS needs the private API for transparency; the frozen frame makes it unnecessary there.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.transparent(true);
    let win = builder.build().map_err(tauri_err)?;
    let monitors = app.available_monitors().unwrap_or_default();
    let (dx, dy) = (display.x.unwrap_or(0), display.y.unwrap_or(0));
    let monitor = monitors
        .iter()
        .find(|m| m.position().x == dx && m.position().y == dy)
        .or_else(|| monitors.iter().find(|m| m.size().width == display.width && m.size().height == display.height))
        .cloned()
        .or_else(|| app.primary_monitor().ok().flatten());
    match monitor {
        Some(m) => {
            let _ = win.set_position(Position::Physical(*m.position()));
            let _ = win.set_size(Size::Physical(*m.size()));
        }
        None => {
            let _ = win.set_position(Position::Physical(PhysicalPosition { x: dx, y: dy }));
            let _ = win.set_size(Size::Physical(PhysicalSize { width: display.width, height: display.height }));
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Lets X11 window managers drop panels/docks above the overlay.
        let _ = win.set_fullscreen(true);
    }
    Ok(())
}

pub fn close_region(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(REGION) {
        let _ = w.close();
    }
}
