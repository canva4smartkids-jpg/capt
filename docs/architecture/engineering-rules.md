# CaptureOne engineering rules and cross-team interfaces

Read this fully before writing code. It is the shared brief for every parallel workstream.

## 1. Build environment (this container)

- Ubuntu 24.04 x86_64, 4 cores, 15 GB RAM, ~30 GB free disk. Several agents build concurrently: be frugal.
- Repo: `/home/user/capt` (git branch `claude/gifted-fermi-0oqiw1`). It is a pnpm + Cargo monorepo.
- Toolchains: Rust 1.97 stable (rustup available; can add targets), Node 22, pnpm 10, JDK 21, Gradle 8.14.3 (`/opt/gradle-8.14.3/bin/gradle`), ffmpeg/ffprobe 6.1 (system, libx264/libx265 enabled), Docker (daemon running), Xvfb, PulseAudio, xdotool, xterm, ImageMagick, clang/lld, NSIS, webkit2gtk-4.1 dev libs (Tauri Linux deps installed).
- Android SDK: `/opt/android-sdk` (platforms 34 & 35, build-tools 35.0.1, platform-tools, cmdline-tools). **No emulator, no KVM, no physical devices.**
- No Windows or macOS machine. Windows/macOS code can only be type-checked/cross-compiled here and built/tested in GitHub Actions.
- Network policy: crates.io, npm, Maven Central (repo.maven.apache.org), Google Maven (maven.google.com), Gradle plugin portal, Docker Hub (rate limited — retry with backoff), Ubuntu apt, static.rust-lang.org are reachable. **Blocked:** dl.google.com, GitHub release/asset downloads and api.github.com, download.tizen.org, LG/Samsung developer sites.
- Proxy: if a tool fails TLS, point it at `/root/.ccr/ca-bundle.crt` (e.g. `CARGO_HTTP_CAINFO`, `NODE_EXTRA_CA_CERTS`). Never disable TLS verification.
- **Use your own Cargo target dir** to avoid lock contention between agents: `export CARGO_TARGET_DIR=/tmp/cargo-target/<your-workstream>` and `export CARGO_BUILD_JOBS=2`.

## 2. Non-negotiable quality rules

1. No fake recording, no mocked screenshots in production code, no placeholder presented as complete. If something cannot work on a platform, return `CaptureError { code: UNSUPPORTED }` and report the feature as `unsupported` in the capability report.
2. Never claim verification you did not perform. "Compiles" ≠ "records". "API call succeeded" ≠ "valid video". Verify produced media with ffprobe (duration, dimensions, codec, audio streams) wherever it can actually run.
3. TypeScript strict mode (extend `tsconfig.base.json`). Rust: no `unwrap()` on fallible runtime paths in production code; map errors to `CaptureError`.
4. Proper resource cleanup (stop streams, kill/await child processes, release OS handles) and interruption handling (source closed, permission revoked, disk full, process killed → recoverable partial file).
5. Never bypass OS security or DRM. Protected content that the OS blanks stays blank; report it honestly.
6. Stay inside the directories you own (section 5). Do **not** edit `packages/capture-contracts`, `packages/settings`, or `native/capture-core`; if you need a contract change, put the exact proposal in your final report.
7. Git: commit only your own paths (`git add <your dirs> [Cargo.lock pnpm-lock.yaml]`; never `git add -A`), small descriptive commits ending with the two trailer lines below. Never push, amend, rebase or reset. If `.git/index.lock` exists, wait a few seconds and retry.

   ```
   Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
   Claude-Session: https://claude.ai/code/session_01964TrhowaiadHz66DPjGrW
   ```
8. Build outputs (installers, APKs, packages) go to `/home/user/capt/artifacts/<platform>/` (gitignored). Small validated sample media go to `/home/user/capt/artifacts/samples/` (committed; keep each file < 3 MB).
9. Licensing: record every new third-party dependency with its license in your final report (and in `docs/release/licenses-<workstream>.md` inside your area if you own docs there).

## 3. Shared contract

- TypeScript source of truth: `packages/capture-contracts/src/*.ts` (zod). JSON Schemas: `packages/capture-contracts/schema/*.schema.json` (regenerate with `pnpm schema`). Cross-language fixtures: `tests/contract/fixtures/presets-cases.json`, `packages/capture-contracts/schema/recording-transitions.json`, `packages/capture-contracts/schema/enums.json`.
- Rust mirror: `native/capture-core` (`CaptureBackend` trait in `src/backend.rs`, types in `src/types.rs`, frames in `src/frame.rs`, presets in `src/presets.rs`, state machine in `src/state.rs`). Kotlin must mirror the same JSON shapes and be contract-tested against the same schemas/fixtures.
- Encoding presets: `packages/settings/presets.json` is the single source of truth (Rust `include_str!`s it; Android copies it into assets at build time).
- Desktop IPC (Tauri commands/events/windows): `packages/capture-contracts/src/desktop-ipc.ts`.
- Companion protocol (hub ⇄ TV/mobile companions): `packages/capture-contracts/src/companion.ts`.

## 4. Cross-workstream Rust interfaces (implement exactly; you may add, not change)

### 4.1 Backend factory (every native backend crate)

Each backend crate exposes, only on its own OS (`#[cfg(target_os = "...")]`):

```rust
pub fn create_backend() -> capture_core::CaptureResult<std::sync::Arc<dyn capture_core::CaptureBackend>>;
```

- `linux-capture` (crate `linux_capture`, X11), `windows-capture-backend` (crate `windows_capture_backend`, Windows Graphics Capture), `macos-capture` (crate `macos_capture`, ScreenCaptureKit).
- On other OSes the crate must still compile (empty or cfg-gated), so the workspace builds everywhere.
- Display ids must be stable strings; `list_sources` returns physical pixel sizes.
- Frames are BGRA8 in system memory (`VideoFrame`). Region targets may be implemented natively or by `VideoFrame::crop`.
- System audio (if supported) is delivered through `start_system_audio` as interleaved f32 PCM at the device rate; media-engine resamples.

### 4.2 `media-engine` public API (owned by the engine workstream, consumed by the desktop app and CLI)

```rust
pub struct EngineConfig {
    pub ffmpeg_path: Option<std::path::PathBuf>,   // None = locate (env CAPTUREONE_FFMPEG, sidecar next to exe, PATH)
    pub ffprobe_path: Option<std::path::PathBuf>,  // None = locate (env CAPTUREONE_FFPROBE, sidecar, PATH)
}

#[derive(Clone)] pub struct MediaEngine { /* Arc inside */ }
impl MediaEngine {
    pub fn new(backend: Arc<dyn CaptureBackend>, cfg: EngineConfig) -> CaptureResult<Self>;
    pub fn backend(&self) -> &Arc<dyn CaptureBackend>;
    /// Backend capture capabilities merged with encoders verified by real test encodes.
    pub fn capabilities(&self) -> CapabilityReport;
    pub fn list_audio_devices(&self) -> CaptureResult<Vec<AudioDeviceInfo>>;
    /// Captures, encodes (PNG/JPEG), writes into `output_dir`, validates by decoding, returns metadata.
    pub fn screenshot(&self, req: &ScreenshotRequest, output_dir: &Path) -> CaptureResult<MediaMetadata>;
    /// PNG data URL of a source scaled to max_width (for source pickers).
    pub fn source_thumbnail(&self, kind: SourceKind, source_id: &str, max_width: u32) -> CaptureResult<String>;
    /// Starts recording immediately (the caller handles any countdown).
    pub fn start_recording(&self, config: RecordingConfig, listener: Arc<dyn RecorderListener>) -> CaptureResult<RecordingSession>;
    /// ffprobe-based metadata + validation for an existing media file.
    pub fn inspect(&self, path: &Path) -> CaptureResult<MediaMetadata>;
    /// Finds interrupted sessions under `output_dir` and finalizes them into playable MP4s.
    pub fn recover_sessions(&self, output_dir: &Path) -> Vec<CaptureResult<MediaMetadata>>;
    pub fn ffmpeg_info(&self) -> FfmpegInfo; // { ffmpeg_path, ffmpeg_version, ffprobe_path }
}

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo { pub id: String, pub name: String, pub kind: String /* "input" | "loopback" */, pub is_default: bool }

pub trait RecorderListener: Send + Sync + 'static {
    fn on_state(&self, session_id: &str, from: RecordingState, to: RecordingState);
    fn on_status(&self, status: &RecordingStatus);          // ~2x per second while active
    fn on_warning(&self, session_id: &str, message: &str);
    fn on_error(&self, session_id: &str, error: &CaptureError);
    /// Exactly once per session, after finalization (user stop, limits, source closed, error).
    fn on_completed(&self, session_id: &str, reason: StopReason, result: &CaptureResult<MediaMetadata>);
}

pub struct RecordingSession { /* cheap clonable handle */ }
impl RecordingSession {
    pub fn id(&self) -> &str;
    pub fn pause(&self) -> CaptureResult<()>;
    pub fn resume(&self) -> CaptureResult<()>;
    /// Blocks until the file is finalized and validated.
    pub fn stop(&self) -> CaptureResult<MediaMetadata>;
    /// Stops and deletes all output.
    pub fn cancel(&self) -> CaptureResult<()>;
    pub fn status(&self) -> RecordingStatus;
}
```

### 4.3 `companion-hub` public API (owned by the TV workstream, consumed by the desktop app)

```rust
pub trait HubBackend: Send + Sync + 'static {
    fn hub_info(&self) -> HubInfo;
    fn list_recordings(&self) -> Vec<MediaMetadata>;
    fn recording_path(&self, id: &str) -> Option<PathBuf>;
    fn thumbnail_path(&self, id: &str) -> Option<PathBuf>;
    fn status(&self) -> Option<RecordingStatus>;
    fn command(&self, cmd: RemoteCommand) -> RemoteCommandResult;
}
pub struct HubConfig { pub bind: std::net::IpAddr, pub port: u16, pub state_dir: PathBuf }
pub struct HubHandle { /* ... */ }
impl HubHandle {
    pub fn start(backend: Arc<dyn HubBackend>, cfg: HubConfig) -> CaptureResult<HubHandle>; // spawns its own tokio runtime thread
    pub fn url(&self) -> String;
    pub fn pairing_code(&self) -> String;
    pub fn regenerate_code(&self) -> String;
    pub fn paired_devices(&self) -> Vec<PairedDevice>; // { name, device_kind, paired_at }
    pub fn broadcast(&self, event: HubEvent);
    pub fn stop(self);
}
```

`device-discovery` exposes `pub fn scan(timeout: Duration) -> CaptureResult<Vec<DiscoveredDevice>>` plus LG SSAP / Samsung remote clients.

## 5. Directory ownership

| Workstream | Owns |
|---|---|
| engine | `native/media-engine`, `native/linux-capture`, `native/capture-cli`, `tests/integration/desktop`, `.github/workflows/linux.yml` |
| desktop-app | `apps/desktop` (React UI + `src-tauri`), `packages/design-system`, `packages/media-metadata`, `packages/shared-types` |
| windows | `native/windows-capture`, `.github/workflows/windows.yml` |
| macos | `native/macos-capture`, `.github/workflows/macos.yml` |
| android | `apps/android`, `apps/android-tv`, `native/android-capture`, `.github/workflows/android.yml` |
| tv | `apps/samsung-tv`, `apps/lg-tv`, `apps/tv-web`, `services/device-discovery`, `services/companion-hub`, `.github/workflows/tv.yml` |
| coordinator | everything else (root files, contracts, capture-core, docs/, top-level reports, `.github/workflows/ci.yml`) |

## 6. Final report format (every workstream)

Return a concise report with: implemented functionality; **verified** functionality with the exact command and observed evidence; build results; test results (counts); artifacts produced (paths); known limitations; blockers (with the exact missing requirement); new dependencies + licenses; proposed contract changes.
