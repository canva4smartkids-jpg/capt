# Android installation (phones and tablets)

Requirements: Android 8.0 (API 26)+. Internal-audio capture needs Android 10+; single-app capture needs Android 14+.

1. Download `captureone-<version>.apk` (artifacts of the `android` workflow).
2. Allow installing from your browser/file manager (*Install unknown apps*) and open the APK, or use `adb install captureone-<version>.apk`.
3. Builds from this sprint are signed with a **test key**; Play Store distribution needs the publisher's upload key (see `BLOCKERS.md`).

## Permissions
- Screen capture: Android asks for consent **every recording** (system dialog). On Android 14+ you may choose a single app or the entire screen.
- Notifications (Android 13+): required for the recording notification with pause/stop controls.
- Microphone: optional, for voice-over. Internal audio is only available for apps that allow capture (many streaming apps opt out).
- Protected content (FLAG_SECURE, DRM video) records as black.
