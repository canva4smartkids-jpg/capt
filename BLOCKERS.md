# CaptureOne — Blockers and External Requirements

Genuine external blockers encountered during the sprint, with the exact missing
requirement. Engineering continued around each one (source code, CI workflows, docs).

## Environment blockers (sprint build host: Ubuntu 24.04 x86_64 container)

| # | Blocker | Impact | Workaround in place | What is needed to clear it |
|---|---|---|---|---|
| E1 | No Windows machine | Windows backend cannot be run locally | Cross type-check (`x86_64-pc-windows-msvc`), real build + capture test on GitHub Actions `windows-latest` | A Windows 10 1903+/11 PC for interactive testing (multi-monitor, HDR, high-DPI, WASAPI devices) |
| E2 | No macOS machine | ScreenCaptureKit backend cannot be run locally | Cross type-check (`aarch64/x86_64-apple-darwin`), native build/tests on GitHub Actions `macos-14` | A Mac with macOS 13+ (14+ for screenshots API) to grant Screen Recording permission and test interactively |
| E3 | `dl.google.com` blocked by network policy | `sdkmanager` cannot install Android SDK packages | SDK (platforms 34/35, build-tools 35.0.1) extracted from the public `cimg/android` Docker image | — (resolved for building; emulator images still unavailable) |
| E4 | No KVM / no Android emulator / no Android device | MediaProjection capture cannot be executed | APKs built and an API 34 emulator instrumentation job (real MediaProjection recording) run on GitHub Actions; JVM tests here | Physical Android 10+ phone/tablet and an Android TV / Google TV device (e.g. Chromecast with Google TV, Android 12+) |
| E8 | Google Maven artifacts are served from `dl.google.com` (blocked) | AGP/AndroidX/Compose cannot resolve in the container | Real APK builds run on GitHub Actions; engine compiled locally against `android.jar` | — (CI covers it) |
| E5 | `download.tizen.org` and Samsung/LG developer sites blocked | Official Tizen Studio / webOS SDK installers unavailable | Docker Hub `vitalets/tizen-webos-sdk` image (rate-limited) and `@webos-tools/cli` from npm | — |
| E6 | No physical Samsung / LG TVs | TV companion apps, LG SSAP and Samsung remote clients cannot be tested against real TVs | Fake SSDP/SSAP/Samsung WebSocket servers in tests; headless Chromium smoke tests of the TV UI | A Samsung Tizen TV (2020+) and an LG webOS TV (4.x+) in developer mode on the same LAN |
| E7 | GitHub release asset downloads and `api.github.com` blocked from the container | Cannot fetch prebuilt binaries (ffmpeg builds) from GitHub here | Linux uses system ffmpeg; release workflows fetch ffmpeg on CI runners | — |

## Distribution requirements (not obtainable by an agent)

| Platform | Requirement |
|---|---|
| Windows | Code-signing certificate (OV/EV, or Azure Trusted Signing) to avoid SmartScreen warnings; Microsoft Store account if Store distribution is wanted |
| macOS | Apple Developer Program membership: Developer ID Application certificate, notarization credentials (App Store Connect API key or Apple ID + app-specific password), hardened runtime |
| Android | Release keystore owned by the publisher (a throwaway key is generated outside the repo for test builds); Google Play Console account; Play policy declaration for MediaProjection/foreground service |
| Android TV | Same as Android + Play TV review (leanback banner, D-pad navigation) |
| Samsung Tizen | Samsung account + Samsung certificate (author + distributor) from Tizen Studio Certificate Manager; Samsung Seller Office for store submission |
| LG webOS | LG developer account, Developer Mode app on the TV for side-loading; LG Seller Lounge for store submission |

No app-store submission was performed during this sprint.
