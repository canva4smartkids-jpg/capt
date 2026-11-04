# CaptureOne — Project Status

Sprint start: 2026-11-04 12:52 UTC. Reports are appended newest-first.

**Final state (2026-11-04 ~14:50 UTC):** all workstreams finished and pushed. Real capture +
MP4 recording verified on Linux (container + CI), macOS 14 and Windows (GitHub-hosted
runners) and Android (API 34 emulator in CI). All CI workflows green (`ci-contracts`,
`linux-engine`, `windows`, `macos`, `android`, `tv`). Sprint ended early at the user's request
to conserve credits.

---

## Final acceptance (per platform)

Legend: **verified** = executed and the output inspected; **unverified** = code written/built
but not executed on that platform; CI = GitHub-hosted runner.

| Platform | Source | Build | Installation | Screenshot | Video recording | Audio capture | Tests | Outstanding limitations |
|---|---|---|---|---|---|---|---|---|
| Linux (X11) | complete | passed (deb/rpm/AppImage from final commit) | **verified** (.deb clean-installed in ubuntu:24.04, libraries resolved) | **verified** (PNG/JPEG, region) | **verified** (H.264/H.265, 24/30/60, pause, limits, recovery) | **verified** (Pulse monitor + cpal mic, mixed/separate) | passed (25 integration + 60 unit) | X11 only; HW encoders untested |
| macOS | complete | passed (CI macos-14 native build + Tauri app/dmg bundle) | unverified | **verified on CI runner** (SCK 1920x1080, region, legacy path) | **verified on CI runner** (CLI H.264 5.03 s 151 frames; region 640x360) | **verified on CI runner** (SCK system audio AAC 48 kHz); mic unverified | passed (40 unit + 6 live) | window capture/close pending; macOS 15, Intel runtime, VideoToolbox, signing/notarization |
| Windows | complete | passed (CI MSVC build; NSIS installer built in CI) | unverified (installer not run) | **verified on CI runner** (WGC display 1024x768, region 200x120, window) | **verified on CI runner** (WGC → CLI H.264 1024x768 5 s 150 frames, validated; SourceClosed on window close) | unverified (runner has no audio device; backend reports it honestly); mic unverified | passed (27 unit + 18-step smoke) | WASAPI loopback, multi-monitor/HDR/high-DPI, HW encoders, signing |
| Android phone/tablet | complete | passed (CI: debug + release-unsigned APKs, lint) | verified on emulator (instrumented install, API 34) | **verified on emulator** (MediaProjection PNG 320x640) | **verified on emulator** (MediaProjection → MediaCodec H.264, pause excluded, 3.157 s, MediaStore) | unverified (emulator has no audio); synthetic AAC path verified | passed (89 JVM + 5 instrumented) | physical devices; mic/internal audio; Play signing |
| Android TV / Google TV | complete | passed (CI: tv APKs, leanback manifest verified) | unverified (no TV emulator/hardware) | unverified | unverified (same engine as phone; device-dependent consent activity) | unverified | passed (JVM) | real TV hardware; consent activity availability |
| Samsung Tizen | complete (companion) | passed (.wgt signed with test certs, unsigned variant) | unverified (no TV) | unsupported (platform) | unsupported (platform; HDMI capture via desktop) | unsupported | passed (44 web + 28 E2E in Chromium) | Samsung certificate needed for retail TVs |
| LG webOS | complete (companion) | passed (.ipk) | unverified (no TV) | experimental desktop-side SSAP screenshot, unverified | unsupported (platform; HDMI capture via desktop) | unsupported | passed (shared with Tizen) | Dev Mode / LG account; SSAP pairing on real firmware |
| Companion hub + discovery | complete | passed | n/a | n/a | n/a | n/a | passed (58) | no real TVs; HTTP (no TLS) |

---

## Report #2 — 2026-11-04 ~14:25 UTC (T+1:30)

### Actual working functionality (verified)
- **Desktop capture + MP4 recording works on Linux X11** (Priority 1 met on the one desktop OS available here): display/region/window capture, H.264 and H.265, 24/30/60 fps, resolution scaling, pause/resume, PNG/JPEG screenshots, system audio and microphone (mixed and separate tracks), A/V sync within ~10–20 ms, crash recovery after `kill -9`, duration/size/disk-full limits, source-closed handling, flat memory. 25/25 integration + 60/60 unit tests; every output checked with ffprobe. Independently re-checked by the coordinator (see `TEST_REPORT.md` §2). Samples in `artifacts/samples/`.
- **TV companion stack** works in headless Chromium against a real hub instance (28/28 E2E steps); hub + discovery 58/58 tests; `.wgt` and `.ipk` packages built.

### Implemented, compile-verified only
- Windows Graphics Capture + WASAPI loopback backend (MSVC type-check, mingw full link, 27 unit tests).
- macOS ScreenCaptureKit backend (aarch64/x86_64 type-check, 40 unit tests).

### Build / CI results
- `ci-contracts`: green. `windows`, `macos`, `linux-engine`: first runs **failed** → the respective workstreams are now debugging on the real runners (Windows/macOS runtime verification depends on these runs).

### Bugs fixed (highlights)
- Engine: disk-guard false positives when the shared disk filled; resize handling; recovery trims partial last fragment.
- TV: SSDP merge dropping devices with the same description URL; empty thumbnails for stills; duplicate mDNS loopback record.
- CI: in-progress runs were being cancelled by frequent pushes → queue instead.

### Remaining
Desktop app (Tauri UI, Linux bundles, E2E, hub integration) and Android (APK builds) still running; Windows/macOS/Linux CI to green; final docs, licensing audit, acceptance.

### Blockers
Unchanged (`BLOCKERS.md`). New: MSVC cross-link needs `aka.ms` / `download.visualstudio.microsoft.com` (blocked) — CI builds MSVC instead.

---

## Report #1 — 2026-11-04 ~13:20 UTC (T+0:30)

### Environment inspection
- Build host: Ubuntu 24.04 x86_64, 4 cores, 15 GB RAM, ~31 GB free disk.
- Available: Rust 1.97, Node 22 / pnpm 10, JDK 21, Gradle 8.14.3, ffmpeg 6.1 (x264/x265), Docker, Xvfb, PulseAudio, clang/lld, NSIS.
- Android SDK: `dl.google.com` is blocked → SDK extracted from Docker image `cimg/android:2025.10` into `/opt/android-sdk` (platforms 34/35, build-tools 35.0.1). No KVM → no emulator.
- Not available: Windows, macOS, physical Android/TV devices, Tizen/LG official SDK downloads (see `BLOCKERS.md`).
- GitHub Actions is enabled on the repository → Windows/macOS builds and capture tests can run on hosted runners.

### Implemented
- Monorepo (pnpm + Cargo workspaces, Gradle root planned at `apps/android`).
- `packages/capture-contracts`: complete shared contract (capabilities, permissions, sources, recording config + state machine, status, screenshots, media metadata + validation, errors, events, CaptureService/RecordingLibrary interfaces, companion protocol, desktop IPC). JSON Schema export (16 schemas + enums + transitions).
- `packages/settings`: `presets.json` single source of truth; resolution, bitrate and file-size estimation; file-name templates; `AppSettings` schema with defaults and corrupt-data fallback.
- `native/capture-core`: Rust mirror of the contract, `CaptureBackend` trait, frame/PCM types (crop, BGRA→RGBA), presets port, state machine, ENOSPC→STORAGE_FULL mapping.
- Cross-language contract tests (Rust validated against the TS-generated JSON Schemas, enum spellings, transition table, 155 preset fixtures).
- Root CI workflow `ci-contracts`.
- `ARCHITECTURE.md`, `PLATFORM_CAPABILITIES.md`, `BLOCKERS.md`, engineering rules for parallel workstreams.

### Actually working (verified)
- Contract TS tests: 13/13 pass; settings tests: 14/14 pass; capture-core: 11/11 pass (8 contract + 3 unit) — locally **and** on GitHub Actions (run `ci-contracts` #1: success).

### In progress (parallel workstreams launched)
| Workstream | Scope |
|---|---|
| engine | media-engine (ffmpeg pipeline, pacer, A/V timelines, fragmented MP4, ffprobe validation, recovery), X11 backend, CLI, Xvfb integration tests |
| desktop-app | Tauri 2 shell + React UI (Record, Library, Settings, Devices, indicator, region selector), SQLite library, tray, shortcuts, Linux bundles |
| windows | Windows Graphics Capture + WASAPI loopback backend, Windows CI with real capture test |
| macos | ScreenCaptureKit backend (video + audio), macOS CI |
| android | Kotlin capture library, phone/tablet app, Android TV app, APKs, unit tests |
| tv | Tizen/webOS companion apps (shared web app), device discovery (SSDP/mDNS, LG SSAP, Samsung remote), companion hub server |

### Blockers
See `BLOCKERS.md` (E1–E7). None stop source implementation.

### Next actions
Integrate workstream outputs as they land; push to trigger platform CI; inspect Windows/macOS runner results; wire the companion hub into the desktop app.
