//! Screenshots.
//!
//! * macOS 14+: `SCScreenshotManager captureImageWithFilter:configuration:completionHandler:`
//!   → `CGImage` → BGRA (drawn into an sRGB bitmap context).
//! * macOS 12.3–13 (or with `CAPTUREONE_MACOS_FORCE_STREAM_SCREENSHOT=1`): start an `SCStream`,
//!   take the first complete frame, stop.
//!
//! Regions are captured as the full display and cropped in pixels with `VideoFrame::crop`, so
//! the result is pixel-exact regardless of the display's backing scale.

use super::stream::{plan, start_video_plan, VideoPlan};
use super::util::{capture_error_from_ns, wait, SharedCf};
use super::{content, sample};
use crate::capabilities::{screenshot_strategy, OsVersion, ScreenshotStrategy};
use crate::errors::timeout_error;
use crate::FORCE_STREAM_SCREENSHOT_ENV;
use block2::RcBlock;
use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, CaptureTarget, StreamEvent, StreamHandle, VideoFrame, VideoSink};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_foundation::NSError;
use objc2_screen_capture_kit::SCScreenshotManager;
use std::ptr::NonNull;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(10);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

fn force_stream() -> bool {
    std::env::var(FORCE_STREAM_SCREENSHOT_ENV).is_ok_and(|v| !v.is_empty() && v != "0")
}

pub(crate) fn capture(os: OsVersion, target: &CaptureTarget, show_cursor: bool) -> CaptureResult<VideoFrame> {
    let strategy = screenshot_strategy(os, force_stream());
    if strategy == ScreenshotStrategy::Unsupported {
        return Err(CaptureError::unsupported(format!("ScreenCaptureKit requires macOS 12.3 (this Mac runs {os})")));
    }
    let content = content::shareable_content(false)?;
    let plan = plan(os, &content, target, false)?;
    log::debug!("macos-capture: screenshot {}x{} via {strategy:?}", plan.width, plan.height);
    let frame = match strategy {
        ScreenshotStrategy::ScreenshotManager => screenshot_manager(os, &plan, show_cursor)?,
        _ => single_frame_stream(os, &plan, show_cursor)?,
    };
    match plan.crop {
        Some(rect) => frame.crop(rect),
        None => Ok(frame),
    }
}

fn screenshot_manager(os: OsVersion, plan: &VideoPlan, show_cursor: bool) -> CaptureResult<VideoFrame> {
    let config = super::stream::video_config(os, plan, 1, show_cursor, 3);
    let (tx, rx) = mpsc::sync_channel::<CaptureResult<SharedCf<CGImage>>>(1);
    // Runs on an arbitrary SCK queue; the image is only borrowed for the call, so retain it.
    let handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        // SAFETY: SCK passes null or valid objects.
        let result = unsafe {
            if let Some(err) = error.as_ref() {
                Err(capture_error_from_ns(err))
            } else if let Some(image) = NonNull::new(image) {
                Ok(SharedCf(CFRetained::retain(image)))
            } else {
                Err(CaptureError::internal("SCScreenshotManager returned neither an image nor an error"))
            }
        };
        let _ = tx.try_send(result);
    });
    // SAFETY: macOS 14+ only (strategy gate); valid filter/config and a heap block.
    unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&plan.filter, &config, Some(&handler)) };
    let image = wait(&rx, SCREENSHOT_TIMEOUT, "SCScreenshotManager")??;
    sample::cgimage_to_frame(&image.0)
}

/// Sink that forwards the first frame (or terminal event) of a stream into a channel.
struct FirstFrameSink {
    tx: Mutex<Option<SyncSender<CaptureResult<VideoFrame>>>>,
}

impl FirstFrameSink {
    fn send(&self, value: CaptureResult<VideoFrame>) {
        if let Some(tx) = self.tx.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = tx.try_send(value);
        }
    }
}

impl VideoSink for FirstFrameSink {
    fn on_frame(&self, frame: VideoFrame) {
        self.send(Ok(frame));
    }
    fn on_event(&self, event: StreamEvent) {
        match event {
            StreamEvent::SourceClosed => self.send(Err(CaptureError::new(CaptureErrorCode::SourceClosed, "the source disappeared during the screenshot"))),
            StreamEvent::Error(e) => self.send(Err(e)),
            StreamEvent::Resized { .. } => {}
        }
    }
}

fn single_frame_stream(os: OsVersion, plan: &VideoPlan, show_cursor: bool) -> CaptureResult<VideoFrame> {
    let (tx, rx) = mpsc::sync_channel(1);
    let sink = Arc::new(FirstFrameSink { tx: Mutex::new(Some(tx)) });
    let mut stream = start_video_plan(os, plan, 30, show_cursor, sink, false)?;
    let result = rx.recv_timeout(FIRST_FRAME_TIMEOUT);
    if let Err(e) = stream.stop() {
        log::warn!("macos-capture: stopping the screenshot stream failed: {}", e.message);
    }
    match result {
        Ok(frame) => frame,
        Err(RecvTimeoutError::Timeout) => Err(timeout_error("the first complete ScreenCaptureKit frame", FIRST_FRAME_TIMEOUT.as_secs())),
        Err(RecvTimeoutError::Disconnected) => Err(CaptureError::internal("the screenshot stream ended without a frame")),
    }
}
