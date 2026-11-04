# CaptureOne for Android (phone/tablet, Android TV / Google TV)

| Gradle path | Directory | What |
|---|---|---|
| `:app` | `apps/android/app` | Phone/tablet app, Jetpack Compose + Material 3, id `dev.captureone.android` |
| `:tv` | `apps/android-tv` | Android TV / Google TV app, Compose for TV (`androidx.tv:tv-material`), id `dev.captureone.tv` |
| `:capture` | `native/android-capture` | Android library: capture engine, contract types, library DB, companion client |
| `:harness` (host mode only) | `native/android-capture/harness` | Framework-only engine harness APK (no AndroidX), for on-device engine checks |

minSdk 26, targetSdk/compileSdk 35, build-tools 35.0.1, AGP 8.13.0, Kotlin 2.2.21, Gradle 8.14.3 (wrapper).

## Two build modes

**Default (AGP) — the real apps.** Needs Google Maven (`dl.google.com/dl/android/maven2`). Used by CI
(`.github/workflows/android.yml`):

```bash
cd apps/android
./gradlew :capture:testDebugUnitTest :app:testDebugUnitTest :tv:testDebugUnitTest
./gradlew :capture:lintDebug :app:lintDebug :tv:lintDebug
./gradlew :app:assembleDebug :app:assembleRelease :tv:assembleDebug :tv:assembleRelease
./gradlew :capture:connectedDebugAndroidTest   # emulator/device: encoder pipeline -> valid MP4
```

**Host mode — where Google Maven is unreachable** (`-Pcaptureone.hostOnly=true`, no AGP). `maven.google.com`
redirects every artifact to `dl.google.com`, which the build container blocks, so AGP/AndroidX/Compose cannot
resolve there. Host mode compiles the `:capture` engine with Kotlin/JVM against the SDK's `android.jar`
(`-no-jdk`, so `java.*` also comes from `android.jar`), type-checks the platform-neutral Compose screens
(`app/.../ui`, `android-tv/.../ui`) against JetBrains Compose for Desktop (same `androidx.compose.*` API on Maven
Central; the TV screens use a stand-in for the thin `tvkit` adapter over tv-material), and runs every JVM test.
It never produces the product APKs.

```bash
cd apps/android
gradle -Pcaptureone.hostOnly=true test            # all JVM unit tests (same sources as the AGP build)
tools/host-verify.sh                              # + d8 --min-api 26, manifest merge, aapt2 link/badging, lint
../../native/android-capture/harness/build-apk.sh # signed framework-only engine harness APK
```

Not compiled in host mode (only by CI): the Android glue that needs AndroidX/media3/tv-material
(`MainActivity`, `AppViewModel`, `MediaViews`, `RecordTileService`, `TvActivity`, `TvViewModel`,
`android-tv/.../tvkit/TvKit.kt`) and the `androidTest` sources (type-checked once against stubs).

## Release signing

Keystores and passwords never enter the repository. Both apps read, in order, Gradle properties then env vars:

| Gradle property (e.g. `~/.gradle/gradle.properties`) | Environment variable |
|---|---|
| `captureone.signing.storeFile` | `CAPTUREONE_KEYSTORE` |
| `captureone.signing.storePassword` | `CAPTUREONE_KEYSTORE_PASSWORD` |
| `captureone.signing.keyAlias` | `CAPTUREONE_KEY_ALIAS` |
| `captureone.signing.keyPassword` | `CAPTUREONE_KEY_PASSWORD` |

Without them `assembleRelease` produces `*-release-unsigned.apk`. CI decodes the repository secrets
`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` into these
variables when they exist. A real release key should be an upload key enrolled in Play App Signing and kept in a
secret store; create one with
`keytool -genkeypair -keystore release.jks -storetype PKCS12 -alias captureone -keyalg RSA -keysize 3072 -validity 10000`.
R8 minification is off until minified builds have been checked on devices.

## How capture works (`:capture`)

1. The UI launches `MediaProjectionManager.createScreenCaptureIntent()` (consent each session).
2. The result goes to `CaptureService` via `startForegroundService`; the service calls `startForeground` with type
   `mediaProjection` (plus `microphone` when audio is recorded and `RECORD_AUDIO` is granted) **before**
   `getMediaProjection`. Each consent result is used once (Android 14+).
3. `RecordingSession`: countdown → registers `MediaProjection.Callback` (before `createVirtualDisplay`, required on 14+)
   → `VirtualDisplay` renders into the input `Surface` of a `MediaCodec` H.264/H.265 encoder (hardware first,
   `VideoCapabilities` size fitting, bitrate from `presets.json`, `KEY_FRAME_RATE`, `KEY_I_FRAME_INTERVAL`,
   `KEY_MAX_FPS_TO_ENCODER` on 29+) → `MediaMuxer` MP4.
4. Audio: microphone `AudioRecord` (`MIC`, fallback `VOICE_RECOGNITION`) and/or `AudioPlaybackCaptureConfiguration`
   (29+, usages MEDIA/GAME/UNKNOWN) → timeline mixer (per-source gain, soft-knee limiter, gap filling) → AAC.
   Mixed (one track) or separate tracks.
5. One clock: `System.nanoTime()` (= CLOCK_MONOTONIC = VirtualDisplay frame timestamps = `AudioTimestamp`
   `TIMEBASE_MONOTONIC`). Pause drops samples captured while paused and subtracts paused time from later PTS,
   identically for audio and video; the VirtualDisplay surface is detached while paused and a sync frame is
   requested on resume (non-key frames are dropped until it arrives).
6. Limits every 500 ms: max duration, max file size, free space (`StatFs`) → graceful stop with `STORAGE_FULL`.
   `MediaProjection.Callback.onStop` (user/system revoked) finalises the partial file with `INTERRUPTED`.
7. Finalise: muxer stop in `finally`, validation with `MediaExtractor` + `MediaMetadataRetriever` →
   `MediaMetadata.validation`, thumbnail, publish from app-specific temp storage to MediaStore
   `Movies/CaptureOne` / `Pictures/CaptureOne` (`IS_PENDING` on 29+; `WRITE_EXTERNAL_STORAGE` on 26–28), SQLite
   history. If the copy is impossible (no space/permission) the file stays in app storage and the UI says so.
8. Screenshots: `ImageReader` (RGBA_8888) on a VirtualDisplay, row padding removed, PNG/JPEG, validated with
   `BitmapFactory`. During a recording the recording's VirtualDisplay is briefly pointed at the ImageReader
   (a second `createVirtualDisplay` is forbidden on 14+), which freezes the video for a frame or two.

Protected content (FLAG_SECURE windows, DRM/HDCP video) is blanked by Android and stays black.

## Per-feature API levels (minSdk 26)

| Feature | API level | Notes |
|---|---|---|
| Screen recording / screenshot (MediaProjection) | 26 (21+) | consent per session; FGS type `mediaProjection` from 29 |
| Frame-rate cap (`KEY_MAX_FPS_TO_ENCODER`) | 29 | below 29 frames follow screen updates |
| Hardware-encoder flag (`isHardwareAccelerated`) | 29 | inferred from codec names below 29 |
| Internal audio (`AudioPlaybackCapture`) | 29 | apps can opt out; never calls |
| MediaStore `RELATIVE_PATH` / `IS_PENDING` | 29 | 26–28: `WRITE_EXTERNAL_STORAGE` (maxSdk 28) |
| `createDeleteRequest` / `createWriteRequest` | 30 | 29: `RecoverableSecurityException` |
| `FOREGROUND_SERVICE_TYPE_MICROPHONE` | 30 | passed only when audio is recorded |
| `POST_NOTIFICATIONS` runtime permission | 33 | recording works without it; controls are then not shown |
| Single-app capture, `onCapturedContentResize/VisibilityChanged`, FGS typed permissions | 34 | |
| Quick Settings tile (`startActivityAndCollapse(PendingIntent)`) | 24 / 34 | |
| Dynamic colour | 31 | |

## TV app

On launch it classifies native capture: **supported** (consent activity + H.264 encoder + API 29 internal audio),
**partially supported** (no internal audio), **not available** (no consent activity — common on TV builds — or no
encoder) and then offers native record/screenshot/library (same engine) or **companion mode**: pair with a
CaptureOne desktop hub (`POST /api/v1/pair`, 6-digit code typed on an on-screen numeric pad), live status over the
`/api/v1/events` WebSocket, `POST /api/v1/command`, and recordings streamed with media3 using the bearer token.
Protocol v1 is plain HTTP on the LAN, so the TV app permits cleartext traffic.

## Verification status

See the workstream report. In short: host-mode compile + JVM tests + d8/aapt2/lint/manifest checks and a signed
framework-only harness APK are verified in the build container; the AGP/Compose APKs are built and linted only in
CI; nothing has run on a device or emulator here (no KVM).

## Third-party dependencies

| Dependency | License |
|---|---|
| AndroidX (core, activity, lifecycle, Compose BOM 2025.06.01, Material 3, tv-material 1.0.0, media3 1.6.1, test) | Apache-2.0 |
| Kotlin stdlib, kotlinx-coroutines 1.10.2, kotlinx-serialization 1.9.0 | Apache-2.0 |
| OkHttp / MockWebServer 4.12.0 (Okio) | Apache-2.0 |
| JUnit 4.13.2 (tests), Hamcrest 1.3 | EPL-1.0, BSD-3-Clause |
| networknt json-schema-validator 1.5.6 (tests; Jackson) | Apache-2.0 |
| JetBrains Compose for Desktop 1.8.2 (host-mode type-check only, never shipped) | Apache-2.0 |
| Android Gradle Plugin 8.13.0, Gradle 8.14.3 wrapper | Apache-2.0 |
