//! Actions shared by the tray menu, global shortcuts and commands.

use crate::events::{emit_capture, CaptureEvent, ShortcutAction};
use crate::state::{SessionCtl, Shared};
use crate::{recording, region, windows};
use capture_core::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "start-stop" => perform(app, ShortcutAction::StartStop),
        "pause-resume" => perform(app, ShortcutAction::PauseResume),
        "screenshot" => perform(app, ShortcutAction::Screenshot),
        "region-screenshot" => perform(app, ShortcutAction::RegionScreenshot),
        "show" => windows::show_main(app),
        "quit" => quit(app),
        _ => {}
    }
}

/// Runs an action off the main thread; failures are reported as capture events + notification.
pub fn perform(app: &AppHandle, action: ShortcutAction) {
    let app = app.clone();
    let _ = std::thread::Builder::new().name("captureone-action".into()).spawn(move || {
        let core = app.state::<Shared>().inner().clone();
        if let Err(e) = run(&app, &core, action) {
            log::warn!("{action:?} failed: {e}");
            emit_capture(&app, CaptureEvent::Error { session_id: None, error: e.clone() });
            recording::notify(&app, "CaptureOne", &e.message);
        }
    });
}

fn run(app: &AppHandle, core: &Shared, action: ShortcutAction) -> CaptureResult<()> {
    match action {
        ShortcutAction::StartStop => match core.active() {
            Some(ctl) => match ctl.phase() {
                RecordingState::Countdown | RecordingState::Idle => cancel(&ctl),
                RecordingState::Recording | RecordingState::Paused => stop(core, &ctl).map(|_| ()),
                _ => Ok(()),
            },
            None => {
                let cfg = recording::default_config(core)?;
                recording::start(app, core, cfg).map(|_| ())
            }
        },
        ShortcutAction::PauseResume => {
            let ctl = core.active().ok_or_else(|| CaptureError::invalid_state("nothing is being recorded"))?;
            let s = ctl.engine_session().ok_or_else(|| CaptureError::invalid_state("the recording has not started yet"))?;
            match ctl.phase() {
                RecordingState::Recording => s.pause(),
                RecordingState::Paused => s.resume(),
                p => Err(CaptureError::invalid_state(format!("cannot pause or resume while {p:?}"))),
            }
        }
        ShortcutAction::Screenshot => {
            let target = match core.last_target.lock().clone() {
                Some(t) => t,
                None => CaptureTarget::Display { display_id: recording::primary_display(core)?.id },
            };
            take_screenshot(app, core, default_request(core, target), true).map(|_| ())
        }
        ShortcutAction::RegionScreenshot => {
            let display_id = match core.last_target.lock().clone() {
                Some(CaptureTarget::Display { display_id }) | Some(CaptureTarget::Region { display_id, .. }) => display_id,
                _ => recording::primary_display(core)?.id,
            };
            let picked = tauri::async_runtime::block_on(region::select_region(app.clone(), core.clone(), display_id.clone()))?;
            match picked {
                Some(rect) => take_screenshot(app, core, default_request(core, CaptureTarget::Region { display_id, rect }), false).map(|_| ()),
                None => Ok(()),
            }
        }
    }
}

fn default_request(core: &Shared, target: CaptureTarget) -> ScreenshotRequest {
    let s = core.settings();
    ScreenshotRequest {
        target,
        format: s.screenshot_format,
        jpeg_quality: s.jpeg_quality,
        output_directory: None,
        file_name_template: Some(s.file_name_template.clone()),
        show_cursor: s.video.show_cursor,
        delay_sec: 0,
    }
}

/// Blocking: captures, validates (engine), stores in the library and emits `screenshot`.
pub fn take_screenshot(app: &AppHandle, core: &Shared, mut req: ScreenshotRequest, hide_main: bool) -> CaptureResult<MediaMetadata> {
    let engine = core.engine()?;
    let dir = req.output_directory.clone().filter(|d| !d.trim().is_empty()).map(PathBuf::from).unwrap_or_else(|| core.output_dir());
    std::fs::create_dir_all(&dir)?;
    if req.file_name_template.is_none() {
        req.file_name_template = Some(core.settings().file_name_template);
    }
    if req.delay_sec > 0 {
        std::thread::sleep(Duration::from_secs(req.delay_sec.min(30) as u64));
    }
    let hidden = hide_main && !matches!(req.target, CaptureTarget::Window { .. }) && windows::hide_main(app);
    if hidden {
        std::thread::sleep(Duration::from_millis(350));
    }
    let result = engine.screenshot(&req, &dir);
    if hidden {
        windows::show_main(app);
    }
    let media = result?;
    let stored = core.library.lock().upsert(&media)?;
    *core.last_target.lock() = Some(req.target.clone());
    emit_capture(app, CaptureEvent::Screenshot { media: stored.clone() });
    Ok(stored)
}

/// Blocking stop; returns the stored library item.
pub fn stop(core: &Shared, ctl: &Arc<SessionCtl>) -> CaptureResult<MediaMetadata> {
    let session = ctl.engine_session().ok_or_else(|| CaptureError::invalid_state("the recording has not started yet (still counting down)"))?;
    let media = session.stop()?;
    Ok(core.library.lock().get(&media.id)?.unwrap_or(media))
}

/// Cancels a countdown or discards an active recording (blocking for the latter).
pub fn cancel(ctl: &Arc<SessionCtl>) -> CaptureResult<()> {
    ctl.cancel_requested.store(true, Ordering::SeqCst);
    if let Some(tx) = ctl.cancel_tx.lock().take() {
        let _ = tx.send(());
        return Ok(());
    }
    match ctl.engine_session() {
        Some(s) => s.cancel(),
        // Between countdown and engine start: the start thread sees the flag and cancels.
        None => Ok(()),
    }
}

/// Quit from the tray: finalize an active recording first so no file is lost.
pub fn quit(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new().name("captureone-quit".into()).spawn(move || {
        let core = app.state::<Shared>().inner().clone();
        if let Some(ctl) = core.active() {
            match ctl.phase() {
                RecordingState::Recording | RecordingState::Paused | RecordingState::Starting => {
                    if let Err(e) = stop(&core, &ctl) {
                        log::warn!("stop on quit failed: {e}");
                    }
                }
                _ => {
                    let _ = cancel(&ctl);
                }
            }
        }
        app.exit(0);
    });
}
