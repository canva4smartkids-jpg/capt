//! `select_region`: opens the overlay and resolves when the overlay calls
//! `region_selection_done` (or is closed).

use crate::state::Shared;
use crate::windows;
use capture_core::*;
use std::time::Duration;
use tauri::AppHandle;

/// Clamps a rect to the display; None when nothing usable remains (< 16 px).
pub fn clamp_to_display(r: Rect, width: u32, height: u32) -> Option<Rect> {
    let x0 = r.x.clamp(0, width as i32);
    let y0 = r.y.clamp(0, height as i32);
    let x1 = (r.x as i64 + r.width as i64).clamp(0, width as i64) as i32;
    let y1 = (r.y as i64 + r.height as i64).clamp(0, height as i64) as i32;
    let (w, h) = ((x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32);
    (w >= 16 && h >= 16).then_some(Rect { x: x0, y: y0, width: w, height: h })
}

pub async fn select_region(app: AppHandle, core: Shared, display_id: String) -> CaptureResult<Option<Rect>> {
    let backend = core.backend()?;
    let id = display_id.clone();
    let display = tauri::async_runtime::spawn_blocking(move || find_source(backend.as_ref(), SourceKind::Display, &id))
        .await
        .map_err(|e| CaptureError::internal(e.to_string()))??;
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut slot = core.region.lock();
        if slot.is_some() {
            return Err(CaptureError::new(CaptureErrorCode::DeviceBusy, "a region selection is already open"));
        }
        *slot = Some(tx);
    }
    let main_was_visible = windows::hide_main(&app);
    if main_was_visible {
        // Give the compositor time to repaint without our window before the frozen frame is taken.
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
    if let Err(e) = windows::open_region(&app, &display) {
        core.region.lock().take();
        if main_was_visible {
            windows::show_main(&app);
        }
        return Err(e);
    }
    let picked = rx.await.unwrap_or(None);
    windows::close_region(&app);
    if main_was_visible {
        windows::show_main(&app);
    }
    Ok(picked.and_then(|r| clamp_to_display(r, display.width, display.height)))
}

/// Called by the overlay. Errors if no selection is pending.
pub fn finish(core: &Shared, rect: Option<Rect>) -> CaptureResult<()> {
    let tx = core.region.lock().take().ok_or_else(|| CaptureError::invalid_state("no region selection in progress"))?;
    let _ = tx.send(rect);
    Ok(())
}

/// The overlay window went away without answering.
pub fn abandon(core: &Shared) {
    if let Some(tx) = core.region.lock().take() {
        let _ = tx.send(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clamps_and_rejects_tiny_regions() {
        let r = |x, y, width, height| Rect { x, y, width, height };
        assert_eq!(clamp_to_display(r(100, 50, 800, 600), 1920, 1080), Some(r(100, 50, 800, 600)));
        assert_eq!(clamp_to_display(r(-10, -10, 100, 100), 1920, 1080), Some(r(0, 0, 90, 90)));
        assert_eq!(clamp_to_display(r(1900, 1000, 500, 500), 1920, 1080), Some(r(1900, 1000, 20, 80)));
        assert_eq!(clamp_to_display(r(1910, 0, 500, 500), 1920, 1080), None);
        assert_eq!(clamp_to_display(r(5000, 5000, 10, 10), 1920, 1080), None);
    }
}
