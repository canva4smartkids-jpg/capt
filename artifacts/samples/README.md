# Validated sample media

Small files produced by the real capture pipelines and verified with `ffprobe` (and
`ffmpeg -v error -f null` full decodes for videos). Each file is < 3 MB.

## Desktop engine — Linux X11 (engine workstream)

Produced by `tests/integration/desktop/run.sh` (Xvfb `:99 -screen 0 1920x1080x24`,
PulseAudio null sink with a 440 Hz test tone, moving content from `ffplay testsrc2` and
`xterm -e top`) through `captureone-cli` → `linux-capture` (MIT-SHM `GetImage`) →
`media-engine` (pacer → ffmpeg 6.1.1 libx264/libx265 → fragmented MP4 → mux → ffprobe).

| File | Command | ffprobe facts |
|---|---|---|
| `linux-x11-h264-720p-system-audio.mp4` (908 221 B) | `record --target display:screen --height 720 --system-audio --quality low --duration 5` | mp4; duration 5.034667 s; video h264 / avc1, 1280x720, yuv420p, bt709, 30/1 fps, 150 packets; audio aac / mp4a 48000 Hz stereo, 236 packets, track name "System audio"; volumedetect mean -21.4 dB, max -17.9 dB; full decode: no errors |
| `linux-x11-h265-720p.mp4` (617 208 B) | `record --target display:screen --codec h265 --height 720 --duration 4` | mp4; duration 4.000000 s; video hevc / **hvc1**, 1280x720, yuv420p, bt709, 30/1 fps, 120 packets; no audio; full decode: no errors |
| `linux-x11-screenshot-region.png` (23 396 B) | `screenshot --target region:screen:380,180,800,600 --format png` | png_pipe; png, 800x600, rgb24; decodes (image-rs and ffmpeg); non-uniform content (testsrc2 bars + xterm) |
| `linux-x11-screenshot-region.jpg` (49 723 B) | `screenshot --target region:screen:380,180,800,600 --format jpeg` (quality 90) | image2; mjpeg, 800x600, yuvj444p; decodes (image-rs and ffmpeg) |

The audio sample lasts 5.0347 s for 5.000 s of recorded time: 236 AAC frames x 1024
samples (the encoder pads the last frame and adds priming delay); the video track is exactly
150 frames = 5.000 s.
