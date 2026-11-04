//! Windows capture backend for CaptureOne.
//!
//! * Video / screenshots: **Windows Graphics Capture** (WGC) via the `windows-capture` crate
//!   (session, dispatcher queue, frame pool) plus our own D3D11 staging-texture readback into
//!   BGRA `VideoFrame`s.
//! * System audio: **WASAPI loopback** on the default (or a chosen) render endpoint, delivered as
//!   interleaved f32 at the endpoint mix format.
//! * Sources: `EnumDisplayMonitors` / `EnumWindows` with DWM cloaking and tool-window filtering.
//!
//! On non-Windows targets only the platform-independent helpers below are compiled (they are
//! unit tested on every OS); `create_backend` exists only on Windows (engineering rules §4.1).

pub mod gating;
pub mod ids;
pub mod pcm;
pub mod pixels;
pub mod region;
pub mod timing;

#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "windows")]
pub use win::{create_backend, list_render_endpoints, open_privacy_settings, AudioEndpoint, WindowsBackend};
