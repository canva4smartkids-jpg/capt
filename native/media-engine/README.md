# media-engine

Encoding, muxing, A/V sync, validation and recording orchestration (see `src/lib.rs`).

## Capture devices (HDMI capture cards / UVC)

`src/device.rs` records external video devices through ffmpeg's OS demuxers, independent of the
platform `CaptureBackend`: `v4l2` (Linux, ids `v4l2:/dev/videoN`, listed from
`/sys/class/video4linux`, `index != 0` nodes skipped), `dshow` (Windows, `dshow:<name>`, parsed
from `-list_devices true`), `avfoundation` (macOS, `avfoundation:<index>`, "Capture screen N"
excluded). ffmpeg writes raw BGRA to stdout; a reader thread delivers `VideoFrame`s to the normal
`VideoSink`, so pacer, pause/resume, limits, audio, fragmented MP4 and validation are unchanged.
Use `MediaEngine::list_capture_devices` / `list_sources(&[SourceKind::Device])`; the CLI target is
`device:<id>`. Listed sizes are a 1920x1080 default (opening a device to probe is not cheap); the
recorded size is that of the first frame. HDCP-protected sources are blank. A card's audio is a
separate input: select it as the microphone device. Capability `hdmiCaptureDevice` is `partial`
when ffmpeg lists the demuxer (`ffmpeg -devices`).

Test seam (tests only): `CAPTUREONE_TEST_DEVICE_INPUT` (ffmpeg input args, e.g.
`-f lavfi -i testsrc2=size=1280x720:rate=30`) replaces the device input and
`CAPTUREONE_TEST_DEVICE_NAME` lists one fake device `test:capture0`. Real hardware capture is
unverified; see `tests/integration/desktop/`.
