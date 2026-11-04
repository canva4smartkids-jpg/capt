# CaptureOne — Test Report

Living document. Every "verified" entry names the command and the observed evidence.
"Compiles" is never reported as "works".

Last updated: 2026-11-04 ~14:20 UTC (report #2). Final numbers are in the acceptance
table of `PROJECT_STATUS.md`.

## 1. Shared contract (all platforms)

| Suite | Command | Result |
|---|---|---|
| TS contract tests | `pnpm --filter @captureone/capture-contracts test` | 13/13 pass |
| Settings/presets tests | `pnpm --filter @captureone/settings test` | 14/14 pass |
| Rust contract tests (Rust ↔ TS JSON Schemas, enums, transitions, 155 preset fixtures) | `cargo test -p capture-core` | 11/11 pass |
| Schema/fixture drift | CI `ci-contracts`: regenerate + `git diff --exit-code` | pass (GitHub Actions run 41836450286) |

## 2. Desktop engine on Linux X11 — **real capture, verified**

Environment: Xvfb 1920x1080x24, PulseAudio null sink with test tone, ffmpeg 6.1.1
(libx264/libx265), moving content (`ffplay testsrc2`, `xterm -e top`).

Command: `tests/integration/desktop/run.sh --memory` (engine workstream) → 25/25
integration tests pass + memory probe pass; unit tests 60/60 (media-engine 56,
linux-capture 2, CLI 2).

| Scenario | Evidence (ffprobe / decode) |
|---|---|
| H.264 5 s @30 fps, 1920x1080 | duration 5.000 s, h264/avc1, 1920x1080, 150 frames, 0 dropped, full decode clean |
| H.265 | hevc tagged `hvc1`, 1280x720, 4.000 s, 120 frames |
| Resolution `--height 720` | 1280x720 |
| Region | exactly 640x360; odd 333x201 → 332x200 (even dims for 4:2:0) |
| Pause/resume (2 s rec, 2 s pause, 2 s rec) | 4.000 s, 120 frames, 6.3 s wall time |
| 60 fps | 240 frames, 60/1, 0 dropped |
| Screenshots PNG/JPEG | 1920x1080 decode, 78/150 distinct sampled colours; region 800x600 exact |
| System audio (PulseAudio monitor) | AAC 48 kHz stereo, 5.03 s, mean −22.9 dB (non-silent) |
| Microphone (cpal → ALSA pulse → null-sink monitor) | −22.9 dB; selectable by id `alsa:pulse` |
| Separate tracks | 2 AAC tracks: "Microphone", "System audio" |
| Mixed track | 1 AAC track, −18.9 dB (two tones summed) |
| A/V sync (flash + beep onsets) | audio offset +9.7 ms on all 7 onsets (other runs +16/+19 ms) |
| `kill -9` at 7 s → `recover` | valid 6.81 s 1280x720 MP4 with audio, clean decode; recover idempotent |
| Max duration 3 s | stop reason `max-duration`, 3.000 s |
| Max size 6 MB | stop reason `max-file-size`, 5.06 MB |
| Real disk full (40 MB tmpfs) | stop reason `low-disk`, valid 11.7 s file |
| Insufficient free space at start | refused with `STORAGE_FULL` |
| Captured window killed mid-recording | stop reason `source-closed`, valid 3.8 s file |
| Captured window resized twice | output stays 484x316, valid |
| Memory, 45 s 1080p30 + audio | CLI RSS 41.2 MB flat (no growth); ffmpeg children ~426 MB steady |

**Independent coordinator re-check** (fresh Xvfb `:91` 1600x900, `captureone-cli record
--target display:all --duration 4 --fps 30 --codec h264`): exit 0, `validation.valid:
true`; ffprobe `h264 1600x900, 120 frames, 4.000000 s`; full decode clean; extracted last
frame shows the live `top` output including the recorder's own `ffmpeg` and
`captureone-cli` processes at 14:17:23 — i.e. genuine live screen content, not a
synthetic source.

Committed samples (`artifacts/samples/`, re-probed by the coordinator): H.264 720p with
system audio (5.03 s, 150 frames + AAC 48 kHz stereo), H.265 720p (`hvc1`, 120 frames),
PNG and JPEG 800x600 region screenshots — all decode without errors.

**Linux CI (GitHub `ubuntu-latest`)**: `linux-engine` run 41848195843 (job 126639314263) — all
steps success: 60 unit tests, release build, Xvfb + PulseAudio integration step (passes only
if all 25 integration tests + memory probe pass). Earlier run 41846405404 memory probe: CLI
33.3 MB, 0.0 MB growth over 40 s, 1200 frames, 0 dropped.

Not verified on Linux: hardware encoders (none functional in the container), multiple
physical monitors (Xvfb exposes one RandR monitor), Wayland, a physical microphone.

## 3. Windows backend (Windows Graphics Capture + WASAPI loopback)

| Check | Where | Result |
|---|---|---|
| `cargo check` / `clippy -D warnings` for `x86_64-pc-windows-msvc` | container | pass |
| Unit tests (PCM conversion, padding removal, ids, OS-build gating, region crop, QPC mapping, pacer) | container | 27/27 pass |
| Full link (`x86_64-pc-windows-gnu`, mingw-w64) of smoke tool + `captureone-cli` | container | PE32+ executables produced (not run) |
| MSVC cross-link via cargo-xwin | container | blocked (`aka.ms`, `download.visualstudio.microsoft.com` denied by network policy) |
| **Backend on a real Windows runner** (`windows-latest`, Windows Server, Hyper-V 1024x768 display, WARP) | GitHub Actions run 41848095895, job 126637005021 — all 18 steps **success** (coordinator re-checked step conclusions) | unit tests + clippy on MSVC; display `\\.\DISPLAY1` 1024x768 enumerated; WGC display screenshot PNG 1024x768 (ffprobe exact); region PNG exactly 200x120; Notepad window found by enumeration and captured (754x512); 5 s WGC stream → `h264 1024x768, 5 s, 150 frames` (95 backend frames, real content: 764 distinct sampled colours); closing Notepad → `SourceClosed`, clean `stop()`; 3 start/stop cycles + drop without stop clean |
| **`captureone-cli` on Windows** | run 41841763518, job 126619406516 — success | `sources`, `screenshot`, `record --duration 5 --codec h264` → ffprobe `h264 1024x768, 5 s, 150 frames, 741 994 bytes`; engine `validation.valid: true` |
| Desktop NSIS installer | run 41841763518 job `desktop` — success, installer uploaded | built, not installed/run |
| WASAPI loopback | hosted runner has **no audio device** → backend correctly returns `SOURCE_NOT_FOUND`, reports systemAudio `unsupported` | **unverified** (needs a machine with an audio endpoint) |

## 4. macOS backend (ScreenCaptureKit)

| Check | Where | Result |
|---|---|---|
| `cargo check` / `clippy -D warnings` for `aarch64-apple-darwin` and `x86_64-apple-darwin` | container | pass |
| Unit tests (PCM de-interleave, bytesPerRow copy, points↔pixels, OS gating, error mapping) | container | 40/40 pass |
| Native build/link + unit tests + clippy, x86_64 link | GitHub Actions `macos-14` (run 41846405670, job 126631263716) | **pass** |
| **Live ScreenCaptureKit tests on a real Mac (hosted runner, macOS 14.8.9, 1920x1080)** | same job | **pass — 6/6**: display screenshot 1920x1080 (480 ms), region stream 640x360, display stream, system audio (102 buffers, 48 kHz stereo ≈2.04 s), permission `Granted` (runner image ships Screen Recording grants; CI adds TCC rows for its binaries), unknown source → `SOURCE_NOT_FOUND`; forced macOS 12.3–13 single-frame screenshot path 1/1 |
| **`captureone-cli` on macOS + ffprobe assertions** | same job (steps 14–15 succeeded; assertions fail the step on mismatch) | screenshot PNG 1920x1080; `record --duration 5` → h264 1920x1080 5.03 s, 151 frames, 0 dropped; `record --system-audio` → h264 + AAC 48 kHz stereo; `record --target region:1:0,0,640,360` → h264 640x360 3.00 s (SCK `sourceRect`) — engine validation `valid: true` for all |
| Window capture + window-closed → `SourceClosed` | GitHub Actions (commit c1e0602) | pending |
| macos-15 runner | GitHub Actions | pending (runner queue) |
| VideoToolbox hardware encoder | — | not exercised (engine chose libx264 on the runner VM) |

## 5. TV companions, discovery and hub

| Check | Result |
|---|---|
| `cargo test -p companion-hub -p device-discovery` | 58/58 pass (real sockets: pairing good/bad/lockout, byte-exact Range fetches, command round trip, WebSocket broadcast, schema validation; fake SSDP / LG SSAP / Samsung WS servers) |
| `clippy -D warnings` | clean |
| tv-web unit tests (vitest) | 44/44 pass; `tsc` clean; bundle parses as ES2015 |
| End-to-end in headless Chromium against a real hub (`node apps/tv-web/scripts/smoke.mjs`) | 28/28 pass using only remote keys: pairing (wrong code refused), live WebSocket status, thumbnails, 24 s H.264 playback with +10 s seek, remote start/pause/stop, Back keys 461/10009, 1280x720 scaling. The hub's recording control was the demo backend (simulated) — proves the control path only |
| Packages | signed `.wgt` (throwaway author cert + Tizen default distributor cert; SHA-512 digests verified), unsigned `.wgt`, `.ipk`; both boot in Chromium from extracted contents |
| GitHub Actions `tv` workflow | **pass** — run 41847053140: Rust 58/58, tv-web 44/44, contracts 13/13, headless smoke 28/28, signed `.wgt` (5 author + 6 distributor digests verified), unsigned `.wgt`, `.ipk` built + inspected; artifacts `tizen-wgt`, `webos-ipk`, `tv-web-dist`, `demo-hub`, `tv-screenshots` |
| Real TVs | **not tested** — no hardware |

## 6. Desktop app (Tauri 2 + React) on Linux

| Check | Result |
|---|---|
| UI unit tests (vitest) | desktop 63, design-system 15, media-metadata 10, shared-types 4 — all pass; `pnpm typecheck` clean |
| `cargo test -p captureone-desktop` | 31 pass (library CRUD, settings fallback, IPC payloads vs contract JSON Schemas, every contract command implemented, hub adapter: start → pair over HTTP → token-protected listing/download → regenerate → restart) |
| Bundles | `.deb` 9.6 MiB, `.rpm` 9.6 MiB, `.AppImage` 85 MiB rebuilt from the final commit (`artifacts/linux/`, `SHA256SUMS`) |
| **E2E of the built release app under Xvfb** (`apps/desktop/e2e/run-linux-e2e.sh`, tauri-driver + WebKitWebDriver) | Record page with real thumbnails + capability report (libx264/libx265 verified, HW encoders rejected) → Start → 3 s countdown → recording with live stats + indicator window → Pause → Resume → Stop → "Recording saved / Validated"; ffprobe: **h264 1600x1000 30 fps, 7.70 s**; Screenshot → validated PNG; Library grid/list with thumbnails, in-app video preview plays (7.7 s); Settings diagnostics (ffmpeg 6.1.1). Screenshots in `docs/screenshots/desktop-*.png` (coordinator viewed `desktop-library.png`: validated 7 s H.264 + PNG items) |
| **Final E2E re-run on the rebuilt release binary (coordinator, 15:32 UTC)** | exit 0 — full flow above (display recording h264 1600x1000, 7.80 s) **plus** Devices page, region overlay drag → "480 × 300 px" → region recording **h264 480x300, 30 fps, 4.13 s** (exactly the selected region). Fixed the harness to accept the region window closing on Enter. Two intermediate attempts were cut short by WebKitWebDriver dropping its connection (tauri-driver `hyper IncompleteMessage`) — test-infrastructure flake; the app's log showed clean recordings in those runs |
| **Clean-install check** | `.deb` installed with `apt-get install` into a fresh `ubuntu:24.04` container: exit 0, package `capture-one 0.1.0` installed, ffmpeg pulled in as a dependency, `ldd /usr/bin/captureone-desktop` → all libraries resolved, `CaptureOne.desktop` menu entry installed. (`.rpm`/`.AppImage` rebuilt from the final commit; not install-tested) |
| Windows/macOS desktop bundles | built by CI jobs (`windows` → NSIS, `macos` → app/dmg; macOS bundle job passed in run 41841621055) |

## 7. Android phone/tablet + Android TV

| Check | Where | Result |
|---|---|---|
| JVM unit tests (`:capture` 72, `:app` 8, `:tv` 9) incl. contract tests vs shared JSON Schemas/fixtures | container + CI | 89/89 pass |
| Lint (`abortOnError = true`) for `:capture`, `:app`, `:tv` | CI | pass |
| AGP/Compose APK build | GitHub Actions `android` run 41849443116 (coordinator confirmed `success`) | `app-debug.apk`, `app-release-unsigned.apk`, `tv-debug.apk`, `tv-release-unsigned.apk` (artifact `android-apks`); debug APKs pass `apksigner verify` |
| `aapt2 dump badging` | CI | ids `dev.captureone.android` / `dev.captureone.tv`, targetSdk 35; FGS + FGS_MEDIA_PROJECTION + FGS_MICROPHONE, POST_NOTIFICATIONS, RECORD_AUDIO; `CaptureService foregroundServiceType=0xa0` (mediaProjection\|microphone); TV: leanback required + leanback launcher banner |
| **Emulator instrumentation (API 34 google_apis x86_64, KVM)** | same run | **5/5 pass**: synthetic Surface → `c2.android.avc.encoder` + AAC + muxer with pause → h264 640x360 + aac 48 kHz stereo, 4.05 s; **real MediaProjection recording** through `CaptureService` (consent pre-authorised via `appops PROJECT_MEDIA`) with a 1.5 s pause → h264 320x640, 46 frames, 3.157 s (paused time excluded), published to MediaStore; **MediaProjection screenshot** PNG 320x640 published to `Pictures/CaptureOne`; presets asset packaged; capability report round-trips |
| Bug found by the emulator and fixed | `ef939f3` | `Regex("(?U)\\s+")` rejected by Android ICU → class init failure would have crashed every save on devices; JVM tests could not catch it; fixed + regression test + `LinkageError` containment |
| Microphone / internal audio | — | **unverified** (emulator runs `-noaudio`) |
| Physical phones/tablets, Android TV / Google TV hardware | — | **unverified** (no hardware) |

## 8. Capture-card (HDMI/UVC) recording — desktop

Implemented once in `media-engine` via ffmpeg device demuxers (Linux v4l2, Windows dshow,
macOS avfoundation), feeding the existing pacer/encoder/mux/validation pipeline.

| Check | Result |
|---|---|
| Parser/enumeration/arg-builder unit tests (dshow + avfoundation list output fixtures, sysfs in temp dir) | pass (part of media-engine 63/63) |
| Device pipeline through the CLI with the env-gated test seam (`CAPTUREONE_TEST_DEVICE_INPUT`, lavfi source standing in for the card) | 5 new integration tests pass (listing + capability, 4 s record → ffprobe h264 1280x720 30 fps 120 packets 4.0 s valid, pause/resume, PNG screenshot 1280x720, missing device error); full suite 30/30 |
| Desktop UI "Capture devices" tab + hub `capture-device` preset | vitest 66/66, `cargo test -p captureone-desktop` 32/32 |
| Real capture hardware (v4l2/dshow/avfoundation) | **unverified** — no capture device available; Windows/macOS listings verified only against fixture text |

## 9. Security review and license audit

`docs/release/SECURITY_REVIEW.md`: 5 Medium + 3 Low findings fixed with regression tests
(hub WebSocket/connection caps, pairing-failure table bound, output-directory validation,
export destination validation, untrusted recovery manifest, loopback media server limits,
request-line validation in discovery, Android hub address restricted to LAN). Residual:
companion protocol is plain HTTP on the LAN; pairing code not rotated after pairing.
`docs/release/LICENSES-AUDIT.md` + `THIRD_PARTY_NOTICES.md`: 660 entries; no GPL/AGPL;
5 MPL-2.0 crates (Tauri tooling, unmodified); EPL-1.0 JUnit test-only. `cargo audit` /
`pnpm audit` run in CI (`ci-contracts` → `audit` job).


## 10. CI status at hand-off (final commit 129ef03)

| Workflow | Result |
|---|---|
| `ci-contracts` (TS + Rust contract tests, schema drift, clippy, **cargo audit + pnpm audit**) | success — run 41862538632 |
| `tv` | success — run 41862538685 |
| `windows` | success on a0b4c09 (run 41859040424); later commits only touched capture-core test code + docs |
| `android` | success on ef939f3 (run 41849443116, incl. API 34 emulator MediaProjection tests); later commits did not touch Android code paths covered by the workflow |
| `linux-engine` | in progress at hand-off (run 41862538424); last completed run success (41855495856) |
| `macos` | queued for a hosted macOS runner at hand-off (run 41862538958); last completed native+capture run success (41846405670); the run on ef3ceca was cancelled before finishing |
