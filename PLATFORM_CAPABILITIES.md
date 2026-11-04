# CaptureOne — Platform Capability Matrix

Support levels (from the shared contract `SupportLevel`):

| Level | Meaning |
|---|---|
| **Native** | Implemented with a public OS API available to ordinary third-party apps |
| **Partial** | Works with restrictions (OS version, device build, app opt-out, DRM blanking) |
| **Companion** | The app on this platform acts as a companion (remote control, browse/play, transfer) |
| **External HW** | Only possible by recording the device's HDMI output with external capture hardware |
| **Unsupported** | Not possible on this platform |

This matrix describes what the platform *allows* and what CaptureOne *implements*.
Whether each item has actually been *verified* on real hardware is tracked separately in
[`TEST_REPORT.md`](TEST_REPORT.md) and the final acceptance table in
[`PROJECT_STATUS.md`](PROJECT_STATUS.md).

## 1. Capture features

| Feature | Windows | macOS | Linux (X11) | Android phone/tablet | Android TV / Google TV | Samsung Tizen TV | LG webOS TV |
|---|---|---|---|---|---|---|---|
| Full-screen recording | Native (WGC) | Native (ScreenCaptureKit) | Native (X11 SHM) | Native (MediaProjection) | Partial — device dependent¹ | External HW | External HW |
| Display selection | Native | Native | Native (RandR) | n/a (single display) | n/a | — | — |
| Window capture | Native (WGC HWND) | Native (SCK window filter) | Partial² | Partial — single-app capture on Android 14+ (user picks in system dialog) | Partial¹ | — | — |
| Region capture | Native (crop) | Native (sourceRect/crop) | Native | Unsupported (crop in post only) | Unsupported | — | — |
| Screenshots PNG/JPEG | Native | Native (macOS 14+ SCScreenshotManager; 12.3–13 via stream) | Native | Native (ImageReader) | Partial¹ | Unsupported³ | Partial — experimental, undocumented SSAP call from desktop⁴ |
| Pause / resume | Native | Native | Native | Native (PTS offset) | Partial¹ | — | — |
| Cursor show/hide | Native (Win10 2004+) | Native | Native (XFixes) | Unsupported (OS draws per projection) | — | — | — |
| H.264 | Native (Media Foundation / NVENC / QSV / AMF via ffmpeg) | Native (VideoToolbox via ffmpeg) | Native (x264 / VAAPI) | Native (MediaCodec AVC) | Partial¹ | — | — |
| H.265 | Partial (needs HEVC MFT/GPU) | Native (VideoToolbox) | Native (x265 / VAAPI) | Partial (device encoder) | Partial¹ | — | — |
| FPS 24/30/60 | Native | Native | Native | Native (60 fps device dependent) | Partial¹ | — | — |
| Microphone | Native (WASAPI via cpal) | Native (CoreAudio via cpal; TCC prompt) | Native (ALSA/Pulse via cpal) | Native (AudioRecord) | Partial (few TVs have mics) | — | — |
| System / internal audio | Native (WASAPI loopback) | Native (SCK audio, macOS 13+) | Partial (PulseAudio/PipeWire monitor) | Partial (AudioPlaybackCapture API 29+; apps can opt out) | Partial¹ | — | — |
| Separate audio tracks | Native | Native | Native | Native | Partial¹ | — | — |
| HDMI capture card input | **Implemented, partial** via ffmpeg `dshow` (`dshow:<name>`); verified only by list-parser unit tests and the lavfi test seam | **Implemented, partial** via ffmpeg `avfoundation` (`avfoundation:<index>`; screens excluded); verified only by parser tests and the test seam | **Implemented, partial** via ffmpeg `v4l2` (`v4l2:/dev/videoN`, enumerated from sysfs); verified only by sysfs/arg tests and the test seam | Unsupported | Unsupported | — | — |
| DRM / HDCP content | Blank (OS-enforced) | Blank (OS-enforced) | Not protected by X11 | Blank (FLAG_SECURE) | Blank | Blank | Blank |

## 2. Product features

| Feature | Windows | macOS | Linux | Android | Android TV | Tizen | webOS |
|---|---|---|---|---|---|---|---|
| Recording history (SQLite) | Native | Native | Native | Native | Native (when capture available) | Via desktop hub | Via desktop hub |
| Preview / rename / delete / export | Native | Native | Native | Native (MediaStore, share sheet) | Native | Play from hub | Play from hub |
| Output folder selection | Native | Native | Native | Fixed MediaStore folders (scoped storage) | Fixed | — | — |
| Countdown + recording indicator | Native (tray + always-on-top bar) | Native | Native | Native (notification) | Native | — | — |
| Global keyboard shortcuts | Native | Native | Native (X11) | Unsupported (Quick Settings tile instead) | Remote keys | Remote keys | Remote keys |
| Device discovery (SSDP/mDNS) | Native | Native | Native | — | — | — | — |
| TV remote control from desktop | Native (LG SSAP, Samsung WS) | Native | Native | — | — | Companion | Companion |
| Remote-start desktop recording | — | — | — | — | Companion | Companion | Companion |

## 3. Notes

1. **Android TV / Google TV.** `MediaProjection` is part of the Android framework, but
   the user-consent screen (`MediaProjectionPermissionActivity`) lives in SystemUI and is
   not guaranteed to exist on TV builds. CaptureOne checks at runtime whether
   `MediaProjectionManager.createScreenCaptureIntent()` resolves to an activity and
   reports `native` / `unsupported` per device accordingly; when unavailable the TV app
   runs in companion mode. FLAG_SECURE and DRM video surfaces are always black. Not
   verified on physical TV hardware during this sprint.
2. **Linux window capture** grabs the window's on-screen rectangle (occluded areas appear
   as seen on screen). Wayland-only sessions are unsupported (XWayland apps only); the
   xdg-desktop-portal ScreenCast path is future work.
3. **Samsung Tizen.** Samsung's Smart TV developer FAQ states that screen capture is not
   supported in Samsung TV applications; the Tizen native screenshot API is not available
   to third-party apps. (`webapis.systemcontrol.*` belongs to Samsung's B2B signage/hospitality
   partner APIs and is not available to consumer TV apps.) The Tizen app is therefore a
   companion; recording the TV picture requires an HDMI capture card on the desktop.
4. **LG webOS.** LG's developer forum states no screen-capture API is provided to apps.
   Community reverse-engineering documents an *undocumented* SSAP call
   `ssap://tv/executeOneShot` that some firmware exposes to a paired LAN client. CaptureOne
   implements it on the desktop side as an **experimental** feature behind explicit
   on-TV pairing; it may not work on a given model, returns black for protected content,
   and was not verified against a real TV during this sprint.

## 4. Minimum OS versions

| Platform | Minimum | Feature gates |
|---|---|---|
| Windows | Windows 10 1903 (WGC from HWND/HMONITOR) | Cursor toggle 2004+, borderless capture Windows 11 / Server 2022 |
| macOS | 12.3 (ScreenCaptureKit) | System audio 13+, SCScreenshotManager 14+ |
| Linux | X11 session, PulseAudio/PipeWire-pulse for system audio | — |
| Android | 8.0 (API 26) | Internal audio API 29+, single-app capture & resize callbacks API 34+, `FOREGROUND_SERVICE_MEDIA_PROJECTION` API 34+ |
| Android TV | Android TV 8.0+ | Capture only where the consent activity exists |
| Tizen | Tizen 5.5+ TV (2020+) | Companion only |
| webOS | webOS TV 4.x+ | Companion only |

## Sources

- Android Media projection guide: https://developer.android.com/media/grow/media-projection
- Android restricted screen reading: https://source.android.com/docs/core/permissions/restricted-screen-reading
- Samsung Smart TV FAQ (other features): https://developer.samsung.com/smarttv/develop/faq/other-features.html
- LG webOS TV developer forum, "How to Capture Screen on Webos TV": https://forum.webostv.developer.lge.com/t/how-to-capture-screen-on-webos-tv/7880
- Community notes on webOS `executeOneShot`: https://gist.github.com/Informatic/1983f2e501444cf1cbd182e50820d6c1
