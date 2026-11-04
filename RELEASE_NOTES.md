# CaptureOne 0.1.0 — Release Notes (sprint build, not store-published)

First integrated build of CaptureOne from a single monorepo. No app-store submission was
performed; all binaries are unsigned or signed with test keys (see `BLOCKERS.md`).

## Packages

| Platform | Package | Produced by | Signing |
|---|---|---|---|
| Linux | `CaptureOne_0.1.0_amd64.deb`, `CaptureOne-0.1.0-1.x86_64.rpm`, `CaptureOne_0.1.0_amd64.AppImage` | local build (`artifacts/linux/`) | unsigned |
| Windows | NSIS installer `CaptureOne_0.1.0_x64-setup.exe` | GitHub Actions `windows` workflow (job `desktop`) | unsigned |
| macOS | `CaptureOne.app` / `.dmg` | GitHub Actions `macos` workflow (job `desktop-bundle`) | unsigned, not notarized |
| Android phone/tablet | `app-*.apk` | GitHub Actions `android` workflow | test key / debug |
| Android TV | `tv-*.apk` | GitHub Actions `android` workflow | test key / debug |
| Samsung Tizen | `CaptureOne-0.1.0.wgt` (+ unsigned variant) | local + GitHub Actions `tv` | throwaway author cert + Tizen default distributor cert (not installable on retail TVs) |
| LG webOS | `dev.captureone.tv_0.1.0_all.ipk` | local + GitHub Actions `tv` | n/a (Developer Mode side-load) |
| Headless recorder | `captureone-cli` (Linux/Windows/macOS) | local + CI | unsigned |

Build outputs are kept out of git (`artifacts/` is ignored except validated samples);
CI artifacts are attached to the corresponding workflow runs.

## Highlights
- Native capture per OS — Windows Graphics Capture + WASAPI loopback, macOS
  ScreenCaptureKit (video + system audio), Linux X11 (MIT-SHM/XFixes + PulseAudio
  monitor), Android MediaProjection + MediaCodec — behind one shared, contract-tested API.
- MP4 output (H.264/H.265), 24/30/60 fps, resolution and quality presets with file-size
  estimation, pause/resume, microphone + system audio mixed or as separate tracks.
- Crash-resilient recording (fragmented intermediates + recovery), disk/size/duration
  limits, post-write ffprobe/MediaExtractor validation of every file.
- Desktop app: record/library/settings/devices UI, countdown, recording indicator, region
  selector, tray, global shortcuts, SQLite history.
- TV: honest companion apps for Tizen/webOS (browse/play desktop recordings, remote
  recording control, capability report), LAN discovery and TV remote control from desktop.

## Known limitations
- Linux: X11 only (no Wayland portal yet); window capture grabs the on-screen rectangle.
- Hardware encoders were not exercised in CI/container (software x264/x265 used there).
- Windows/macOS: multi-monitor, HDR and high-DPI edge cases not tested on physical hardware.
- Android/Android TV: not executed on physical devices; Android TV capture depends on the
  device shipping the MediaProjection consent activity.
- Tizen/webOS: no system capture API exists for apps; TV screen recording requires an HDMI
  capture card on the desktop. LG one-shot screenshot is experimental and undocumented.
- Companion protocol v1 is plain HTTP on the LAN (token-authenticated, pairing-code gated);
  TLS is planned.

See `PROJECT_STATUS.md` (final acceptance table) and `TEST_REPORT.md` for exactly what was
verified on each platform.
