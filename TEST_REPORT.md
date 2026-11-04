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

Not verified on Linux: hardware encoders (none functional in the container), multiple
physical monitors (Xvfb exposes one RandR monitor), Wayland, a physical microphone.

## 3. Windows backend (Windows Graphics Capture + WASAPI loopback)

| Check | Where | Result |
|---|---|---|
| `cargo check` / `clippy -D warnings` for `x86_64-pc-windows-msvc` | container | pass |
| Unit tests (PCM conversion, padding removal, ids, OS-build gating, region crop, QPC mapping, pacer) | container | 27/27 pass |
| Full link (`x86_64-pc-windows-gnu`, mingw-w64) of smoke tool + `captureone-cli` | container | PE32+ executables produced (not run) |
| MSVC cross-link via cargo-xwin | container | blocked (`aka.ms`, `download.visualstudio.microsoft.com` denied by network policy) |
| Native build, tests and real WGC capture/recording on `windows-latest` | GitHub Actions | **in progress** — first runs failed; being fixed |

## 4. macOS backend (ScreenCaptureKit)

| Check | Where | Result |
|---|---|---|
| `cargo check` / `clippy -D warnings` for `aarch64-apple-darwin` and `x86_64-apple-darwin` | container | pass |
| Unit tests (PCM de-interleave, bytesPerRow copy, points↔pixels, OS gating, error mapping) | container | 40/40 pass |
| Native build/link, live SCK tests, CLI recording | GitHub Actions `macos-14`/`macos-15` | **in progress** — first run failed; being fixed. Capture depends on whether the runner accepts a TCC grant |

## 5. TV companions, discovery and hub

| Check | Result |
|---|---|
| `cargo test -p companion-hub -p device-discovery` | 58/58 pass (real sockets: pairing good/bad/lockout, byte-exact Range fetches, command round trip, WebSocket broadcast, schema validation; fake SSDP / LG SSAP / Samsung WS servers) |
| `clippy -D warnings` | clean |
| tv-web unit tests (vitest) | 44/44 pass; `tsc` clean; bundle parses as ES2015 |
| End-to-end in headless Chromium against a real hub (`node apps/tv-web/scripts/smoke.mjs`) | 28/28 pass using only remote keys: pairing (wrong code refused), live WebSocket status, thumbnails, 24 s H.264 playback with +10 s seek, remote start/pause/stop, Back keys 461/10009, 1280x720 scaling. The hub's recording control was the demo backend (simulated) — proves the control path only |
| Packages | signed `.wgt` (throwaway author cert + Tizen default distributor cert; SHA-512 digests verified), unsigned `.wgt`, `.ipk`; both boot in Chromium from extracted contents |
| Real TVs | **not tested** — no hardware |

## 6. Pending sections
Desktop app (Tauri UI + E2E), Android (unit tests, APK builds, CI), CI results for
Windows/macOS/Linux/TV/Android workflows.
