# Third-party notices

CaptureOne is built from the components below. The complete machine-generated inventory (every Rust crate, npm package and declared Android library, with SPDX expressions) is in [`LICENSES-AUDIT.md`](LICENSES-AUDIT.md) and `licenses-audit.json`; regenerate with `python3 scripts/license-audit.py`.

## What ships

* **Rust crates** (Tauri 2, axum/tokio, rustls, rusqlite, image, roxmltree, serde and their dependencies): MIT, Apache-2.0, BSD, Zlib, Unicode-3.0, Unlicense, or dual combinations. License texts are in each crate's source and are collected into the installer's notices file at release time.
* **Web UI** (React, React DOM, Scheduler, Zod, Tauri JS API, Tauri dialog plugin): MIT or Apache-2.0.
* **Android apps** (Kotlin, kotlinx.coroutines/serialization, OkHttp, AndroidX, Jetpack Compose, Media3, TV Material): Apache-2.0.
* **FFmpeg / FFprobe** (desktop sidecar, separate process): LGPL-2.1-or-later. Only builds configured without GPL or nonfree components are bundled (`apps/desktop/scripts/fetch-ffmpeg.mjs` enforces this). Releases must include the FFmpeg license text and a link to the exact source of the shipped build.
* **System libraries** used through dynamic linking or the OS: libpulse (LGPL-2.1+), WebKitGTK (LGPL-2.1+), Microsoft WebView2 Runtime (Microsoft license, installed separately), Apple system frameworks.

## Copyleft notes

* Five Rust crates are MPL-2.0 (`cssparser`, `cssparser-macros`, `dtoa-short`, `selectors`, `option-ext`), pulled in by Tauri's tooling and webview stack. MPL-2.0 is file-level copyleft: it only requires that modifications to those files be published. CaptureOne uses them unmodified, so the obligation is satisfied by retaining their license notices and pointing to their upstream source.
* `junit` (EPL-1.0) is test-only and never packaged.
* No GPL or AGPL dependency was found. Do not ship an FFmpeg build containing x264/x265/fdk-aac.
* The workspace package metadata is `UNLICENSED` and the repository has no top-level LICENSE file; the project's own license must be chosen before publication.

## Build-time tools (not distributed)

Tizen Studio and webOS CLI (packaging of the TV web apps), Vite, TypeScript, Vitest, Gradle/AGP, Android SDK.
