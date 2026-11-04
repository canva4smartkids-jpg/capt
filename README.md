# CaptureOne

Cross-platform screen capture and recording: Windows, macOS, Linux, Android phones and
tablets, Android TV / Google TV, plus companion apps for Samsung Tizen and LG webOS TVs.
Works fully offline.

- **Desktop** (Tauri 2 + React + Rust): display/window/region recording to MP4
  (H.264/H.265, 24/30/60 fps, quality presets), microphone + system audio (mixed or
  separate tracks), pause/resume, PNG/JPEG screenshots, countdown, recording indicator,
  global shortcuts, tray, recording library (preview, rename, delete, export), crash
  recovery, storage limits.
- **Android** (Kotlin + Compose): MediaProjection recording with MediaCodec, mic +
  internal audio, pause/resume, screenshots, MediaStore library, share.
- **Android TV / Google TV**: D-pad UI; native capture where the device allows it,
  otherwise companion mode.
- **Samsung Tizen / LG webOS**: honest companion apps (these platforms give apps no
  screen-capture API): browse and play desktop recordings, remote-control desktop
  recording (e.g. of the TV via an HDMI capture card), device/capability info.

## Documents

| Document | Contents |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System design, shared contract, pipelines, security model |
| [PLATFORM_CAPABILITIES.md](PLATFORM_CAPABILITIES.md) | What each platform supports (native / partial / companion / external hardware / unsupported) |
| [PROJECT_STATUS.md](PROJECT_STATUS.md) | Progress reports and the final per-platform acceptance table |
| [TEST_REPORT.md](TEST_REPORT.md) | Test suites, results and validated media evidence |
| [BLOCKERS.md](BLOCKERS.md) | External blockers and distribution requirements |
| [RELEASE_NOTES.md](RELEASE_NOTES.md) | Release contents and known limitations |
| [docs/](docs/) | Installation per platform, release/licensing audit, engineering rules |

## Quick start (development)

```bash
pnpm install
```

```bash
pnpm -r test
```

```bash
cargo test --workspace
```

Desktop app (Linux dev; needs the Tauri Linux prerequisites and `ffmpeg` on `PATH`):

```bash
pnpm --filter ./apps/desktop tauri dev
```

Android (needs an Android SDK with platform 35 and build-tools 35.0.1):

```bash
cd apps/android && ./gradlew assembleDebug
```

See [docs/installation](docs/installation) for end-user installation per platform.

## Repository layout

```
apps/       desktop (Tauri), android, android-tv, tv-web, samsung-tv, lg-tv
native/     capture-core, media-engine, linux/windows/macos capture backends, android-capture, capture-cli
packages/   capture-contracts (source of truth), settings, design-system, media-metadata, shared-types
services/   companion-hub, device-discovery
tests/      contract fixtures and integration suites
docs/       architecture, platform-capabilities, installation, release
```
