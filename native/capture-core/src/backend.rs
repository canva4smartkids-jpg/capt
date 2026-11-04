//! The CaptureBackend trait: implemented once per OS with that OS's native capture API
//! (Windows Graphics Capture, ScreenCaptureKit, X11). Encoding is NOT a backend concern —
//! backends deliver raw frames/PCM to sinks and `media-engine` encodes them.

use crate::frame::{AudioBuffer, VideoFrame};
use crate::types::*;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Captured window closed / display disconnected. The stream has ended.
    SourceClosed,
    /// Source size changed; subsequent frames use the new size.
    Resized { width: u32, height: u32 },
    /// Fatal stream error. The stream has ended.
    Error(CaptureError),
}

/// Sinks are called on backend-owned threads (capture/dispatch queues) and must return
/// quickly without blocking; heavy work belongs on the consumer's own threads. Calling
/// `StreamHandle::stop` from inside a sink callback is allowed (backends must not deadlock).
pub trait VideoSink: Send + Sync + 'static {
    fn on_frame(&self, frame: VideoFrame);
    fn on_event(&self, event: StreamEvent);
}

pub trait AudioSink: Send + Sync + 'static {
    fn on_audio(&self, buffer: AudioBuffer);
    fn on_event(&self, event: StreamEvent);
}

/// A running capture stream. Dropping without `stop` must also release OS resources.
pub trait StreamHandle: Send {
    fn stop(&mut self) -> CaptureResult<()>;
}

#[derive(Debug, Clone)]
pub struct VideoStreamOptions {
    /// Desired delivery rate. Backends may deliver fewer frames (e.g. WGC only on change);
    /// the encoder's pacer duplicates frames to maintain constant frame rate.
    pub fps: u32,
    pub show_cursor: bool,
}

#[derive(Debug, Clone)]
pub struct AudioStreamOptions {
    pub sample_rate: u32,
    pub channels: u16,
    /// Platform device id. `None`, `"default"` and `"system-default"` all mean the default
    /// output (loopback) / input device.
    pub device_id: Option<String>,
}

pub trait CaptureBackend: Send + Sync {
    fn platform(&self) -> Platform;

    /// Capture-side capabilities (encoder list is filled in by media-engine).
    fn capabilities(&self) -> CapabilityReport;

    fn permission_status(&self, kind: PermissionKind) -> PermissionStatus;

    /// May show a system prompt. Returns the resulting status.
    fn request_permission(&self, kind: PermissionKind) -> CaptureResult<PermissionStatus>;

    fn list_sources(&self, kinds: &[SourceKind]) -> CaptureResult<Vec<CaptureSource>>;

    /// Single frame of a display, window or region (regions may be cropped with `VideoFrame::crop`).
    fn screenshot(&self, target: &CaptureTarget, show_cursor: bool) -> CaptureResult<VideoFrame>;

    /// Starts delivering frames for the target to `sink` until the handle is stopped.
    fn start_video(&self, target: &CaptureTarget, opts: &VideoStreamOptions, sink: Arc<dyn VideoSink>) -> CaptureResult<Box<dyn StreamHandle>>;

    /// System/loopback audio. Default: unsupported.
    fn start_system_audio(&self, _opts: &AudioStreamOptions, _sink: Arc<dyn AudioSink>) -> CaptureResult<Box<dyn StreamHandle>> {
        Err(CaptureError::unsupported("system audio capture is not supported on this platform"))
    }
}

/// Looks up a source by id in `list_sources` output.
pub fn find_source(backend: &dyn CaptureBackend, kind: SourceKind, id: &str) -> CaptureResult<CaptureSource> {
    backend
        .list_sources(&[kind])?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("{kind:?} '{id}' not found")))
}
