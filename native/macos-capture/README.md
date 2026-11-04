# macos-capture — ScreenCaptureKit backend

`macos-capture` (lib `macos_capture`) implements `capture_core::CaptureBackend` on macOS with
**ScreenCaptureKit** (SCK). It delivers BGRA frames and interleaved `f32` system-audio PCM to
`media-engine`, which encodes them (VideoToolbox encoders are probed by the engine).

```rust
#[cfg(target_os = "macos")]
let backend = macos_capture::create_backend()?;          // Arc<dyn CaptureBackend>
#[cfg(target_os = "macos")]
macos_capture::open_privacy_settings(PermissionKind::ScreenCapture)?;
```

On other operating systems the crate compiles to platform-independent helpers only, so the
workspace builds everywhere.

## Bindings: why `objc2`

The `objc2` crates (`objc2`, `objc2-foundation`, `objc2-screen-capture-kit`,
`objc2-core-media`, `objc2-core-video`, `objc2-core-graphics`, `objc2-core-audio-types`,
`objc2-av-foundation`, `block2`, `dispatch2`) are pure Rust: no build script needs the macOS
SDK, so `cargo check --target aarch64-apple-darwin` works from Linux. The `screencapturekit`
crate (v11) was rejected because its `build.rs` compiles a Swift bridge with `xcrun`/SwiftPM,
which cannot be type-checked without Xcode.

## Feature matrix

| Feature | macOS | Implementation |
|---|---|---|
| Sources (displays, windows) | 12.3+ | `SCShareableContent getShareableContentExcludingDesktopWindows:onScreenWindowsOnly:` |
| Display / window / region video | 12.3+ | `SCStream` → `SCStreamOutput` on a serial dispatch queue |
| Screenshot | 14+ | `SCScreenshotManager captureImageWithFilter:…` → `CGImage` → sRGB BGRA bitmap |
| Screenshot | 12.3–13 | one-frame `SCStream` (first complete frame); forceable with `CAPTUREONE_MACOS_FORCE_STREAM_SCREENSHOT=1` |
| System audio | 13+ | audio-only `SCStream` (`capturesAudio`, `excludesCurrentProcessAudio`) |
| Microphone permission | 10.14+ | `AVCaptureDevice authorizationStatusForMediaType:` / `requestAccessForMediaType:` (capture itself is media-engine's) |
| Capture devices (HDMI cards) | — | not implemented → `UNSUPPORTED` |
| DRM content | — | blanked by macOS (Apple TV app, FairPlay in Safari); never bypassed |

`capabilities()` reports exactly this, gated on the running OS version (see
`src/capabilities.rs`; unit-tested for 12.2/12.6/13.0/14.5/15.2).

### Ids, sizes and coordinates

* Display id = `CGDirectDisplayID` (decimal string; stable for a monitor across reboots).
  Window id = `CGWindowID`.
* `width`/`height` are **physical pixels**: display = `CGDisplayModeGetPixelWidth/Height`;
  window = frame (points) × backing scale (macOS 14+: `SCShareableContent.infoForFilter`'s
  `pointPixelScale`; older: scale of the display containing the window).
* `x`/`y` are macOS global **points** (the native virtual-desktop space), `scaleFactor` =
  pixels per point.
* Region targets are pixels relative to the display. Video uses `sourceRect` (points =
  pixels ÷ scale, output exactly the pixel size); screenshots capture the whole display and crop
  with `VideoFrame::crop` for pixel-exact results. Odd region sizes are delivered as-is.

### Streams

* Video: `pixelFormat = kCVPixelFormatType_32BGRA`, `colorSpaceName = sRGB`,
  `minimumFrameInterval = 1/fps`, `queueDepth = 6`, `showsCursor`. Display/region filters exclude
  CaptureOne's own application (its windows and audio); windows use a desktop-independent window
  filter with `scalesToFit` (+ `preservesAspectRatio`, `ignoreShadowsSingleWindow` on 14+), so
  the output size stays constant when the window is resized.
* Only `SCFrameStatus.complete` frames are delivered; idle/blank/suspended frames are skipped
  (the engine's pacer repeats the last frame, so timing is preserved). Frame timestamps come from
  the sample's presentation time on the host clock.
* `CVPixelBuffer` rows are copied honouring `bytesPerRow` into tightly packed frames.
* Termination: `stream:didStopWithError:`, `streamDidBecomeInactive:` and a 1 s CoreGraphics
  monitor (window still listed? display still online?) produce exactly one
  `StreamEvent::SourceClosed` (source gone) or `StreamEvent::Error` (permission revoked →
  `PERMISSION_DENIED`, stopped from the system menu / by macOS → `INTERRUPTED`, recoverable).
  Events are delivered from a fresh thread so a sink may call `stop()` from `on_event`.
* `StreamHandle::stop` (and `Drop`) → `stopCaptureWithCompletionHandler:` (5 s timeout), remove
  outputs, drain the sample queue; no sample reaches the sink after `stop()` returns.
* Every completion handler is bridged with a channel and a timeout (10 s for content/start),
  so a missing callback yields an error instead of a hang.

### System audio

A dedicated audio-only stream on the main display (2×2 px, 1 fps placeholder video that is
discarded) with `sampleRate` 48000 (or 44100 if requested) and `channelCount` 1–2. Keeping it
separate from the video stream means window recordings still contain the full system mix and the
engine can start/stop audio independently. SCK delivers non-interleaved Float32, converted to
interleaved `f32` (`src/audio.rs`; integer and interleaved layouts are handled too).

### Permissions

| Kind | Query | Request |
|---|---|---|
| `screenCapture` | `CGPreflightScreenCaptureAccess` → `granted` / `prompt` (not yet requested this run) / `denied` + `requiresSystemSettings` | `CGRequestScreenCaptureAccess`: macOS prompts **once per app**; afterwards only System Settings can grant it, usually followed by a relaunch |
| `systemAudio` | same grant as Screen Recording (13+), `not-applicable` before 13 | same |
| `microphone` | `AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio` | `requestAccessForMediaType:` (waits up to 300 s for the user). Refused with `INVALID_CONFIG` if the app bundle lacks `NSMicrophoneUsageDescription` (TCC would kill the process) |

`open_privacy_settings(kind)` runs `/usr/bin/open` with
`x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture` or
`?Privacy_Microphone`. macOS 15+ additionally re-confirms Screen Recording periodically for apps
that capture without `SCContentSharingPicker` (not used here).

## Verification

| What | Where | Command |
|---|---|---|
| Type-check for macOS | Linux (here) | `cargo check -p macos-capture --target aarch64-apple-darwin` (and `x86_64-apple-darwin`), `cargo clippy … --all-targets` |
| Platform-independent unit tests | Linux + macOS | `cargo test -p macos-capture` (PCM conversion, row packing, points/pixels, version gating, permission/error mapping) |
| Native build, link, tests | GitHub Actions `macos-14`, `macos-15` | `.github/workflows/macos.yml` |
| Real capture | GitHub Actions (needs the TCC grant) | `CAPTUREONE_LIVE_CAPTURE=1 cargo test -p macos-capture --test macos_live -- --test-threads=1`, then `captureone-cli` + ffprobe |

On a developer Mac: grant Screen Recording to your terminal, then run the live tests with
`CAPTUREONE_LIVE_CAPTURE=1`.

## Distribution requirements (Apple)

Builds from CI are **unsigned**. Shipping CaptureOne to users requires:

1. **Apple Developer Program membership** (organisation or individual) and a
   **Developer ID Application** certificate (exported as `.p12` with its private key).
   A *Developer ID Installer* certificate is only needed for `.pkg` installers.
2. **Hardened Runtime** enabled when signing (`codesign --options runtime`, Tauri does this when a
   signing identity is set), with an entitlements file:

   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <!-- microphone capture under the hardened runtime -->
     <key>com.apple.security.device.audio-input</key>
     <true/>
   </dict>
   </plist>
   ```

   Screen Recording has **no entitlement**: it is a TCC permission granted by the user (prompt /
   System Settings). The app is not sandboxed (Mac App Store distribution would need the App
   Sandbox and is out of scope).
3. **Info.plist** keys: `NSMicrophoneUsageDescription` (mandatory before any microphone request),
   `LSMinimumSystemVersion` = `12.3` (ScreenCaptureKit is linked strongly; in Tauri set
   `bundle.macOS.minimumSystemVersion` to `"12.3"`).
4. **Every executable inside the bundle signed** with the same identity and hardened runtime —
   including a bundled ffmpeg/ffprobe sidecar.
5. **Notarization** with `xcrun notarytool submit … --wait`, then `xcrun stapler staple` on the
   `.app`/`.dmg`, authenticated with either
   * an **App Store Connect API key** (`.p8` file, Key ID, Issuer ID) — preferred for CI; or
   * an **Apple ID** + **app-specific password** + **Team ID**.

   Tauri reads `APPLE_CERTIFICATE` (base64 `.p12`), `APPLE_CERTIFICATE_PASSWORD`,
   `APPLE_SIGNING_IDENTITY`, and `APPLE_API_ISSUER`/`APPLE_API_KEY`/`APPLE_API_KEY_PATH` or
   `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID`; store them as GitHub Actions secrets.
6. Keep the signing identity stable: TCC ties the Screen Recording grant to the code signature's
   designated requirement, so ad-hoc or re-signed builds lose the user's grant.

## Dependencies (macOS only)

| Crate | License |
|---|---|
| objc2 0.6, objc2-encode 4, objc2-foundation 0.3, block2 0.6 | MIT |
| objc2-core-foundation, objc2-screen-capture-kit, objc2-core-media, objc2-core-video, objc2-core-graphics, objc2-core-audio-types, objc2-av-foundation 0.3; dispatch2 0.3 | Zlib OR Apache-2.0 OR MIT |
| libc 0.2, bitflags 2 | MIT OR Apache-2.0 |
