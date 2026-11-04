//! `CaptureBackend` implementation for X11.

use crate::pulse;
use crate::x11::{format_window_id, parse_window_id, WinState, XRect, WHOLE_SCREEN_ID, X11};
use capture_core::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct LinuxBackend;

#[derive(Debug, Clone)]
enum Resolved {
    /// Fixed rect of a display (`display_id` is re-checked periodically) or region.
    Area { rect: XRect, display_id: String, region: Option<Rect> },
    Window(u32),
}

fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_else(|| "Linux".into())
}

fn os_version() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default()
}

fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
}

/// Rect of a display id ("all" = whole root).
fn display_rect(x: &X11, id: &str) -> CaptureResult<XRect> {
    if id == WHOLE_SCREEN_ID {
        let (w, h) = x.root_size()?;
        return Ok(XRect { x: 0, y: 0, w, h });
    }
    x.monitors()
        .into_iter()
        .find(|m| m.id == id)
        .map(|m| m.rect)
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("display '{id}' not found")))
}

/// Region rect (relative to the display) clamped into the display, in root coordinates.
fn region_rect(display: XRect, r: Rect) -> CaptureResult<XRect> {
    let x0 = r.x.max(0) as i64;
    let y0 = r.y.max(0) as i64;
    let x1 = (r.x as i64 + r.width as i64).min(display.w as i64);
    let y1 = (r.y as i64 + r.height as i64).min(display.h as i64);
    if x1 <= x0 || y1 <= y0 {
        return Err(CaptureError::invalid_config("region is outside the display"));
    }
    Ok(XRect { x: display.x + x0 as i32, y: display.y + y0 as i32, w: (x1 - x0) as u32, h: (y1 - y0) as u32 })
}

fn resolve(x: &X11, target: &CaptureTarget) -> CaptureResult<Resolved> {
    match target {
        CaptureTarget::Display { display_id } => Ok(Resolved::Area { rect: display_rect(x, display_id)?, display_id: display_id.clone(), region: None }),
        CaptureTarget::Region { display_id, rect } => {
            let d = display_rect(x, display_id)?;
            Ok(Resolved::Area { rect: region_rect(d, *rect)?, display_id: display_id.clone(), region: Some(*rect) })
        }
        CaptureTarget::Window { window_id } => {
            let w = parse_window_id(window_id).ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("invalid window id '{window_id}'")))?;
            match x.window_state(w) {
                WinState::Gone => Err(CaptureError::new(CaptureErrorCode::SourceNotFound, format!("window {window_id} not found"))),
                _ => Ok(Resolved::Window(w)),
            }
        }
        CaptureTarget::Device { .. } => Err(CaptureError::unsupported("capture devices are not supported by the X11 backend")),
    }
}

fn grab_once(x: &X11, shm: Option<&crate::x11::ShmSeg>, r: &Resolved, show_cursor: bool) -> CaptureResult<Option<VideoFrame>> {
    match r {
        Resolved::Area { rect, .. } => x.grab(shm, *rect, show_cursor).map(Some),
        Resolved::Window(w) => match x.window_state(*w) {
            WinState::Visible(rect) => match x.grab(shm, rect, show_cursor) {
                Ok(f) => Ok(Some(f)),
                // closed between the geometry query and the grab
                Err(e) if matches!(x.window_state(*w), WinState::Gone) => Err(CaptureError::new(CaptureErrorCode::SourceClosed, format!("window closed: {}", e.message))),
                Err(e) => Err(e),
            },
            WinState::Hidden => Ok(None),
            WinState::Gone => Err(CaptureError::new(CaptureErrorCode::SourceClosed, "window closed")),
        },
    }
}

struct X11Stream {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StreamHandle for X11Stream {
    fn stop(&mut self) -> CaptureResult<()> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        Ok(())
    }
}

impl Drop for X11Stream {
    fn drop(&mut self) {
        let _ = StreamHandle::stop(self);
    }
}

fn stream_loop(x: X11, mut target: Resolved, fps: u32, show_cursor: bool, sink: Arc<dyn VideoSink>, stop: Arc<AtomicBool>) {
    let shm = x.root_size().ok().and_then(|(w, h)| x.create_shm(w as usize * h as usize * 4));
    log::info!("X11 stream: {fps} fps, MIT-SHM {}", if shm.is_some() { "on" } else { "off" });
    let interval = Duration::from_secs_f64(1.0 / fps.clamp(1, 240) as f64);
    let mut next = Instant::now();
    let mut last_size: Option<(u32, u32)> = None;
    let mut last_check = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        // Displays can be unplugged / reconfigured: re-resolve about once per second.
        if last_check.elapsed() >= Duration::from_secs(1) {
            last_check = Instant::now();
            if let Resolved::Area { display_id, region, rect } = &mut target {
                match display_rect(&x, display_id) {
                    Ok(d) => {
                        if let Ok(r) = region.map_or(Ok(d), |reg| region_rect(d, reg)) {
                            *rect = r;
                        }
                    }
                    Err(e) if e.code == CaptureErrorCode::SourceNotFound => {
                        sink.on_event(StreamEvent::SourceClosed);
                        return;
                    }
                    Err(_) => {}
                }
            }
        }
        match grab_once(&x, shm.as_ref(), &target, show_cursor) {
            Ok(Some(frame)) => {
                let size = (frame.width, frame.height);
                if last_size.is_some() && last_size != Some(size) {
                    sink.on_event(StreamEvent::Resized { width: size.0, height: size.1 });
                }
                last_size = Some(size);
                sink.on_frame(frame);
            }
            Ok(None) => {} // window not viewable: no new frame (the encoder repeats the last one)
            Err(e) if e.code == CaptureErrorCode::SourceClosed => {
                sink.on_event(StreamEvent::SourceClosed);
                return;
            }
            Err(e) => {
                sink.on_event(StreamEvent::Error(e));
                return;
            }
        }
        next += interval;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now; // fell behind: do not try to catch up with a burst
        }
    }
}

impl CaptureBackend for LinuxBackend {
    fn platform(&self) -> Platform {
        Platform::Linux
    }

    fn capabilities(&self) -> CapabilityReport {
        let mut features = BTreeMap::new();
        let mut notes = Vec::new();
        let mut max_res = None;
        match X11::connect() {
            Ok(x) => {
                let xwayland = wayland_session();
                let base = if xwayland {
                    FeatureSupport::with(SupportLevel::Partial, "running under XWayland: only X11 applications are captured; Wayland-native windows and the desktop may appear black")
                } else {
                    FeatureSupport::native()
                };
                for k in [FeatureKey::Screenshot, FeatureKey::DisplayCapture, FeatureKey::RegionCapture, FeatureKey::VideoRecording] {
                    features.insert(k, base.clone());
                }
                features.insert(
                    FeatureKey::WindowCapture,
                    FeatureSupport::with(SupportLevel::Partial, "captures the window's on-screen area: overlapping windows are captured as seen; minimized windows produce no new frames"),
                );
                features.insert(
                    FeatureKey::MultiDisplay,
                    if x.has_randr { base.clone() } else { FeatureSupport::unsupported("RandR 1.5 is unavailable: only the whole screen can be captured") },
                );
                features.insert(FeatureKey::CursorCapture, if x.has_xfixes { FeatureSupport::native() } else { FeatureSupport::unsupported("XFixes extension unavailable") });
                if let Ok((w, h)) = x.root_size() {
                    max_res = Some(Size { width: w, height: h });
                }
                notes.push(format!(
                    "X11 backend: MIT-SHM {}, RandR 1.5 {}, XFixes {}",
                    if x.has_shm { "available" } else { "unavailable (slower GetImage)" },
                    if x.has_randr { "available" } else { "unavailable" },
                    if x.has_xfixes { "available" } else { "unavailable" }
                ));
            }
            Err(e) => {
                let reason = if wayland_session() {
                    "Wayland session without XWayland: X11 capture is unavailable (PipeWire portal capture is not implemented)".to_string()
                } else {
                    e.message.clone()
                };
                for k in [
                    FeatureKey::Screenshot,
                    FeatureKey::DisplayCapture,
                    FeatureKey::WindowCapture,
                    FeatureKey::RegionCapture,
                    FeatureKey::MultiDisplay,
                    FeatureKey::VideoRecording,
                    FeatureKey::CursorCapture,
                ] {
                    features.insert(k, FeatureSupport::unsupported(reason.clone()));
                }
            }
        }
        features.insert(
            FeatureKey::SystemAudio,
            match pulse::probe() {
                Ok(()) => FeatureSupport::with(SupportLevel::Partial, "records the monitor of the default output; requires a PulseAudio or PipeWire-pulse server"),
                Err(e) => FeatureSupport::unsupported(format!("no PulseAudio/PipeWire-pulse server reachable ({e})")),
            },
        );
        CapabilityReport {
            schema_version: 1,
            platform: Platform::Linux,
            os_name: os_name(),
            os_version: os_version(),
            device_model: None,
            app_version: APP_VERSION.into(),
            generated_at: now_iso8601(),
            features,
            encoders: Vec::new(),
            frame_rates: vec![24, 30, 60],
            screenshot_formats: vec![ImageFormat::Png, ImageFormat::Jpeg],
            max_recording_resolution: max_res,
            notes,
        }
    }

    fn permission_status(&self, kind: PermissionKind) -> PermissionStatus {
        let mut s = PermissionStatus::new(kind, PermissionState::NotApplicable);
        if matches!(kind, PermissionKind::ScreenCapture) {
            s.message = Some("X11 does not gate screen capture behind a permission".into());
        }
        s
    }

    fn request_permission(&self, kind: PermissionKind) -> CaptureResult<PermissionStatus> {
        Ok(self.permission_status(kind))
    }

    fn list_sources(&self, kinds: &[SourceKind]) -> CaptureResult<Vec<CaptureSource>> {
        let x = X11::connect()?;
        let mut out = Vec::new();
        if kinds.contains(&SourceKind::Display) {
            let monitors = x.monitors();
            for m in &monitors {
                out.push(CaptureSource {
                    id: m.id.clone(),
                    kind: SourceKind::Display,
                    name: format!("{} ({}x{})", m.id, m.rect.w, m.rect.h),
                    width: m.rect.w,
                    height: m.rect.h,
                    x: Some(m.rect.x),
                    y: Some(m.rect.y),
                    scale_factor: Some(1.0),
                    is_primary: Some(m.primary || monitors.len() == 1),
                    app_name: None,
                    device_descriptor: None,
                });
            }
            let (w, h) = x.root_size()?;
            out.push(CaptureSource {
                id: WHOLE_SCREEN_ID.into(),
                kind: SourceKind::Display,
                name: format!("Entire screen ({w}x{h})"),
                width: w,
                height: h,
                x: Some(0),
                y: Some(0),
                scale_factor: Some(1.0),
                is_primary: Some(monitors.is_empty()),
                app_name: None,
                device_descriptor: None,
            });
        }
        if kinds.contains(&SourceKind::Window) {
            for w in x.windows() {
                out.push(CaptureSource {
                    id: format_window_id(w.id),
                    kind: SourceKind::Window,
                    name: w.title,
                    width: w.rect.w,
                    height: w.rect.h,
                    x: Some(w.rect.x),
                    y: Some(w.rect.y),
                    scale_factor: Some(1.0),
                    is_primary: None,
                    app_name: w.class,
                    device_descriptor: None,
                });
            }
        }
        Ok(out)
    }

    fn screenshot(&self, target: &CaptureTarget, show_cursor: bool) -> CaptureResult<VideoFrame> {
        let x = X11::connect()?;
        let r = resolve(&x, target)?;
        grab_once(&x, None, &r, show_cursor)?.ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "window is not visible (minimized or unmapped)"))
    }

    fn start_video(&self, target: &CaptureTarget, opts: &VideoStreamOptions, sink: Arc<dyn VideoSink>) -> CaptureResult<Box<dyn StreamHandle>> {
        let x = X11::connect()?;
        let r = resolve(&x, target)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (fps, cursor, stop2) = (opts.fps, opts.show_cursor, stop.clone());
        let thread = std::thread::Builder::new()
            .name("x11-capture".into())
            .spawn(move || stream_loop(x, r, fps, cursor, sink, stop2))
            .map_err(|e| CaptureError::internal(format!("cannot spawn capture thread: {e}")))?;
        Ok(Box::new(X11Stream { stop, thread: Some(thread) }))
    }

    fn start_system_audio(&self, opts: &AudioStreamOptions, sink: Arc<dyn AudioSink>) -> CaptureResult<Box<dyn StreamHandle>> {
        Ok(Box::new(pulse::start(opts, sink)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_is_clamped_into_display() {
        let d = XRect { x: 1920, y: 0, w: 1280, h: 720 };
        let r = region_rect(d, Rect { x: 100, y: 50, width: 400, height: 300 }).unwrap();
        assert_eq!(r, XRect { x: 2020, y: 50, w: 400, h: 300 });
        let c = region_rect(d, Rect { x: 1200, y: 700, width: 400, height: 300 }).unwrap();
        assert_eq!((c.w, c.h), (80, 20));
        assert!(region_rect(d, Rect { x: 2000, y: 0, width: 10, height: 10 }).is_err());
    }
}
