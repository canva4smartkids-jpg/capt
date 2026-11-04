//! TCC permission queries/requests (Screen Recording via CoreGraphics, Microphone via
//! AVFoundation).

use super::util::wait;
use crate::permissions::av_status;
use block2::RcBlock;
use capture_core::{CaptureError, CaptureResult};
use objc2::runtime::Bool;
use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use objc2_foundation::{ns_string, NSBundle};
use std::sync::mpsc;
use std::time::Duration;

/// How long we wait for the user to answer the microphone prompt.
const MIC_PROMPT_TIMEOUT: Duration = Duration::from_secs(300);

/// `CGPreflightScreenCaptureAccess()` (macOS 10.15+): true if this process may capture.
pub(crate) fn screen_preflight() -> bool {
    CGPreflightScreenCaptureAccess()
}

/// `CGRequestScreenCaptureAccess()`: shows the TCC prompt the first time it is called for this
/// app; returns immediately with the current grant (normally `false` until relaunch).
pub(crate) fn screen_request() -> bool {
    CGRequestScreenCaptureAccess()
}

/// Raw `AVAuthorizationStatus` for audio capture.
pub(crate) fn microphone_raw_status() -> isize {
    // SAFETY: AVMediaTypeAudio is an immutable NSString constant exported by AVFoundation.
    let Some(media) = (unsafe { AVMediaTypeAudio }) else {
        return av_status::DENIED;
    };
    // SAFETY: AVMediaTypeAudio is one of the two media types this method accepts.
    unsafe { AVCaptureDevice::authorizationStatusForMediaType(media) }.0
}

/// Requesting microphone access from an app bundle without `NSMicrophoneUsageDescription`
/// makes TCC kill the process, so refuse instead. Unbundled executables (captureone-cli run
/// from a terminal) are attributed to the responsible app (the terminal), which has its own
/// usage description.
fn check_usage_description() -> CaptureResult<()> {
    let bundle = NSBundle::mainBundle();
    let path = bundle.bundlePath().to_string();
    if !path.ends_with(".app") {
        return Ok(());
    }
    if bundle.objectForInfoDictionaryKey(ns_string!("NSMicrophoneUsageDescription")).is_none() {
        return Err(CaptureError::invalid_config(format!(
            "{path}/Contents/Info.plist lacks NSMicrophoneUsageDescription; macOS would terminate the app on a microphone request"
        )));
    }
    Ok(())
}

/// Shows the microphone prompt if the status is still "not determined" and waits for the
/// user's answer.
pub(crate) fn request_microphone() -> CaptureResult<()> {
    if microphone_raw_status() != av_status::NOT_DETERMINED {
        return Ok(());
    }
    check_usage_description()?;
    // SAFETY: see `microphone_raw_status`.
    let media = unsafe { AVMediaTypeAudio }.ok_or_else(|| CaptureError::internal("AVMediaTypeAudio is unavailable"))?;
    let (tx, rx) = mpsc::sync_channel::<bool>(1);
    // The completion handler runs on an arbitrary dispatch queue.
    let handler = RcBlock::new(move |granted: Bool| {
        let _ = tx.try_send(granted.as_bool());
    });
    // SAFETY: valid media type and a heap block that AVFoundation copies/retains.
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &handler) };
    let granted = wait(&rx, MIC_PROMPT_TIMEOUT, "the microphone permission prompt")?;
    log::info!("macos-capture: microphone access {}", if granted { "granted" } else { "denied" });
    Ok(())
}
