//! macOS version parsing and the (honest) capture-side capability report.
//!
//! API availability that drives the gating:
//! * ScreenCaptureKit (`SCShareableContent`, `SCStream`): macOS 12.3
//! * `SCStreamConfiguration.capturesAudio` (system audio): macOS 13.0
//! * `SCScreenshotManager`: macOS 14.0 (12.3–13 use a single-frame `SCStream`)
//! * `ignoreShadowsSingleWindow`, `SCShareableContent.infoForFilter`: macOS 14.0

use capture_core::{
    CapabilityReport, FeatureKey, FeatureSupport, ImageFormat, PermissionKind, Platform, Size, SupportLevel,
};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OsVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl OsVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// Parses `"14"`, `"14.5"` or `"14.5.1"` (as printed by `sw_vers -productVersion`).
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.trim().split('.');
        let major = it.next()?.trim().parse().ok()?;
        let minor = match it.next() {
            Some(m) => m.trim().parse().ok()?,
            None => 0,
        };
        let patch = match it.next() {
            Some(p) => p.trim().parse().ok()?,
            None => 0,
        };
        if it.next().is_some() {
            return None;
        }
        Some(Self { major, minor, patch })
    }

    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

impl fmt::Display for OsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub const MIN_SCREEN_CAPTURE_KIT: OsVersion = OsVersion::new(12, 3, 0);
pub const MIN_SYSTEM_AUDIO: OsVersion = OsVersion::new(13, 0, 0);
pub const MIN_SCREENSHOT_MANAGER: OsVersion = OsVersion::new(14, 0, 0);

pub fn supports_screen_capture_kit(os: OsVersion) -> bool {
    os.at_least(MIN_SCREEN_CAPTURE_KIT.major, MIN_SCREEN_CAPTURE_KIT.minor)
}
pub fn supports_system_audio(os: OsVersion) -> bool {
    os.at_least(MIN_SYSTEM_AUDIO.major, MIN_SYSTEM_AUDIO.minor)
}
pub fn supports_screenshot_manager(os: OsVersion) -> bool {
    os.at_least(MIN_SCREENSHOT_MANAGER.major, MIN_SCREENSHOT_MANAGER.minor)
}
/// `ignoreShadowsSingleWindow`, `shouldBeOpaque`, `SCShareableContent.infoForFilter`, …
pub fn supports_sonoma_stream_options(os: OsVersion) -> bool {
    os.at_least(14, 0)
}

/// How `screenshot` is implemented on a given OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotStrategy {
    /// `SCScreenshotManager captureImageWithFilter:configuration:completionHandler:` (14+).
    ScreenshotManager,
    /// Start an `SCStream`, take the first complete frame, stop (12.3–13, or forced).
    SingleFrameStream,
    Unsupported,
}

pub fn screenshot_strategy(os: OsVersion, force_stream: bool) -> ScreenshotStrategy {
    if !supports_screen_capture_kit(os) {
        ScreenshotStrategy::Unsupported
    } else if supports_screenshot_manager(os) && !force_stream {
        ScreenshotStrategy::ScreenshotManager
    } else {
        ScreenshotStrategy::SingleFrameStream
    }
}

/// Facts about the running Mac that feed the report.
#[derive(Debug, Clone, PartialEq)]
pub struct HostInfo {
    pub os_version: OsVersion,
    pub device_model: Option<String>,
    /// Largest attached display in physical pixels.
    pub largest_display: Option<Size>,
    pub display_count: usize,
}

const SCK_MIN: &str = "12.3";

/// Capture-side capabilities. Encoders (h264/hevc VideoToolbox) are probed and filled in by
/// media-engine; pause/resume, track layout, shortcuts, history and sharing are engine/app
/// features and are left for those layers to report.
pub fn capability_report(host: &HostInfo, generated_at: String) -> CapabilityReport {
    let os = host.os_version;
    let mut features = BTreeMap::new();
    let screen = vec![PermissionKind::ScreenCapture];

    if !supports_screen_capture_kit(os) {
        let why = format!("ScreenCaptureKit requires macOS {SCK_MIN} or newer (this Mac runs {os})");
        for key in [
            FeatureKey::Screenshot,
            FeatureKey::DisplayCapture,
            FeatureKey::WindowCapture,
            FeatureKey::RegionCapture,
            FeatureKey::MultiDisplay,
            FeatureKey::VideoRecording,
            FeatureKey::CursorCapture,
            FeatureKey::SystemAudio,
        ] {
            let min = if key == FeatureKey::SystemAudio { "13.0" } else { SCK_MIN };
            features.insert(key, FeatureSupport::unsupported(why.clone()).min_os(min));
        }
    } else {
        let screenshot = if supports_screenshot_manager(os) {
            FeatureSupport::native()
        } else {
            FeatureSupport::with(
                SupportLevel::Native,
                "SCScreenshotManager needs macOS 14; on this version a single-frame SCStream is used",
            )
        };
        features.insert(FeatureKey::Screenshot, screenshot.requires(screen.clone()).min_os(SCK_MIN));
        for key in [FeatureKey::DisplayCapture, FeatureKey::WindowCapture, FeatureKey::VideoRecording, FeatureKey::CursorCapture] {
            features.insert(key, FeatureSupport::native().requires(screen.clone()).min_os(SCK_MIN));
        }
        features.insert(
            FeatureKey::RegionCapture,
            FeatureSupport::with(SupportLevel::Native, "SCStreamConfiguration.sourceRect (video); crop of the display image (screenshots)")
                .requires(screen.clone())
                .min_os(SCK_MIN),
        );
        let multi = if host.display_count > 1 {
            FeatureSupport::native()
        } else {
            FeatureSupport::with(SupportLevel::Native, "every attached display can be captured; only one is attached right now")
        };
        features.insert(FeatureKey::MultiDisplay, multi.requires(screen.clone()).min_os(SCK_MIN));
        let audio = if supports_system_audio(os) {
            FeatureSupport::with(
                SupportLevel::Native,
                "ScreenCaptureKit audio (capturesAudio), covered by the Screen & System Audio Recording permission; CaptureOne's own audio is excluded",
            )
        } else {
            FeatureSupport::unsupported(format!(
                "ScreenCaptureKit audio capture requires macOS 13.0 (this Mac runs {os}); a virtual loopback driver would be needed"
            ))
        };
        features.insert(FeatureKey::SystemAudio, audio.requires(screen.clone()).min_os("13.0"));
    }

    features.insert(
        FeatureKey::MicrophoneAudio,
        FeatureSupport::with(
            SupportLevel::Native,
            "AVFoundation/CoreAudio input devices after the Microphone permission is granted (the input pipeline lives in media-engine)",
        )
        .requires(vec![PermissionKind::Microphone]),
    );
    features.insert(
        FeatureKey::HdmiCaptureDevice,
        FeatureSupport::unsupported("external capture devices are not implemented by the macOS backend"),
    );

    let mut notes = vec![
        "DRM-protected content (e.g. Apple TV app, FairPlay video in Safari) is blanked by macOS in captures; CaptureOne does not and cannot bypass this.".to_string(),
        "Screen Recording is a TCC permission without an entitlement: macOS prompts once; afterwards it can only be changed in System Settings → Privacy & Security, and the app usually has to be relaunched.".to_string(),
        "ScreenCaptureKit is linked strongly, so the application requires macOS 12.3 or newer.".to_string(),
        "CaptureSource x/y are reported in macOS global points; width/height in physical pixels (scaleFactor = pixels per point).".to_string(),
    ];
    if os.at_least(15, 0) {
        notes.push("macOS 15+ periodically re-confirms Screen Recording access for apps that capture without the system picker.".to_string());
    }

    CapabilityReport {
        schema_version: 1,
        platform: Platform::Macos,
        os_name: "macOS".to_string(),
        os_version: os.to_string(),
        device_model: host.device_model.clone(),
        app_version: capture_core::APP_VERSION.to_string(),
        generated_at,
        features,
        encoders: Vec::new(),
        frame_rates: vec![24, 30, 60],
        screenshot_formats: vec![ImageFormat::Png, ImageFormat::Jpeg],
        max_recording_resolution: host.largest_display,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(v: &str) -> HostInfo {
        HostInfo {
            os_version: OsVersion::parse(v).unwrap(),
            device_model: Some("Mac14,2".into()),
            largest_display: Some(Size { width: 2880, height: 1800 }),
            display_count: 2,
        }
    }

    #[test]
    fn parses_versions() {
        assert_eq!(OsVersion::parse("14.5.1"), Some(OsVersion::new(14, 5, 1)));
        assert_eq!(OsVersion::parse("15"), Some(OsVersion::new(15, 0, 0)));
        assert_eq!(OsVersion::parse(" 12.3 \n"), Some(OsVersion::new(12, 3, 0)));
        assert_eq!(OsVersion::parse("26.1"), Some(OsVersion::new(26, 1, 0)));
        assert_eq!(OsVersion::parse(""), None);
        assert_eq!(OsVersion::parse("x.1"), None);
        assert_eq!(OsVersion::parse("1.2.3.4"), None);
        assert_eq!(OsVersion::new(13, 6, 7).to_string(), "13.6.7");
    }

    #[test]
    fn version_gates() {
        let v = |s| OsVersion::parse(s).unwrap();
        assert!(!supports_screen_capture_kit(v("12.2.1")));
        assert!(supports_screen_capture_kit(v("12.3")));
        assert!(!supports_system_audio(v("12.7")));
        assert!(supports_system_audio(v("13.0")));
        assert!(!supports_screenshot_manager(v("13.6")));
        assert!(supports_screenshot_manager(v("14.0")));
        assert!(supports_screenshot_manager(v("26.0")));
    }

    #[test]
    fn screenshot_strategies() {
        let v = |s| OsVersion::parse(s).unwrap();
        assert_eq!(screenshot_strategy(v("11.7"), false), ScreenshotStrategy::Unsupported);
        assert_eq!(screenshot_strategy(v("12.3"), false), ScreenshotStrategy::SingleFrameStream);
        assert_eq!(screenshot_strategy(v("13.5"), false), ScreenshotStrategy::SingleFrameStream);
        assert_eq!(screenshot_strategy(v("14.0"), false), ScreenshotStrategy::ScreenshotManager);
        assert_eq!(screenshot_strategy(v("15.1"), true), ScreenshotStrategy::SingleFrameStream);
        assert_eq!(screenshot_strategy(v("11.0"), true), ScreenshotStrategy::Unsupported);
    }

    #[test]
    fn sonoma_report() {
        let r = capability_report(&host("14.5"), "2026-01-01T00:00:00Z".into());
        assert_eq!(r.platform, Platform::Macos);
        assert_eq!(r.os_version, "14.5.0");
        assert!(r.is_usable(FeatureKey::DisplayCapture));
        assert!(r.is_usable(FeatureKey::WindowCapture));
        assert!(r.is_usable(FeatureKey::RegionCapture));
        assert!(r.is_usable(FeatureKey::SystemAudio));
        assert!(r.is_usable(FeatureKey::Screenshot));
        assert_eq!(r.features[&FeatureKey::Screenshot].reason, None);
        assert_eq!(r.features[&FeatureKey::SystemAudio].min_os_version.as_deref(), Some("13.0"));
        assert_eq!(r.features[&FeatureKey::DisplayCapture].requires, Some(vec![PermissionKind::ScreenCapture]));
        assert_eq!(r.level(FeatureKey::HdmiCaptureDevice), SupportLevel::Unsupported);
        // engine-level features are not claimed by the backend
        assert!(!r.features.contains_key(&FeatureKey::H264));
        assert!(!r.features.contains_key(&FeatureKey::PauseResume));
        assert!(r.encoders.is_empty());
        assert_eq!(r.max_recording_resolution, Some(Size { width: 2880, height: 1800 }));
        assert!(r.notes.iter().any(|n| n.contains("DRM")));
        assert!(!r.notes.iter().any(|n| n.contains("macOS 15+")));
        // round-trips through the shared serde mirror
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["platform"], "macos");
        assert_eq!(json["features"]["systemAudio"]["minOsVersion"], "13.0");
    }

    #[test]
    fn monterey_report_gates_audio_and_screenshot_api() {
        let r = capability_report(&host("12.6"), "t".into());
        assert_eq!(r.level(FeatureKey::SystemAudio), SupportLevel::Unsupported);
        assert_eq!(r.features[&FeatureKey::SystemAudio].min_os_version.as_deref(), Some("13.0"));
        assert!(r.is_usable(FeatureKey::Screenshot));
        assert!(r.features[&FeatureKey::Screenshot].reason.as_deref().unwrap().contains("single-frame"));
        let ventura = capability_report(&host("13.0"), "t".into());
        assert!(ventura.is_usable(FeatureKey::SystemAudio));
    }

    #[test]
    fn pre_sck_report_is_all_unsupported() {
        let r = capability_report(&host("12.2"), "t".into());
        for key in [FeatureKey::Screenshot, FeatureKey::DisplayCapture, FeatureKey::VideoRecording, FeatureKey::SystemAudio] {
            assert_eq!(r.level(key), SupportLevel::Unsupported, "{key:?}");
        }
        assert_eq!(r.features[&FeatureKey::DisplayCapture].min_os_version.as_deref(), Some("12.3"));
    }

    #[test]
    fn sequoia_note_and_single_display() {
        let mut h = host("15.2");
        h.display_count = 1;
        let r = capability_report(&h, "t".into());
        assert!(r.notes.iter().any(|n| n.contains("macOS 15+")));
        assert!(r.features[&FeatureKey::MultiDisplay].reason.is_some());
        assert!(r.is_usable(FeatureKey::MultiDisplay));
    }
}
