# CaptureOne — Project Status

Sprint start: 2026-11-04 12:52 UTC. Reports are appended newest-first.

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
