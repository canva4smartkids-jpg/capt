//! OS version, registry helpers, microphone privacy state and opening Settings pages.

use crate::gating::{self, MicrophoneConsent, OsVersion};
use capture_core::{CaptureError, CaptureResult, PermissionKind, PermissionStatus};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// NUL-terminated UTF-16 copy of `s`.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer up to the first NUL.
pub(crate) fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub(crate) fn reg_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let sub = wide(subkey);
    let val = wide(value);
    let mut size: u32 = 0;
    // SAFETY: valid NUL-terminated strings; size query with no data pointer.
    let rc = unsafe { RegGetValueW(root, PCWSTR(sub.as_ptr()), PCWSTR(val.as_ptr()), RRF_RT_REG_SZ, None, None, Some(&mut size)) };
    if rc != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: `buf` holds `bytes` writable bytes.
    let rc = unsafe {
        RegGetValueW(
            root,
            PCWSTR(sub.as_ptr()),
            PCWSTR(val.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    Some(from_wide(&buf))
}

pub(crate) fn reg_dword(root: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let sub = wide(subkey);
    let val = wide(value);
    let mut data: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: `data` is a writable u32 and `size` says so.
    let rc = unsafe {
        RegGetValueW(
            root,
            PCWSTR(sub.as_ptr()),
            PCWSTR(val.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    (rc == ERROR_SUCCESS).then_some(data)
}

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// Reads the OS version from the registry (GetVersionEx lies to unmanifested executables).
pub(crate) fn os_version() -> OsVersion {
    let build = reg_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentBuildNumber")
        .or_else(|| reg_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentBuild"))
        .and_then(|b| b.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let installation = reg_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "InstallationType").unwrap_or_default();
    let product = reg_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "ProductName").unwrap_or_default();
    OsVersion {
        major: reg_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentMajorVersionNumber").unwrap_or(10),
        minor: reg_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentMinorVersionNumber").unwrap_or(0),
        build,
        ubr: reg_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "UBR").unwrap_or(0),
        is_server: installation.to_ascii_lowercase().contains("server") || product.to_ascii_lowercase().contains("server"),
        display_version: reg_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "DisplayVersion")
            .or_else(|| reg_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "ReleaseId")),
    }
}

const CONSENT_MIC: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

pub(crate) fn microphone_permission() -> PermissionStatus {
    let consent = MicrophoneConsent {
        policy: reg_dword(HKEY_LOCAL_MACHINE, r"SOFTWARE\Policies\Microsoft\Windows\AppPrivacy", "LetAppsAccessMicrophone"),
        machine: reg_string(HKEY_LOCAL_MACHINE, CONSENT_MIC, "Value"),
        user: reg_string(HKEY_CURRENT_USER, CONSENT_MIC, "Value"),
        user_desktop_apps: reg_string(HKEY_CURRENT_USER, &format!(r"{CONSENT_MIC}\NonPackaged"), "Value"),
    };
    gating::microphone_permission(&consent)
}

/// Opens the Windows Settings page that controls `kind` (e.g. `ms-settings:privacy-microphone`).
/// Windows cannot show an in-app permission prompt for desktop apps; the desktop app calls this
/// when a `PermissionStatus` has `requires_system_settings == true`.
pub fn open_privacy_settings(kind: PermissionKind) -> CaptureResult<()> {
    let uri = gating::settings_uri(kind).ok_or_else(|| CaptureError::unsupported(format!("Windows has no Settings page for the {kind:?} permission")))?;
    // ShellExecute may use COM shell extensions: run it on a short-lived STA thread so the
    // caller's apartment is untouched.
    let handle = std::thread::Builder::new()
        .name("captureone-open-settings".into())
        .spawn(move || -> CaptureResult<()> {
            // SAFETY: balanced with CoUninitialize below on this thread.
            let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            let target = wide(uri);
            // SAFETY: valid NUL-terminated strings.
            let rc = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(target.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
            if init.is_ok() {
                // SAFETY: matches the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
            // ShellExecute returns a value > 32 on success.
            if (rc.0 as isize) <= 32 {
                return Err(CaptureError::internal(format!("ShellExecuteW({uri}) failed with code {}", rc.0 as isize)));
            }
            Ok(())
        })
        .map_err(|e| CaptureError::internal(format!("cannot spawn settings thread: {e}")))?;
    handle.join().map_err(|_| CaptureError::internal("settings thread panicked"))?
}

pub(crate) fn request_permission(kind: PermissionKind) -> PermissionStatus {
    match kind {
        PermissionKind::Microphone => {
            let mut st = microphone_permission();
            if st.state == capture_core::PermissionState::Denied {
                st.requires_system_settings = true;
                let base = st.message.take().unwrap_or_default();
                st.message = Some(format!(
                    "{base} Windows cannot prompt desktop apps for microphone access; enable it in Settings (ms-settings:privacy-microphone)."
                ));
            }
            st
        }
        other => gating::static_permission(other),
    }
}
