//! OS-specific bits: capture backend factory and system-settings deep links.

use capture_core::*;
use std::sync::Arc;

pub fn create_backend() -> CaptureResult<Arc<dyn CaptureBackend>> {
    #[cfg(target_os = "linux")]
    {
        linux_capture::create_backend()
    }
    #[cfg(target_os = "windows")]
    {
        windows_capture_backend::create_backend()
    }
    #[cfg(target_os = "macos")]
    {
        macos_capture::create_backend()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(CaptureError::unsupported("no capture backend for this OS"))
    }
}

/// Opens the OS privacy settings page for a permission (implemented by the backend crates).
pub fn open_privacy_settings(kind: PermissionKind) -> CaptureResult<()> {
    #[cfg(target_os = "macos")]
    {
        macos_capture::open_privacy_settings(kind)
    }
    #[cfg(target_os = "windows")]
    {
        windows_capture_backend::open_privacy_settings(kind)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = kind;
        Err(CaptureError::unsupported("Linux desktops have no standard privacy settings page; screen capture on X11 needs no permission."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_has_no_privacy_settings_page() {
        assert_eq!(open_privacy_settings(PermissionKind::Microphone).unwrap_err().code, CaptureErrorCode::Unsupported);
    }
}
