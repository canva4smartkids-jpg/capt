//! OS-build capability gating and permission decisions (platform independent so they are unit
//! tested everywhere; the Windows module only gathers the raw inputs).

use capture_core::{FeatureKey, FeatureSupport, PermissionKind, PermissionState, PermissionStatus, SupportLevel};
use std::collections::BTreeMap;

/// Windows 10 1607 (minimum for per-thread DPI awareness).
pub const BUILD_1607: u32 = 14_393;
/// Windows 10 1803: first build with Windows.Graphics.Capture (picker only).
pub const BUILD_1803: u32 = 17_134;
/// Windows 10 1903: `IGraphicsCaptureItemInterop` (capture from HWND / HMONITOR).
pub const BUILD_1903: u32 = 18_362;
/// Windows 10 2004: `GraphicsCaptureSession.IsCursorCaptureEnabled`.
pub const BUILD_2004: u32 = 19_041;
/// Windows Server 2022: `GraphicsCaptureSession.IsBorderRequired` (server line).
pub const BUILD_SERVER_2022: u32 = 20_348;
/// Windows 11 21H2: `IsBorderRequired` on the client line.
pub const BUILD_WIN11: u32 = 22_000;
/// Windows 11 24H2: `GraphicsCaptureSession.MinUpdateInterval`.
pub const BUILD_WIN11_24H2: u32 = 26_100;

pub fn build_label(build: u32) -> String {
    format!("10.0.{build}")
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    /// Update build revision (the part after the last dot in winver).
    pub ubr: u32,
    pub is_server: bool,
    /// e.g. "23H2" (Windows 10 20H2 and later).
    pub display_version: Option<String>,
}

impl OsVersion {
    /// Marketing name. The registry `ProductName` still says "Windows 10" on Windows 11, so the
    /// name is derived from the build number.
    pub fn name(&self) -> String {
        if self.major != 10 {
            return format!("Windows NT {}.{}", self.major, self.minor);
        }
        if self.is_server {
            return match self.build {
                b if b >= BUILD_WIN11_24H2 => "Windows Server 2025".into(),
                b if b >= BUILD_SERVER_2022 => "Windows Server 2022".into(),
                b if b >= 17_763 => "Windows Server 2019".into(),
                b if b >= BUILD_1607 => "Windows Server 2016".into(),
                _ => "Windows Server".into(),
            };
        }
        if self.build >= BUILD_WIN11 {
            "Windows 11".into()
        } else {
            "Windows 10".into()
        }
    }

    /// e.g. "10.0.22631.4317 (23H2)".
    pub fn version_string(&self) -> String {
        let mut s = format!("{}.{}.{}", self.major, self.minor, self.build);
        if self.ubr > 0 {
            s.push_str(&format!(".{}", self.ubr));
        }
        if let Some(dv) = self.display_version.as_deref().filter(|d| !d.is_empty()) {
            s.push_str(&format!(" ({dv})"));
        }
        s
    }
}

/// Runtime `ApiInformation` / `GraphicsCaptureSession::IsSupported()` results. `None` = the probe
/// itself failed; the build-number gate is used instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WgcProbe {
    pub api_supported: Option<bool>,
    pub cursor_toggle: Option<bool>,
    pub border_toggle: Option<bool>,
    pub min_update_interval: Option<bool>,
}

/// What the WGC session can actually be configured to do on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WgcFeatures {
    /// HWND/HMONITOR capture available.
    pub capture: bool,
    pub cursor_toggle: bool,
    pub border_toggle: bool,
    pub min_update_interval: bool,
}

pub fn resolve_wgc(os: &OsVersion, probe: &WgcProbe) -> WgcFeatures {
    let by_build = os.major > 10 || (os.major == 10 && os.build >= BUILD_1903);
    let capture = probe.api_supported.unwrap_or(by_build) && by_build;
    if !capture {
        return WgcFeatures::default();
    }
    WgcFeatures {
        capture,
        cursor_toggle: probe.cursor_toggle.unwrap_or(os.build >= BUILD_2004),
        border_toggle: probe.border_toggle.unwrap_or(os.build >= BUILD_SERVER_2022),
        min_update_interval: probe.min_update_interval.unwrap_or(os.build >= BUILD_WIN11_24H2),
    }
}

/// Audio endpoints found at report time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioProbe {
    /// `Ok(name)` of the default render endpoint, or why there is none.
    pub default_render: Result<String, String>,
}

/// Capture-side feature map + notes for the capability report.
pub fn capture_features(os: &OsVersion, wgc: &WgcFeatures, audio: &AudioProbe) -> (BTreeMap<FeatureKey, FeatureSupport>, Vec<String>) {
    let mut f = BTreeMap::new();
    let mut notes = Vec::new();
    let min = build_label(BUILD_1903);

    if wgc.capture {
        f.insert(FeatureKey::Screenshot, FeatureSupport::native().min_os(&min));
        f.insert(FeatureKey::DisplayCapture, FeatureSupport::native().min_os(&min));
        f.insert(FeatureKey::WindowCapture, FeatureSupport::native().min_os(&min));
        f.insert(
            FeatureKey::RegionCapture,
            FeatureSupport::with(SupportLevel::Native, "cropped from the display capture").min_os(&min),
        );
        f.insert(
            FeatureKey::MultiDisplay,
            FeatureSupport::with(SupportLevel::Native, "any connected display can be selected (one display per capture)").min_os(&min),
        );
        f.insert(
            FeatureKey::VideoRecording,
            FeatureSupport::with(SupportLevel::Native, "Windows Graphics Capture frames, encoded by media-engine").min_os(&min),
        );
        let cursor = if wgc.cursor_toggle {
            FeatureSupport::native()
        } else {
            FeatureSupport::with(
                SupportLevel::Partial,
                format!("the cursor is always captured on this Windows build; hiding it requires Windows 10 2004 (build {BUILD_2004})"),
            )
        };
        f.insert(FeatureKey::CursorCapture, cursor.min_os(build_label(BUILD_2004)));
        if wgc.border_toggle {
            notes.push("The Windows capture border is turned off where the OS allows it (Windows 11 / Windows Server 2022 and later).".into());
        } else {
            notes.push(format!(
                "Windows draws a yellow border around the captured display/window while capturing; removing it requires Windows 11 (build {BUILD_WIN11}) or Windows Server 2022 (build {BUILD_SERVER_2022})."
            ));
        }
        notes.push(if wgc.min_update_interval {
            "Frame delivery is throttled by the OS (GraphicsCaptureSession.MinUpdateInterval).".into()
        } else {
            "Frame delivery is rate-limited in the backend (MinUpdateInterval needs Windows 11 24H2).".into()
        });
    } else {
        let reason = if os.build < BUILD_1903 {
            format!(
                "Windows Graphics Capture from a display/window handle requires Windows 10 1903 (build {BUILD_1903}) or later; this is build {}",
                os.build
            )
        } else {
            "Windows Graphics Capture is not available in this session (GraphicsCaptureSession.IsSupported() returned false — e.g. no interactive desktop or no Direct3D 11 device)".to_string()
        };
        for k in [
            FeatureKey::Screenshot,
            FeatureKey::DisplayCapture,
            FeatureKey::WindowCapture,
            FeatureKey::RegionCapture,
            FeatureKey::MultiDisplay,
            FeatureKey::VideoRecording,
            FeatureKey::CursorCapture,
        ] {
            f.insert(k, FeatureSupport::unsupported(reason.clone()).min_os(&min));
        }
    }

    f.insert(
        FeatureKey::MicrophoneAudio,
        FeatureSupport::with(SupportLevel::Native, "WASAPI capture endpoints (recorded by media-engine)").requires(vec![PermissionKind::Microphone]),
    );
    match &audio.default_render {
        Ok(name) => {
            f.insert(
                FeatureKey::SystemAudio,
                FeatureSupport::with(SupportLevel::Native, format!("WASAPI loopback of the default output device ({name})")),
            );
        }
        Err(why) => {
            f.insert(
                FeatureKey::SystemAudio,
                FeatureSupport::unsupported(format!("WASAPI loopback needs an active audio output device: {why}")),
            );
        }
    }
    f.insert(
        FeatureKey::HdmiCaptureDevice,
        FeatureSupport::unsupported("capture-card (UVC) sources are not implemented in the Windows backend"),
    );

    notes.push(
        "Protected content stays protected: DRM video and windows that use SetWindowDisplayAffinity (WDA_MONITOR / WDA_EXCLUDEFROMCAPTURE) are blacked out or omitted by Windows."
            .into(),
    );
    notes.push("Windows Graphics Capture delivers frames only when the content changes; media-engine repeats frames for a constant frame rate.".into());
    notes.push("Minimized windows produce no frames until they are restored.".into());
    (f, notes)
}

/// Raw inputs for the microphone privacy decision (registry values, `None` = absent).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MicrophoneConsent {
    /// `HKLM\SOFTWARE\Policies\Microsoft\Windows\AppPrivacy\LetAppsAccessMicrophone`
    /// (0 = user in control, 1 = force allow, 2 = force deny).
    pub policy: Option<u32>,
    /// `HKLM\...\CapabilityAccessManager\ConsentStore\microphone` `Value` (device-wide switch).
    pub machine: Option<String>,
    /// `HKCU\...\CapabilityAccessManager\ConsentStore\microphone` `Value`.
    pub user: Option<String>,
    /// `HKCU\...\ConsentStore\microphone\NonPackaged` `Value` ("Let desktop apps access your microphone").
    pub user_desktop_apps: Option<String>,
}

fn is_deny(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| s.trim().eq_ignore_ascii_case("deny"))
}

pub const MIC_SETTINGS_HINT: &str = "Settings > Privacy & security > Microphone";

pub fn microphone_permission(c: &MicrophoneConsent) -> PermissionStatus {
    let mut st = PermissionStatus::new(PermissionKind::Microphone, PermissionState::Granted);
    let deny = |msg: String| PermissionStatus {
        kind: PermissionKind::Microphone,
        state: PermissionState::Denied,
        requires_system_settings: true,
        message: Some(msg),
    };
    match c.policy {
        Some(2) => {
            st.state = PermissionState::Restricted;
            st.message = Some("Microphone access is blocked by group policy (LetAppsAccessMicrophone = Force Deny); ask your administrator.".into());
            return st;
        }
        Some(1) => {
            st.message = Some("Microphone access is forced on by group policy.".into());
            return st;
        }
        _ => {}
    }
    if is_deny(&c.machine) {
        return deny(format!(
            "Microphone access is turned off for this device ({MIC_SETTINGS_HINT}, requires an administrator)."
        ));
    }
    if is_deny(&c.user) {
        return deny(format!("Microphone access is turned off ({MIC_SETTINGS_HINT})."));
    }
    if is_deny(&c.user_desktop_apps) {
        return deny(format!("'Let desktop apps access your microphone' is turned off ({MIC_SETTINGS_HINT})."));
    }
    if c.machine.is_none() && c.user.is_none() && c.user_desktop_apps.is_none() {
        st.message = Some("No Windows privacy restriction is recorded for the microphone (desktop apps are allowed by default).".into());
    }
    st
}

/// Static permission answers for everything except the microphone.
pub fn static_permission(kind: PermissionKind) -> PermissionStatus {
    let mut st = PermissionStatus::new(kind, PermissionState::NotApplicable);
    st.message = Some(
        match kind {
            PermissionKind::ScreenCapture => "Windows Graphics Capture needs no permission for desktop apps.",
            PermissionKind::SystemAudio => "WASAPI loopback capture needs no permission.",
            PermissionKind::Storage => "Desktop apps write to user folders without a permission prompt.",
            PermissionKind::Notifications => "Notification permission is managed in Windows Settings > System > Notifications.",
            PermissionKind::LocalNetwork => "Windows Defender Firewall may ask once when the companion hub starts listening.",
            PermissionKind::Microphone => "Microphone access is governed by Windows privacy settings.",
        }
        .into(),
    );
    st
}

/// `ms-settings:` page for a permission, if Windows has one.
pub fn settings_uri(kind: PermissionKind) -> Option<&'static str> {
    match kind {
        PermissionKind::Microphone => Some("ms-settings:privacy-microphone"),
        PermissionKind::Notifications => Some("ms-settings:notifications"),
        // Windows 11 lists apps that used Windows Graphics Capture here.
        PermissionKind::ScreenCapture => Some("ms-settings:privacy-graphicscaptureprogrammatic"),
        PermissionKind::SystemAudio | PermissionKind::Storage | PermissionKind::LocalNetwork => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(build: u32) -> OsVersion {
        OsVersion {
            major: 10,
            minor: 0,
            build,
            ubr: 1,
            is_server: false,
            display_version: None,
        }
    }

    #[test]
    fn os_names_and_versions() {
        assert_eq!(os(19_045).name(), "Windows 10");
        assert_eq!(os(22_631).name(), "Windows 11");
        let srv = OsVersion { is_server: true, ..os(20_348) };
        assert_eq!(srv.name(), "Windows Server 2022");
        assert_eq!(OsVersion { is_server: true, ..os(17_763) }.name(), "Windows Server 2019");
        assert_eq!(OsVersion { is_server: true, ..os(26_100) }.name(), "Windows Server 2025");
        let v = OsVersion {
            ubr: 4317,
            display_version: Some("23H2".into()),
            ..os(22_631)
        };
        assert_eq!(v.version_string(), "10.0.22631.4317 (23H2)");
        assert_eq!(OsVersion { ubr: 0, ..os(19_041) }.version_string(), "10.0.19041");
    }

    #[test]
    fn build_gates_without_runtime_probe() {
        let none = WgcProbe::default();
        assert!(!resolve_wgc(&os(17_763), &none).capture, "1809 has no HWND/HMONITOR interop");
        let f1903 = resolve_wgc(&os(BUILD_1903), &none);
        assert!(f1903.capture && !f1903.cursor_toggle && !f1903.border_toggle && !f1903.min_update_interval);
        let f2004 = resolve_wgc(&os(19_045), &none);
        assert!(f2004.cursor_toggle && !f2004.border_toggle);
        let srv = resolve_wgc(
            &OsVersion {
                is_server: true,
                ..os(BUILD_SERVER_2022)
            },
            &none,
        );
        assert!(srv.border_toggle && !srv.min_update_interval);
        let w11 = resolve_wgc(&os(22_631), &none);
        assert!(w11.border_toggle && !w11.min_update_interval);
        assert!(resolve_wgc(&os(26_100), &none).min_update_interval);
    }

    #[test]
    fn runtime_probe_wins_over_build() {
        let probe = WgcProbe {
            api_supported: Some(false),
            ..Default::default()
        };
        assert_eq!(resolve_wgc(&os(22_631), &probe), WgcFeatures::default());
        let probe = WgcProbe {
            api_supported: Some(true),
            cursor_toggle: Some(true),
            border_toggle: Some(false),
            min_update_interval: Some(true),
        };
        let f = resolve_wgc(&os(22_631), &probe);
        assert!(f.capture && f.cursor_toggle && !f.border_toggle && f.min_update_interval);
        // A runtime "supported" cannot resurrect a build without the interop.
        assert!(
            !resolve_wgc(
                &os(17_134),
                &WgcProbe {
                    api_supported: Some(true),
                    ..Default::default()
                }
            )
            .capture
        );
    }

    #[test]
    fn feature_report_is_honest() {
        let audio_ok = AudioProbe {
            default_render: Ok("Speakers".into()),
        };
        let (f, notes) = capture_features(&os(22_631), &resolve_wgc(&os(22_631), &WgcProbe::default()), &audio_ok);
        assert_eq!(f[&FeatureKey::DisplayCapture].level, SupportLevel::Native);
        assert_eq!(f[&FeatureKey::CursorCapture].level, SupportLevel::Native);
        assert_eq!(f[&FeatureKey::SystemAudio].level, SupportLevel::Native);
        assert_eq!(f[&FeatureKey::HdmiCaptureDevice].level, SupportLevel::Unsupported);
        assert_eq!(f[&FeatureKey::MicrophoneAudio].requires, Some(vec![PermissionKind::Microphone]));
        assert!(notes.iter().any(|n| n.contains("WDA_EXCLUDEFROMCAPTURE")));
        assert!(notes.iter().any(|n| n.contains("border is turned off")));

        let (f, notes) = capture_features(&os(18_363), &resolve_wgc(&os(18_363), &WgcProbe::default()), &audio_ok);
        assert_eq!(f[&FeatureKey::CursorCapture].level, SupportLevel::Partial);
        assert!(notes.iter().any(|n| n.contains("yellow border")));

        let no_audio = AudioProbe {
            default_render: Err("no device".into()),
        };
        let (f, _) = capture_features(&os(17_134), &resolve_wgc(&os(17_134), &WgcProbe::default()), &no_audio);
        assert_eq!(f[&FeatureKey::Screenshot].level, SupportLevel::Unsupported);
        assert!(f[&FeatureKey::Screenshot].reason.as_deref().unwrap_or("").contains("18362"));
        assert_eq!(f[&FeatureKey::SystemAudio].level, SupportLevel::Unsupported);
    }

    #[test]
    fn microphone_decisions() {
        let granted = microphone_permission(&MicrophoneConsent::default());
        assert_eq!(granted.state, PermissionState::Granted);
        assert!(!granted.requires_system_settings);

        let allow = MicrophoneConsent {
            machine: Some("Allow".into()),
            user: Some("Allow".into()),
            user_desktop_apps: Some("Allow".into()),
            policy: Some(0),
        };
        assert_eq!(microphone_permission(&allow).state, PermissionState::Granted);

        let st = microphone_permission(&MicrophoneConsent {
            user_desktop_apps: Some("Deny".into()),
            ..allow.clone()
        });
        assert_eq!(st.state, PermissionState::Denied);
        assert!(st.requires_system_settings);
        assert!(st.message.unwrap().contains("desktop apps"));

        let st = microphone_permission(&MicrophoneConsent {
            user: Some("deny".into()),
            ..allow.clone()
        });
        assert_eq!(st.state, PermissionState::Denied);

        let st = microphone_permission(&MicrophoneConsent {
            machine: Some("Deny".into()),
            ..allow.clone()
        });
        assert_eq!(st.state, PermissionState::Denied);
        assert!(st.message.unwrap().contains("administrator"));

        let st = microphone_permission(&MicrophoneConsent {
            policy: Some(2),
            ..allow.clone()
        });
        assert_eq!(st.state, PermissionState::Restricted);
        let st = microphone_permission(&MicrophoneConsent {
            policy: Some(1),
            user: Some("Deny".into()),
            ..Default::default()
        });
        assert_eq!(st.state, PermissionState::Granted);
    }

    #[test]
    fn static_permissions_and_settings_pages() {
        assert_eq!(static_permission(PermissionKind::ScreenCapture).state, PermissionState::NotApplicable);
        assert_eq!(static_permission(PermissionKind::SystemAudio).state, PermissionState::NotApplicable);
        assert_eq!(settings_uri(PermissionKind::Microphone), Some("ms-settings:privacy-microphone"));
        assert_eq!(settings_uri(PermissionKind::LocalNetwork), None);
    }
}
