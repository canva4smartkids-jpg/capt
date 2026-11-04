//! Small Objective-C bridging helpers shared by the macOS modules.

use crate::errors::{map_ns_error, timeout_error};
use capture_core::{CaptureError, CaptureResult};
use objc2::rc::Retained;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCShareableContent, SCStream, SCStreamConfiguration, SCWindow,
    SCStreamErrorDomain,
};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// An Objective-C object moved between threads (e.g. out of a ScreenCaptureKit completion
/// handler, which runs on an arbitrary SCK queue, into the waiting Rust thread).
///
/// `objc2` conservatively does not mark the ScreenCaptureKit classes `Send`. The wrapped types
/// below are immutable after creation (content snapshots, displays, windows, applications,
/// filters, configurations we no longer mutate) or internally synchronised (`SCStream`, whose
/// start/stop/output methods are documented to be callable from any thread and are invoked
/// with completion handlers on SCK's own queues). We additionally never use one instance from
/// two threads at the same time: ownership is handed over through a channel or kept behind
/// `&mut self`.
pub(crate) struct Shared<T: ?Sized>(pub(crate) Retained<T>);

macro_rules! shared_send {
    ($($t:ty),* $(,)?) => {
        $(
            // SAFETY: see the type-level comment on `Shared`.
            unsafe impl Send for Shared<$t> {}
        )*
    };
}
shared_send!(SCShareableContent, SCDisplay, SCWindow, SCRunningApplication, SCContentFilter, SCStreamConfiguration, SCStream);

/// A CoreFoundation object moved between threads. `CGImage` is immutable and documented as
/// thread-safe by Apple.
pub(crate) struct SharedCf<T: ?Sized + objc2_core_foundation::Type>(pub(crate) CFRetained<T>);
// SAFETY: CGImage instances are immutable; Apple documents Quartz images as thread-safe.
unsafe impl Send for SharedCf<CGImage> {}

/// Converts an `NSError` into a `CaptureError`, recognising ScreenCaptureKit's error domain.
pub(crate) fn capture_error_from_ns(err: &NSError) -> CaptureError {
    let domain = err.domain();
    // SAFETY: `SCStreamErrorDomain` is an immutable NSString constant exported by
    // ScreenCaptureKit (macOS 12.3+, which `create_backend` enforces).
    let is_sck = unsafe { SCStreamErrorDomain }.isEqualToString(&domain);
    map_ns_error(is_sck, &domain.to_string(), err.code(), &err.localizedDescription().to_string())
}

/// Waits for the value a completion handler sends.
pub(crate) fn wait<T>(rx: &Receiver<T>, timeout: Duration, what: &str) -> CaptureResult<T> {
    match rx.recv_timeout(timeout) {
        Ok(v) => Ok(v),
        Err(RecvTimeoutError::Timeout) => Err(timeout_error(what, timeout.as_secs())),
        Err(RecvTimeoutError::Disconnected) => Err(CaptureError::internal(format!(
            "{what}: the completion handler was released without being called"
        ))),
    }
}

/// Runs `f` on a new, detached thread (used to deliver stream events off ScreenCaptureKit's
/// queues, so a sink that calls `StreamHandle::stop` from `on_event` cannot deadlock).
pub(crate) fn spawn_detached(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.to_string()).spawn(f) {
        log::error!("macos-capture: could not spawn thread '{name}': {e}");
    }
}
