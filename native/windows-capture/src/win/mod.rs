//! Windows implementation of `CaptureBackend`.

mod audio;
mod clock;
mod os;
mod sources;
mod video;

pub use audio::{list_render_endpoints, AudioEndpoint};
pub use os::open_privacy_settings;

use crate::gating::{self, AudioProbe, OsVersion, WgcFeatures};
use crate::region::CroppingSink;
use capture_core::{
    AudioSink, AudioStreamOptions, CapabilityReport, CaptureBackend, CaptureError, CaptureResult, CaptureSource, CaptureTarget, ImageFormat, PermissionKind,
    PermissionStatus, Platform, SourceKind, StreamHandle, VideoFrame, VideoSink, VideoStreamOptions,
};
use std::sync::Arc;
use video::CaptureItem;

/// Windows Graphics Capture + WASAPI loopback backend.
pub struct WindowsBackend {
    os: OsVersion,
    wgc: WgcFeatures,
}

/// Engineering rules §4.1 factory.
pub fn create_backend() -> CaptureResult<Arc<dyn CaptureBackend>> {
    Ok(Arc::new(WindowsBackend::new()))
}

impl WindowsBackend {
    pub fn new() -> Self {
        let os = os::os_version();
        let probe = video::probe_wgc();
        let wgc = gating::resolve_wgc(&os, &probe);
        log::info!("Windows backend: {} {} — WGC probe {probe:?} → {wgc:?}", os.name(), os.version_string());
        Self { os, wgc }
    }

    /// What the WGC session can be configured to do on this machine.
    pub fn wgc_features(&self) -> WgcFeatures {
        self.wgc
    }

    fn resolve_item(&self, target: &CaptureTarget) -> CaptureResult<(CaptureItem, Option<capture_core::Rect>)> {
        match target {
            CaptureTarget::Display { display_id } => Ok((CaptureItem::Monitor(sources::find_monitor(display_id)?.hmonitor), None)),
            CaptureTarget::Region { display_id, rect } => {
                if rect.width == 0 || rect.height == 0 {
                    return Err(CaptureError::invalid_config("region has zero size"));
                }
                let m = sources::find_monitor(display_id)?;
                let (w, h) = (m.source.width as i64, m.source.height as i64);
                let (x0, y0) = (rect.x as i64, rect.y as i64);
                if x0 >= w || y0 >= h || x0 + rect.width as i64 <= 0 || y0 + rect.height as i64 <= 0 {
                    return Err(CaptureError::invalid_config(format!(
                        "region {}x{}+{}+{} lies outside display {} ({}x{})",
                        rect.width, rect.height, rect.x, rect.y, m.source.id, w, h
                    )));
                }
                Ok((CaptureItem::Monitor(m.hmonitor), Some(*rect)))
            }
            CaptureTarget::Window { window_id } => {
                let hwnd = sources::find_window(window_id)?;
                if sources::is_minimized(hwnd) {
                    return Err(CaptureError::invalid_state(format!(
                        "window {window_id} is minimized; restore it to capture it"
                    )));
                }
                Ok((CaptureItem::Window(hwnd), None))
            }
            CaptureTarget::Device { .. } => Err(CaptureError::unsupported(
                "capture-card (device) sources are not supported by the Windows backend",
            )),
        }
    }
}

impl Default for WindowsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureBackend for WindowsBackend {
    fn platform(&self) -> Platform {
        Platform::Windows
    }

    fn capabilities(&self) -> CapabilityReport {
        let audio = AudioProbe {
            default_render: audio::default_render_name(),
        };
        let (features, notes) = gating::capture_features(&self.os, &self.wgc, &audio);
        CapabilityReport {
            schema_version: 1,
            platform: Platform::Windows,
            os_name: self.os.name(),
            os_version: self.os.version_string(),
            device_model: None,
            app_version: capture_core::APP_VERSION.to_string(),
            generated_at: capture_core::now_iso8601(),
            features,
            encoders: Vec::new(),
            frame_rates: vec![24, 30, 60],
            screenshot_formats: vec![ImageFormat::Png, ImageFormat::Jpeg],
            max_recording_resolution: None,
            notes,
        }
    }

    fn permission_status(&self, kind: PermissionKind) -> PermissionStatus {
        match kind {
            PermissionKind::Microphone => os::microphone_permission(),
            other => gating::static_permission(other),
        }
    }

    fn request_permission(&self, kind: PermissionKind) -> CaptureResult<PermissionStatus> {
        Ok(os::request_permission(kind))
    }

    fn list_sources(&self, kinds: &[SourceKind]) -> CaptureResult<Vec<CaptureSource>> {
        let mut out = Vec::new();
        for kind in kinds {
            match kind {
                SourceKind::Display => out.extend(sources::monitors()?.into_iter().map(|m| m.source)),
                SourceKind::Window => out.extend(sources::windows()?),
                // No capture-card support on Windows yet (reported as unsupported).
                SourceKind::Device => {}
            }
        }
        Ok(out)
    }

    fn screenshot(&self, target: &CaptureTarget, show_cursor: bool) -> CaptureResult<VideoFrame> {
        let (item, crop) = self.resolve_item(target)?;
        let frame = video::screenshot(item, &self.wgc, show_cursor)?;
        match crop {
            Some(rect) => frame.crop(rect),
            None => Ok(frame),
        }
    }

    fn start_video(&self, target: &CaptureTarget, opts: &VideoStreamOptions, sink: Arc<dyn VideoSink>) -> CaptureResult<Box<dyn StreamHandle>> {
        let (item, crop) = self.resolve_item(target)?;
        let sink: Arc<dyn VideoSink> = match crop {
            Some(rect) => Arc::new(CroppingSink::new(sink, rect)),
            None => sink,
        };
        let stream = video::start_stream(item, &self.wgc, opts.show_cursor, opts.fps, sink)?;
        Ok(Box::new(stream))
    }

    fn start_system_audio(&self, opts: &AudioStreamOptions, sink: Arc<dyn AudioSink>) -> CaptureResult<Box<dyn StreamHandle>> {
        let stream = audio::start_loopback(opts, sink)?;
        Ok(Box::new(stream))
    }
}
