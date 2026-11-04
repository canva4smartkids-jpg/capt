# macOS installation

Requirements: macOS 12.3 or later (13+ for system audio, 14+ for the fast screenshot path). Apple Silicon and Intel.

1. Download `CaptureOne_<version>.dmg` from the CI artifacts (`macos` workflow) and drag CaptureOne to Applications.
2. Builds from this sprint are **not signed or notarized**. First launch: right-click → *Open* → *Open*, or `xattr -dr com.apple.quarantine /Applications/CaptureOne.app`. Distribution builds need a Developer ID certificate and notarization (see `BLOCKERS.md`).

## Permissions
- **Screen Recording** (required): on first capture macOS prompts; afterwards enable CaptureOne in *System Settings → Privacy & Security → Screen & System Audio Recording* and relaunch the app. Settings → Permissions in CaptureOne shows the status and opens the pane.
- **Microphone** (optional): prompted on first use.
- System audio is captured through ScreenCaptureKit (covered by Screen Recording permission). DRM-protected content (e.g. FairPlay video) is blank by design.
