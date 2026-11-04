//! X11 capture backend (reference/verification backend). Implementation in progress.

#[cfg(target_os = "linux")]
pub fn create_backend() -> capture_core::CaptureResult<std::sync::Arc<dyn capture_core::CaptureBackend>> {
    Err(capture_core::CaptureError::internal("not implemented yet"))
}
