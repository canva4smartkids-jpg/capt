//! Desktop-app level capability adjustments on top of the engine report: features
//! the app itself implements (global shortcuts, history) and features that depend
//! on services not integrated in this build (companion hub, discovery).

use crate::events::HUB_NOT_INTEGRATED;
use capture_core::*;

pub fn with_app_features(mut r: CapabilityReport, app_version: &str) -> CapabilityReport {
    r.app_version = app_version.to_string();
    r.generated_at = now_iso8601();
    let shortcuts = if cfg!(target_os = "linux") {
        FeatureSupport::with(SupportLevel::Partial, "Global shortcuts work on X11 sessions; Wayland compositors do not allow global key grabs.")
    } else if cfg!(target_os = "macos") {
        FeatureSupport::native().min_os("macOS 10.15")
    } else {
        FeatureSupport::native()
    };
    r.features.insert(FeatureKey::GlobalShortcuts, shortcuts);
    r.features.insert(FeatureKey::RecordingHistory, FeatureSupport::native());
    r.features.entry(FeatureKey::Share).or_insert_with(|| FeatureSupport::unsupported("Use Export or Show in folder to share files."));
    for key in [FeatureKey::RemoteControl, FeatureKey::DeviceDiscovery, FeatureKey::FileTransfer, FeatureKey::TvNativeScreenshot] {
        r.features.insert(key, FeatureSupport::unsupported(HUB_NOT_INTEGRATED));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn app_features_are_honest() {
        let base = CapabilityReport {
            schema_version: 1,
            platform: Platform::Linux,
            os_name: "Linux".into(),
            os_version: "6".into(),
            device_model: None,
            app_version: "x".into(),
            generated_at: "t".into(),
            features: BTreeMap::new(),
            encoders: vec![],
            frame_rates: vec![30],
            screenshot_formats: vec![ImageFormat::Png],
            max_recording_resolution: None,
            notes: vec![],
        };
        let r = with_app_features(base, "0.1.0");
        assert_eq!(r.app_version, "0.1.0");
        assert!(r.is_usable(FeatureKey::RecordingHistory));
        assert_eq!(r.level(FeatureKey::DeviceDiscovery), SupportLevel::Unsupported);
        assert_eq!(r.features[&FeatureKey::RemoteControl].reason.as_deref(), Some(HUB_NOT_INTEGRATED));
        if cfg!(target_os = "linux") {
            assert_eq!(r.level(FeatureKey::GlobalShortcuts), SupportLevel::Partial);
        }
    }
}
