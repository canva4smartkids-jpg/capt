//! Mapping of ScreenCaptureKit (`SCStreamErrorDomain`) errors to `CaptureError`s, and
//! classification of "stream stopped" callbacks into `StreamEvent`s.

use capture_core::{CaptureError, CaptureErrorCode, StreamEvent};

/// `SCStreamError.Code` values (SCError.h).
pub mod sck_code {
    pub const USER_DECLINED: isize = -3801;
    pub const FAILED_TO_START: isize = -3802;
    pub const MISSING_ENTITLEMENTS: isize = -3803;
    pub const FAILED_APPLICATION_CONNECTION_INVALID: isize = -3804;
    pub const FAILED_APPLICATION_CONNECTION_INTERRUPTED: isize = -3805;
    pub const FAILED_NO_MATCHING_APPLICATION_CONTEXT: isize = -3806;
    pub const ATTEMPT_TO_START_STREAM_STATE: isize = -3807;
    pub const ATTEMPT_TO_STOP_STREAM_STATE: isize = -3808;
    pub const ATTEMPT_TO_UPDATE_FILTER_STATE: isize = -3809;
    pub const ATTEMPT_TO_CONFIG_STATE: isize = -3810;
    pub const INTERNAL_ERROR: isize = -3811;
    pub const INVALID_PARAMETER: isize = -3812;
    pub const NO_WINDOW_LIST: isize = -3813;
    pub const NO_DISPLAY_LIST: isize = -3814;
    pub const NO_CAPTURE_SOURCE: isize = -3815;
    pub const REMOVING_STREAM: isize = -3816;
    pub const USER_STOPPED: isize = -3817;
    pub const FAILED_TO_START_AUDIO_CAPTURE: isize = -3818;
    pub const FAILED_TO_STOP_AUDIO_CAPTURE: isize = -3819;
    pub const FAILED_TO_START_MICROPHONE_CAPTURE: isize = -3820;
    pub const SYSTEM_STOPPED_STREAM: isize = -3821;
}

/// Hint appended to permission errors.
pub const SCREEN_PERMISSION_HINT: &str = "Allow CaptureOne (or the terminal running captureone-cli) under System Settings → Privacy & Security → Screen & System Audio Recording, then relaunch it.";

/// Maps an `NSError` (domain/code/description) to a `CaptureError`.
/// `is_sck_domain` is true when the domain equals `SCStreamErrorDomain`.
pub fn map_ns_error(is_sck_domain: bool, domain: &str, code: isize, description: &str) -> CaptureError {
    use sck_code::*;
    let (code_kind, message) = if is_sck_domain {
        match code {
            USER_DECLINED => (CaptureErrorCode::PermissionDenied, format!("Screen Recording permission is not granted ({description}). {SCREEN_PERMISSION_HINT}")),
            MISSING_ENTITLEMENTS => (CaptureErrorCode::PermissionDenied, format!("ScreenCaptureKit reports missing entitlements: {description}")),
            FAILED_APPLICATION_CONNECTION_INVALID | FAILED_APPLICATION_CONNECTION_INTERRUPTED | USER_STOPPED | SYSTEM_STOPPED_STREAM => {
                let why = match code {
                    USER_STOPPED => "capture was stopped from the macOS screen-sharing menu",
                    SYSTEM_STOPPED_STREAM => "macOS stopped the capture stream",
                    _ => "the connection to the ScreenCaptureKit service was lost",
                };
                (CaptureErrorCode::Interrupted, format!("{why}: {description}"))
            }
            ATTEMPT_TO_START_STREAM_STATE | ATTEMPT_TO_STOP_STREAM_STATE | ATTEMPT_TO_UPDATE_FILTER_STATE | ATTEMPT_TO_CONFIG_STATE | REMOVING_STREAM => {
                (CaptureErrorCode::InvalidState, format!("ScreenCaptureKit stream state error: {description}"))
            }
            INVALID_PARAMETER => (CaptureErrorCode::InvalidConfig, format!("ScreenCaptureKit rejected the stream configuration: {description}")),
            NO_WINDOW_LIST | NO_DISPLAY_LIST => (CaptureErrorCode::SourceNotFound, format!("ScreenCaptureKit could not enumerate capture sources: {description}")),
            NO_CAPTURE_SOURCE => (CaptureErrorCode::SourceClosed, format!("the capture source is gone: {description}")),
            FAILED_TO_START_AUDIO_CAPTURE | FAILED_TO_STOP_AUDIO_CAPTURE | FAILED_TO_START_MICROPHONE_CAPTURE => {
                (CaptureErrorCode::DeviceBusy, format!("ScreenCaptureKit audio capture failed: {description}"))
            }
            FAILED_TO_START | FAILED_NO_MATCHING_APPLICATION_CONTEXT | INTERNAL_ERROR => {
                (CaptureErrorCode::Internal, format!("ScreenCaptureKit failed: {description}"))
            }
            _ => (CaptureErrorCode::Internal, format!("ScreenCaptureKit error {code}: {description}")),
        }
    } else {
        (CaptureErrorCode::Internal, format!("{domain} error {code}: {description}"))
    };
    let mut err = CaptureError::new(code_kind, message).with_detail("domain", domain.to_string()).with_detail("code", code as i64);
    if code_kind == CaptureErrorCode::Interrupted {
        // The partially written recording is still usable.
        err.recoverable = true;
    }
    err
}

/// Turns `stream:didStopWithError:` into the event the sink receives.
///
/// `source_exists` is the result of probing the display/window after the stop: a vanished
/// source is always reported as `SourceClosed` (window closed, display unplugged), whatever
/// code SCK used.
pub fn classify_stream_stop(error: CaptureError, source_exists: bool) -> StreamEvent {
    if !source_exists || error.code == CaptureErrorCode::SourceClosed {
        StreamEvent::SourceClosed
    } else {
        StreamEvent::Error(error)
    }
}

/// Error for an Objective-C completion handler that never fired.
pub fn timeout_error(what: &str, secs: u64) -> CaptureError {
    CaptureError::new(CaptureErrorCode::Internal, format!("timed out after {secs}s waiting for {what}")).with_detail("timeoutSec", secs as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOMAIN: &str = "com.apple.ScreenCaptureKit.SCStreamErrorDomain";

    #[test]
    fn permission_errors() {
        let e = map_ns_error(true, DOMAIN, sck_code::USER_DECLINED, "The user declined TCCs");
        assert_eq!(e.code, CaptureErrorCode::PermissionDenied);
        assert!(!e.recoverable);
        assert!(e.message.contains("System Settings"));
        let details = e.details.unwrap();
        assert_eq!(details["code"], serde_json::json!(-3801));
        assert_eq!(details["domain"], serde_json::json!(DOMAIN));
    }

    #[test]
    fn interruption_errors_are_recoverable() {
        for code in [sck_code::USER_STOPPED, sck_code::SYSTEM_STOPPED_STREAM, sck_code::FAILED_APPLICATION_CONNECTION_INTERRUPTED] {
            let e = map_ns_error(true, DOMAIN, code, "x");
            assert_eq!(e.code, CaptureErrorCode::Interrupted, "{code}");
            assert!(e.recoverable);
        }
    }

    #[test]
    fn other_codes() {
        let m = |c| map_ns_error(true, DOMAIN, c, "d").code;
        assert_eq!(m(sck_code::NO_CAPTURE_SOURCE), CaptureErrorCode::SourceClosed);
        assert_eq!(m(sck_code::INVALID_PARAMETER), CaptureErrorCode::InvalidConfig);
        assert_eq!(m(sck_code::ATTEMPT_TO_STOP_STREAM_STATE), CaptureErrorCode::InvalidState);
        assert_eq!(m(sck_code::NO_DISPLAY_LIST), CaptureErrorCode::SourceNotFound);
        assert_eq!(m(sck_code::FAILED_TO_START_AUDIO_CAPTURE), CaptureErrorCode::DeviceBusy);
        assert_eq!(m(sck_code::MISSING_ENTITLEMENTS), CaptureErrorCode::PermissionDenied);
        assert_eq!(m(sck_code::INTERNAL_ERROR), CaptureErrorCode::Internal);
        assert_eq!(m(-1), CaptureErrorCode::Internal);
        // codes from foreign domains are never interpreted as SCK codes
        let e = map_ns_error(false, "NSOSStatusErrorDomain", sck_code::USER_DECLINED, "d");
        assert_eq!(e.code, CaptureErrorCode::Internal);
        assert!(e.message.contains("NSOSStatusErrorDomain"));
    }

    #[test]
    fn stop_classification() {
        let err = map_ns_error(true, DOMAIN, sck_code::SYSTEM_STOPPED_STREAM, "stopped");
        assert!(matches!(classify_stream_stop(err.clone(), false), StreamEvent::SourceClosed));
        match classify_stream_stop(err, true) {
            StreamEvent::Error(e) => assert_eq!(e.code, CaptureErrorCode::Interrupted),
            other => panic!("unexpected {other:?}"),
        }
        let gone = map_ns_error(true, DOMAIN, sck_code::NO_CAPTURE_SOURCE, "gone");
        assert!(matches!(classify_stream_stop(gone, true), StreamEvent::SourceClosed));
        let denied = map_ns_error(true, DOMAIN, sck_code::USER_DECLINED, "revoked");
        match classify_stream_stop(denied, true) {
            StreamEvent::Error(e) => assert_eq!(e.code, CaptureErrorCode::PermissionDenied),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn timeouts() {
        let e = timeout_error("SCShareableContent", 10);
        assert_eq!(e.code, CaptureErrorCode::Internal);
        assert!(e.message.contains("10s"));
    }
}
