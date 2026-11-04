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

/// Deep link into the OS privacy settings for a permission, if the OS has one.
pub fn permission_settings_url(kind: PermissionKind) -> CaptureResult<&'static str> {
    let unsupported = || Err(CaptureError::unsupported("This system has no settings page for that permission."));
    if cfg!(target_os = "macos") {
        return match kind {
            PermissionKind::ScreenCapture | PermissionKind::SystemAudio => Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"),
            PermissionKind::Microphone => Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"),
            PermissionKind::LocalNetwork => Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork"),
            PermissionKind::Notifications => Ok("x-apple.systempreferences:com.apple.preference.notifications"),
            PermissionKind::Storage => unsupported(),
        };
    }
    if cfg!(target_os = "windows") {
        return match kind {
            PermissionKind::Microphone => Ok("ms-settings:privacy-microphone"),
            PermissionKind::ScreenCapture => Ok("ms-settings:privacy-graphicscaptureprogrammatic"),
            PermissionKind::Notifications => Ok("ms-settings:notifications"),
            _ => unsupported(),
        };
    }
    unsupported()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_links_are_platform_specific() {
        let r = permission_settings_url(PermissionKind::Microphone);
        if cfg!(target_os = "macos") {
            assert!(r.unwrap().contains("Privacy_Microphone"));
        } else if cfg!(target_os = "windows") {
            assert_eq!(r.unwrap(), "ms-settings:privacy-microphone");
        } else {
            assert_eq!(r.unwrap_err().code, CaptureErrorCode::Unsupported);
        }
    }
}
