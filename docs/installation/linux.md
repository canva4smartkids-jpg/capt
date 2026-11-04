# Linux installation

Requirements: an **X11** session (Wayland-only sessions are not yet supported; XWayland apps are captured within X11 sessions), `ffmpeg` with libx264 (and libx265 for H.265), PulseAudio or PipeWire-pulse for system audio, WebKitGTK 4.1.

Debian/Ubuntu:

```bash
sudo apt install ./captureone_<version>_amd64.deb ffmpeg
```

Fedora/RHEL:

```bash
sudo dnf install ./captureone-<version>-1.x86_64.rpm ffmpeg
```

AppImage:

```bash
chmod +x CaptureOne_<version>_amd64.AppImage && ./CaptureOne_<version>_amd64.AppImage
```

Recordings default to `~/Videos/CaptureOne`. System audio uses the default sink's monitor source.
