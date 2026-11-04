//! Permission state logic (pure; the macOS module feeds it the raw OS answers).
//!
//! * Screen capture: `CGPreflightScreenCaptureAccess()` only answers granted / not granted.
//!   macOS shows the TCC prompt for `CGRequestScreenCaptureAccess()` **once per app**; after
//!   a denial (or once the prompt was shown) the user has to flip the switch in System Settings,
//!   and the process usually has to be relaunched before the grant is visible.
//! * System audio: ScreenCaptureKit audio is covered by the same Screen Recording grant
//!   (macOS 13+).
//! * Microphone: `AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio`.

use crate::capabilities::{supports_system_audio, OsVersion};
use crate::errors::SCREEN_PERMISSION_HINT;
use capture_core::{PermissionKind, PermissionState, PermissionStatus};

/// `AVAuthorizationStatus` raw values.
pub mod av_status {
    pub const NOT_DETERMINED: isize = 0;
    pub const RESTRICTED: isize = 1;
    pub const DENIED: isize = 2;
    pub const AUTHORIZED: isize = 3;
}

pub const SCREEN_CAPTURE_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
pub const MICROPHONE_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

/// System Settings deep link for a permission, if macOS has a pane for it.
pub fn privacy_settings_url(kind: PermissionKind) -> Option<&'static str> {
    match kind {
        PermissionKind::ScreenCapture | PermissionKind::SystemAudio => Some(SCREEN_CAPTURE_SETTINGS_URL),
        PermissionKind::Microphone => Some(MICROPHONE_SETTINGS_URL),
        _ => None,
    }
}

fn status(kind: PermissionKind, state: PermissionState, settings: bool, message: impl Into<String>) -> PermissionStatus {
    PermissionStatus { kind, state, requires_system_settings: settings, message: Some(message.into()) }
}

/// Screen capture / system audio status.
///
/// * `preflight`: result of `CGPreflightScreenCaptureAccess()`.
/// * `requested_this_session`: whether `CGRequestScreenCaptureAccess()` was already called by
///   this process (after that, a missing grant means the user has to use System Settings).
pub fn screen_status(kind: PermissionKind, os: OsVersion, preflight: bool, requested_this_session: bool) -> PermissionStatus {
    if kind == PermissionKind::SystemAudio && !supports_system_audio(os) {
        return status(kind, PermissionState::NotApplicable, false, format!("System audio capture requires macOS 13 (this Mac runs {os})"));
    }
    if preflight {
        let msg = if kind == PermissionKind::SystemAudio {
            "Granted through the Screen & System Audio Recording permission"
        } else {
            "Screen Recording permission granted"
        };
        return status(kind, PermissionState::Granted, false, msg);
    }
    if requested_this_session {
        status(kind, PermissionState::Denied, true, format!("Screen Recording permission is not granted. {SCREEN_PERMISSION_HINT}"))
    } else {
        status(
            kind,
            PermissionState::Prompt,
            false,
            "Not granted yet. Requesting shows the macOS prompt the first time only; if it was denied before, enable CaptureOne in System Settings → Privacy & Security → Screen & System Audio Recording.",
        )
    }
}

/// Status after calling `CGRequestScreenCaptureAccess()` (which returns immediately; the grant
/// itself normally only becomes effective after relaunching the app).
pub fn screen_status_after_request(kind: PermissionKind, os: OsVersion, granted: bool) -> PermissionStatus {
    if granted {
        screen_status(kind, os, true, true)
    } else {
        let mut s = screen_status(kind, os, false, true);
        if s.state == PermissionState::Denied {
            s.message = Some(format!(
                "macOS shows the Screen Recording prompt only once per app. {SCREEN_PERMISSION_HINT}"
            ));
        }
        s
    }
}

/// Microphone status from a raw `AVAuthorizationStatus`.
pub fn microphone_status(raw: isize) -> PermissionStatus {
    let kind = PermissionKind::Microphone;
    match raw {
        av_status::AUTHORIZED => status(kind, PermissionState::Granted, false, "Microphone access granted"),
        av_status::NOT_DETERMINED => status(kind, PermissionState::Prompt, false, "Requesting access shows the macOS microphone prompt"),
        av_status::RESTRICTED => status(kind, PermissionState::Restricted, false, "Microphone access is restricted by a configuration profile or parental controls"),
        av_status::DENIED => status(
            kind,
            PermissionState::Denied,
            true,
            "Microphone access was denied. Enable CaptureOne under System Settings → Privacy & Security → Microphone.",
        ),
        other => status(kind, PermissionState::Denied, true, format!("Unknown AVAuthorizationStatus {other}")),
    }
}

/// Permissions with no macOS equivalent relevant to capture.
pub fn other_status(kind: PermissionKind, os: OsVersion) -> PermissionStatus {
    match kind {
        PermissionKind::LocalNetwork if os.at_least(15, 0) => status(
            kind,
            PermissionState::Prompt,
            false,
            "macOS 15+ asks for Local Network access on first use; the state cannot be queried in advance",
        ),
        PermissionKind::LocalNetwork => status(kind, PermissionState::NotApplicable, false, "No Local Network permission before macOS 15"),
        PermissionKind::Storage => status(kind, PermissionState::NotApplicable, false, "Writing to user-chosen folders needs no permission for non-sandboxed apps"),
        PermissionKind::Notifications => status(kind, PermissionState::NotApplicable, false, "Capturing does not require notification permission on macOS"),
        _ => status(kind, PermissionState::NotApplicable, false, "Not applicable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SONOMA: OsVersion = OsVersion::new(14, 4, 0);
    const MONTEREY: OsVersion = OsVersion::new(12, 6, 0);

    #[test]
    fn screen_states() {
        let g = screen_status(PermissionKind::ScreenCapture, SONOMA, true, false);
        assert_eq!(g.state, PermissionState::Granted);
        assert!(!g.requires_system_settings);

        let p = screen_status(PermissionKind::ScreenCapture, SONOMA, false, false);
        assert_eq!(p.state, PermissionState::Prompt);
        assert!(!p.requires_system_settings);

        let d = screen_status(PermissionKind::ScreenCapture, SONOMA, false, true);
        assert_eq!(d.state, PermissionState::Denied);
        assert!(d.requires_system_settings);

        let after = screen_status_after_request(PermissionKind::ScreenCapture, SONOMA, false);
        assert_eq!(after.state, PermissionState::Denied);
        assert!(after.requires_system_settings);
        assert!(after.message.unwrap().contains("only once"));

        let ok = screen_status_after_request(PermissionKind::ScreenCapture, SONOMA, true);
        assert_eq!(ok.state, PermissionState::Granted);
    }

    #[test]
    fn system_audio_follows_screen_permission() {
        let s = screen_status(PermissionKind::SystemAudio, SONOMA, true, false);
        assert_eq!((s.kind, s.state), (PermissionKind::SystemAudio, PermissionState::Granted));
        let n = screen_status(PermissionKind::SystemAudio, MONTEREY, true, false);
        assert_eq!(n.state, PermissionState::NotApplicable);
        let a = screen_status_after_request(PermissionKind::SystemAudio, MONTEREY, false);
        assert_eq!(a.state, PermissionState::NotApplicable);
    }

    #[test]
    fn microphone_states() {
        assert_eq!(microphone_status(av_status::AUTHORIZED).state, PermissionState::Granted);
        assert_eq!(microphone_status(av_status::NOT_DETERMINED).state, PermissionState::Prompt);
        assert_eq!(microphone_status(av_status::RESTRICTED).state, PermissionState::Restricted);
        let d = microphone_status(av_status::DENIED);
        assert_eq!(d.state, PermissionState::Denied);
        assert!(d.requires_system_settings);
        assert_eq!(microphone_status(42).state, PermissionState::Denied);
    }

    #[test]
    fn other_permissions() {
        assert_eq!(other_status(PermissionKind::Storage, SONOMA).state, PermissionState::NotApplicable);
        assert_eq!(other_status(PermissionKind::LocalNetwork, SONOMA).state, PermissionState::NotApplicable);
        assert_eq!(other_status(PermissionKind::LocalNetwork, OsVersion::new(15, 0, 0)).state, PermissionState::Prompt);
        assert_eq!(other_status(PermissionKind::Notifications, SONOMA).state, PermissionState::NotApplicable);
    }

    #[test]
    fn settings_urls() {
        assert_eq!(privacy_settings_url(PermissionKind::ScreenCapture), Some(SCREEN_CAPTURE_SETTINGS_URL));
        assert_eq!(privacy_settings_url(PermissionKind::SystemAudio), Some(SCREEN_CAPTURE_SETTINGS_URL));
        assert!(privacy_settings_url(PermissionKind::Microphone).unwrap().ends_with("Privacy_Microphone"));
        assert_eq!(privacy_settings_url(PermissionKind::Storage), None);
        // serialized shape matches the contract
        let json = serde_json::to_value(microphone_status(av_status::DENIED)).unwrap();
        assert_eq!(json["kind"], "microphone");
        assert_eq!(json["state"], "denied");
        assert_eq!(json["requiresSystemSettings"], true);
    }
}
