//! ScreenCaptureKit implementation of `CaptureBackend` (compiled only for macOS).

mod content;
mod host;
mod permission;
mod sample;
mod screenshot;
mod stream;
mod util;

use crate::capabilities::{capability_report, supports_screen_capture_kit, OsVersion};
use crate::permissions::{microphone_status, other_status, privacy_settings_url, screen_status, screen_status_after_request};
use capture_core::{
    AudioSink, AudioStreamOptions, CapabilityReport, CaptureBackend, CaptureError, CaptureResult, CaptureSource,
    CaptureTarget, PermissionKind, PermissionStatus, Platform, SourceKind, StreamHandle, VideoFrame, VideoSink,
    VideoStreamOptions,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The ScreenCaptureKit backend.
pub struct MacCaptureBackend {
    os: OsVersion,
    /// `CGRequestScreenCaptureAccess` was already called by this process.
    screen_requested: AtomicBool,
}

impl MacCaptureBackend {
    /// Version of macOS this backend detected.
    pub fn os_version(&self) -> OsVersion {
        self.os
    }
}

pub(crate) fn create_backend() -> CaptureResult<Arc<dyn CaptureBackend>> {
    let os = host::os_version();
    if !supports_screen_capture_kit(os) {
        return Err(CaptureError::unsupported(format!("ScreenCaptureKit requires macOS 12.3 or newer (this Mac runs {os})"))
            .with_detail("minOsVersion", "12.3"));
    }
    // Make sure this process has a CoreGraphics window-server connection before the first
    // ScreenCaptureKit call. GUI apps get one from AppKit; command-line tools (captureone-cli)
    // do not, and the first CG call establishes it.
    let main_display = objc2_core_graphics::CGMainDisplayID();
    log::info!("macos-capture: ScreenCaptureKit backend on macOS {os} (main display {main_display})");
    Ok(Arc::new(MacCaptureBackend { os, screen_requested: AtomicBool::new(false) }))
}

/// Opens System Settings at the privacy pane for `kind` (Screen & System Audio Recording or
/// Microphone) using `/usr/bin/open` with an `x-apple.systempreferences:` URL.
pub fn open_privacy_settings(kind: PermissionKind) -> CaptureResult<()> {
    let url = privacy_settings_url(kind).ok_or_else(|| CaptureError::unsupported(format!("macOS has no privacy settings pane for {kind:?}")))?;
    let status = std::process::Command::new("/usr/bin/open").arg(url).status().map_err(CaptureError::io)?;
    if status.success() {
        Ok(())
    } else {
        Err(CaptureError::internal(format!("`open {url}` failed with {status}")))
    }
}

impl CaptureBackend for MacCaptureBackend {
    fn platform(&self) -> Platform {
        Platform::Macos
    }

    fn capabilities(&self) -> CapabilityReport {
        capability_report(&host::host_info(self.os), capture_core::now_iso8601())
    }

    fn permission_status(&self, kind: PermissionKind) -> PermissionStatus {
        match kind {
            PermissionKind::ScreenCapture | PermissionKind::SystemAudio => {
                screen_status(kind, self.os, permission::screen_preflight(), self.screen_requested.load(Ordering::SeqCst))
            }
            PermissionKind::Microphone => microphone_status(permission::microphone_raw_status()),
            other => other_status(other, self.os),
        }
    }

    fn request_permission(&self, kind: PermissionKind) -> CaptureResult<PermissionStatus> {
        match kind {
            PermissionKind::ScreenCapture | PermissionKind::SystemAudio => {
                let current = self.permission_status(kind);
                if matches!(current.state, capture_core::PermissionState::Granted | capture_core::PermissionState::NotApplicable) {
                    return Ok(current);
                }
                let granted = permission::screen_request();
                self.screen_requested.store(true, Ordering::SeqCst);
                Ok(screen_status_after_request(kind, self.os, granted))
            }
            PermissionKind::Microphone => {
                permission::request_microphone()?;
                Ok(microphone_status(permission::microphone_raw_status()))
            }
            other => Ok(other_status(other, self.os)),
        }
    }

    fn list_sources(&self, kinds: &[SourceKind]) -> CaptureResult<Vec<CaptureSource>> {
        content::list_sources(kinds)
    }

    fn screenshot(&self, target: &CaptureTarget, show_cursor: bool) -> CaptureResult<VideoFrame> {
        screenshot::capture(self.os, target, show_cursor)
    }

    fn start_video(&self, target: &CaptureTarget, opts: &VideoStreamOptions, sink: Arc<dyn VideoSink>) -> CaptureResult<Box<dyn StreamHandle>> {
        if opts.fps == 0 {
            return Err(CaptureError::invalid_config("fps must be positive"));
        }
        stream::start_video(self.os, target, opts.fps, opts.show_cursor, sink)
    }

    fn start_system_audio(&self, opts: &AudioStreamOptions, sink: Arc<dyn AudioSink>) -> CaptureResult<Box<dyn StreamHandle>> {
        if opts.device_id.is_some() {
            log::info!("macos-capture: system audio ignores device_id (ScreenCaptureKit captures the system mix)");
        }
        stream::start_system_audio(self.os, opts.sample_rate, opts.channels, sink)
    }
}
