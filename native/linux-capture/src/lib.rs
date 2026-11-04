//! X11 capture backend for CaptureOne (reference/verification backend).
//!
//! * displays: RandR 1.5 monitors (id = monitor name, e.g. "DP-1") plus the whole screen
//!   (id "all"); windows: `_NET_CLIENT_LIST(_STACKING)` (or top-level windows without an
//!   EWMH window manager), id = hex XID;
//! * frames: MIT-SHM `GetImage` of the root window (plain `GetImage` fallback), BGRA with
//!   opaque alpha, XFixes cursor overlay; window capture grabs the window's on-screen
//!   rectangle, so overlapping windows are captured as seen;
//! * system audio: PulseAudio / PipeWire-pulse monitor of the default output.
//!
//! Wayland-only sessions (no XWayland) are reported as unsupported.

#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "linux")]
mod pulse;
#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub use backend::LinuxBackend;

/// Creates the X11 backend. Connection problems surface as `unsupported` capabilities and
/// errors from the capture calls, so the app can still show a capability report.
#[cfg(target_os = "linux")]
pub fn create_backend() -> capture_core::CaptureResult<std::sync::Arc<dyn capture_core::CaptureBackend>> {
    Ok(std::sync::Arc::new(LinuxBackend))
}
