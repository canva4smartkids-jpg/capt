//! System tray icon + menu reflecting the recording state.

use crate::state::Shared;
use crate::{actions, windows};
use capture_core::RecordingState;
use parking_lot::Mutex;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

const IDLE_ICON: &[u8] = include_bytes!("../icons/tray-idle.png");
const REC_ICON: &[u8] = include_bytes!("../icons/tray-recording.png");

pub struct TrayHandles {
    icon: TrayIcon<Wry>,
    start_stop: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    showing_recording: bool,
    last: Option<RecordingState>,
}

#[derive(Default)]
pub struct TrayState(pub Mutex<Option<TrayHandles>>);

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let start_stop = MenuItem::with_id(app, "start-stop", "Start recording", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause-resume", "Pause recording", false, None::<&str>)?;
    let shot = MenuItem::with_id(app, "screenshot", "Take screenshot", true, None::<&str>)?;
    let region = MenuItem::with_id(app, "region-screenshot", "Screenshot of a region…", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show CaptureOne", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit CaptureOne", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&start_stop, &pause, &sep1, &shot, &region, &sep2, &show, &quit])?;
    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(IDLE_ICON)?)
        .tooltip("CaptureOne")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| actions::on_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                windows::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    if let Some(state) = app.try_state::<TrayState>() {
        *state.0.lock() = Some(TrayHandles { icon, start_stop, pause, showing_recording: false, last: None });
    }
    Ok(())
}

/// Updates menu labels/enabled state and the icon from the active session's phase.
pub fn refresh(app: &AppHandle, core: &Shared) {
    let phase = core.active().map(|c| c.phase()).unwrap_or(RecordingState::Idle);
    let Some(state) = app.try_state::<TrayState>() else { return };
    let mut guard = state.0.lock();
    let Some(h) = guard.as_mut() else { return };
    if h.last == Some(phase) {
        return;
    }
    h.last = Some(phase);
    use RecordingState::*;
    let (label, enabled) = match phase {
        Idle | Completed | Failed => ("Start recording", true),
        Countdown => ("Cancel countdown", true),
        Starting => ("Starting…", false),
        Recording | Paused => ("Stop recording", true),
        Stopping | Finalizing => ("Saving…", false),
    };
    let _ = h.start_stop.set_text(label);
    let _ = h.start_stop.set_enabled(enabled);
    let _ = h.pause.set_text(if phase == Paused { "Resume recording" } else { "Pause recording" });
    let _ = h.pause.set_enabled(matches!(phase, Recording | Paused));
    let recording = !matches!(phase, Idle | Completed | Failed);
    if recording != h.showing_recording {
        if let Ok(img) = Image::from_bytes(if recording { REC_ICON } else { IDLE_ICON }) {
            let _ = h.icon.set_icon(Some(img));
        }
        h.showing_recording = recording;
    }
    let tip = match phase {
        Countdown => "CaptureOne — starting…".to_string(),
        Paused => "CaptureOne — paused".to_string(),
        Stopping | Finalizing => "CaptureOne — saving recording…".to_string(),
        Recording | Starting => "CaptureOne — recording".to_string(),
        _ => "CaptureOne".to_string(),
    };
    let _ = h.icon.set_tooltip(Some(tip));
}

/// Live tooltip with the elapsed time (called from status updates).
pub fn tooltip(app: &AppHandle, state: RecordingState, elapsed_ms: u64) {
    if !matches!(state, RecordingState::Recording | RecordingState::Paused) {
        return;
    }
    if let Some(ts) = app.try_state::<TrayState>() {
        if let Some(h) = ts.0.lock().as_ref() {
            let label = if state == RecordingState::Paused { "paused" } else { "recording" };
            let _ = h.icon.set_tooltip(Some(format!("CaptureOne — {label} {}", crate::recording::format_duration(elapsed_ms))));
        }
    }
}
