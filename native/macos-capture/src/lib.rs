//! macOS capture backend for CaptureOne, built on **ScreenCaptureKit** (macOS 12.3+).
//!
//! * [`create_backend`] (macOS only) returns the [`capture_core::CaptureBackend`] implementation.
//! * [`open_privacy_settings`] (macOS only) opens the matching System Settings privacy pane.
//!
//! The crate is split in two layers:
//!
//! * Platform-independent logic (PCM layout conversion, BGRA row packing, points/pixels
//!   geometry, OS-version capability gating, permission/error mapping, timing). These modules
//!   compile and are unit-tested on every OS, including the Linux CI/dev containers.
//! * `macos` (only compiled for `target_os = "macos"`): the Objective-C bridge to
//!   ScreenCaptureKit, CoreMedia, CoreVideo, CoreGraphics and AVFoundation via the `objc2`
//!   crates.
//!
//! On other operating systems the crate exposes only the helpers, so the workspace builds
//! everywhere.

pub mod audio;
pub mod capabilities;
pub mod errors;
pub mod geometry;
pub mod permissions;
pub mod pixels;
pub mod timing;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{open_privacy_settings, MacCaptureBackend};

/// Creates the ScreenCaptureKit backend.
///
/// Fails with `UNSUPPORTED` on macOS versions older than 12.3 (ScreenCaptureKit is not
/// available there).
#[cfg(target_os = "macos")]
pub fn create_backend() -> capture_core::CaptureResult<std::sync::Arc<dyn capture_core::CaptureBackend>> {
    macos::create_backend()
}

/// Environment variable that forces the single-frame `SCStream` screenshot path (the macOS
/// 12.3–13 implementation) even on macOS 14+, so CI on current runners can exercise it.
pub const FORCE_STREAM_SCREENSHOT_ENV: &str = "CAPTUREONE_MACOS_FORCE_STREAM_SCREENSHOT";
