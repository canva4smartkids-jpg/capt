# Windows installation

Requirements: Windows 10 version 1903 or later (Windows 11 recommended for borderless capture), x64.

1. Download `CaptureOne_<version>_x64-setup.exe` from the CI artifacts (`windows` workflow) or a release.
2. Run the installer. Builds from this sprint are **not code-signed**, so Microsoft Defender SmartScreen shows "Windows protected your PC" → *More info* → *Run anyway*. A signed release requires a code-signing certificate (see `BLOCKERS.md`).
3. The installer bundles WebView2 bootstrap (downloaded if missing) and an LGPL ffmpeg sidecar.

## Permissions
- Screen capture: no permission required (Windows Graphics Capture). Windows 10 shows a yellow capture border; Windows 11 allows hiding it.
- Microphone: *Settings → Privacy & security → Microphone → Let desktop apps access your microphone* must be on. CaptureOne's Settings → Permissions opens this page.
- Protected windows (DRM video, apps using `WDA_EXCLUDEFROMCAPTURE`) are captured as black by design.

## Uninstall
*Settings → Apps → CaptureOne → Uninstall*. Recordings in `Videos\CaptureOne` are kept.
