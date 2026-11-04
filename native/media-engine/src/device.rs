//! External video capture devices (HDMI capture cards, UVC webcams) via ffmpeg's OS device
//! demuxers: `v4l2` (Linux), `dshow` (Windows), `avfoundation` (macOS).
//!
//! ffmpeg reads the device and writes raw BGRA frames to stdout; a reader thread turns them
//! into `VideoFrame`s for the same `VideoSink` the recorder uses for screen capture, so the
//! pacer, pause/resume, limits, audio, fragmented MP4 and validation apply unchanged.
//!
//! Device ids: `v4l2:/dev/videoN`, `dshow:<friendly name>`, `avfoundation:<index>`.
//!
//! Audio from a capture card is not part of the video stream: select the card's audio input as
//! the microphone device in the recording config.
//!
//! Test seam (never set in production): `CAPTUREONE_TEST_DEVICE_INPUT` holds ffmpeg input args
//! (whitespace separated, e.g. `-f lavfi -i testsrc2=size=1280x720:rate=30`) used instead of
//! the OS device input (`-re` is prepended so synthetic sources run in real time), and
//! `CAPTUREONE_TEST_DEVICE_NAME` makes one fake device (`test:capture0`) appear in listings.

use crate::tools::{new_command, run_with_timeout, wait_child};
use capture_core::*;
use parking_lot::Mutex;
use std::io::{BufRead, Read};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub const ENV_TEST_INPUT: &str = "CAPTUREONE_TEST_DEVICE_INPUT";
pub const ENV_TEST_NAME: &str = "CAPTUREONE_TEST_DEVICE_NAME";
pub const TEST_DEVICE_ID: &str = "test:capture0";
/// Listed size when the device is not probed (probing would open the device).
const DEFAULT_W: u32 = 1920;
const DEFAULT_H: u32 = 1080;
const START_TIMEOUT: Duration = Duration::from_secs(10);

fn source(id: String, name: String, descriptor: String) -> CaptureSource {
    CaptureSource {
        id,
        kind: SourceKind::Device,
        name,
        // Devices are not opened for listing; the real size is that of the first frame.
        width: DEFAULT_W,
        height: DEFAULT_H,
        x: None,
        y: None,
        scale_factor: None,
        is_primary: None,
        app_name: None,
        device_descriptor: Some(descriptor),
    }
}

fn test_input() -> Option<Vec<String>> {
    let v = std::env::var(ENV_TEST_INPUT).ok()?;
    let args: Vec<String> = v.split_whitespace().map(str::to_string).collect();
    (!args.is_empty()).then_some(args)
}

/// Lists video capture devices for this OS.
pub fn list_devices(ffmpeg: Option<&Path>) -> Vec<CaptureSource> {
    let mut out = Vec::new();
    if let Ok(name) = std::env::var(ENV_TEST_NAME) {
        if !name.trim().is_empty() {
            out.push(source(TEST_DEVICE_ID.into(), name, "test device (CAPTUREONE_TEST_DEVICE_INPUT)".into()));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = ffmpeg;
        out.extend(list_v4l2(Path::new("/sys/class/video4linux")));
    }
    #[cfg(target_os = "windows")]
    if let Some(ff) = ffmpeg {
        let text = list_output(ff, &["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"]);
        out.extend(parse_dshow_list(&text).into_iter().map(|n| source(format!("dshow:{n}"), n.clone(), format!("DirectShow video device \"{n}\""))));
    }
    #[cfg(target_os = "macos")]
    if let Some(ff) = ffmpeg {
        let text = list_output(ff, &["-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""]);
        out.extend(parse_avfoundation_list(&text).into_iter().map(|(i, n)| source(format!("avfoundation:{i}"), n.clone(), format!("AVFoundation video device [{i}] {n}"))));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = ffmpeg;
    out
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn list_output(ffmpeg: &Path, args: &[&str]) -> String {
    let mut cmd = new_command(ffmpeg);
    cmd.args(args);
    // ffmpeg exits non-zero after listing; the listing is on stderr.
    run_with_timeout(cmd, Duration::from_secs(15)).map(|o| o.stderr_text()).unwrap_or_default()
}

pub fn find_device(ffmpeg: Option<&Path>, id: &str) -> CaptureResult<CaptureSource> {
    list_devices(ffmpeg)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("capture device '{id}' not found")))
}

/// Linux: `<root>/video*/name` where `index` is 0 (other nodes are metadata/secondary).
pub fn list_v4l2(root: &Path) -> Vec<CaptureSource> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut nodes: Vec<(u32, String)> = Vec::new();
    for e in rd.flatten() {
        let fname = e.file_name().to_string_lossy().to_string();
        let Some(n) = fname.strip_prefix("video").and_then(|n| n.parse::<u32>().ok()) else { continue };
        let index = std::fs::read_to_string(e.path().join("index")).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
        if index != 0 {
            continue;
        }
        let name = std::fs::read_to_string(e.path().join("name")).map(|s| s.trim().to_string()).unwrap_or_default();
        nodes.push((n, if name.is_empty() { format!("Video device {n}") } else { name }));
    }
    nodes.sort();
    nodes.into_iter().map(|(n, name)| source(format!("v4l2:/dev/video{n}"), name, format!("V4L2 /dev/video{n}"))).collect()
}

/// Windows: video entries of `-list_devices true -f dshow`, e.g. `[dshow @ 0x1] "Name" (video)`.
pub fn parse_dshow_list(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        let Some(end) = line.rfind('"') else { continue };
        let Some(start) = line.find('"') else { continue };
        if start >= end {
            continue;
        }
        let tail = &line[end + 1..];
        let is_video = tail.trim().strip_prefix('(').and_then(|t| t.strip_suffix(')')).is_some_and(|t| t.split(',').any(|k| k.trim() == "video"));
        let name = &line[start + 1..end];
        if is_video && !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// macOS: `[AVFoundation indev @ 0x1] [0] FaceTime HD Camera` under the video section; screens excluded.
pub fn parse_avfoundation_list(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut in_video = false;
    for line in text.lines() {
        if line.contains("AVFoundation video devices") {
            in_video = true;
            continue;
        }
        if line.contains("AVFoundation audio devices") {
            in_video = false;
            continue;
        }
        if !in_video {
            continue;
        }
        // the entry is the last "[N] name" on the line (the first bracket is the logger prefix)
        let Some(pos) = line.rfind("] [").map(|p| p + 2).or_else(|| line.starts_with('[').then_some(0)) else { continue };
        let rest = &line[pos..];
        let Some(close) = rest.find(']') else { continue };
        let Ok(idx) = rest[1..close].parse::<u32>() else { continue };
        let name = rest[close + 1..].trim();
        if name.is_empty() || name.starts_with("Capture screen") {
            continue;
        }
        out.push((idx, name.to_string()));
    }
    out
}

/// ffmpeg input arguments (everything before the output options) for a device id.
pub fn input_args(device_id: &str, fps: u32) -> CaptureResult<Vec<String>> {
    if let Some(mut t) = test_input() {
        t.insert(0, "-re".into());
        return Ok(t);
    }
    let fps = fps.max(1).to_string();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    if let Some(path) = device_id.strip_prefix("v4l2:") {
        if !path.starts_with("/dev/video") {
            return Err(CaptureError::invalid_config(format!("invalid v4l2 device '{path}'")));
        }
        Ok([s(&["-f", "v4l2", "-framerate", &fps, "-i"]), vec![path.to_string()]].concat())
    } else if let Some(name) = device_id.strip_prefix("dshow:") {
        if name.is_empty() {
            return Err(CaptureError::invalid_config("empty dshow device name"));
        }
        Ok([s(&["-f", "dshow", "-rtbufsize", "256M", "-framerate", &fps, "-i"]), vec![format!("video={name}")]].concat())
    } else if let Some(idx) = device_id.strip_prefix("avfoundation:") {
        if idx.parse::<u32>().is_err() {
            return Err(CaptureError::invalid_config(format!("invalid avfoundation index '{idx}'")));
        }
        Ok([s(&["-f", "avfoundation", "-framerate", &fps, "-i"]), vec![format!("{idx}:none")]].concat())
    } else {
        Err(CaptureError::invalid_config(format!("unknown capture device id '{device_id}'")))
    }
}

/// Complete ffmpeg argument list: device input, raw BGRA on stdout.
pub fn ffmpeg_args(device_id: &str, fps: u32) -> CaptureResult<Vec<String>> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"].iter().map(|s| s.to_string()).collect();
    a.extend(input_args(device_id, fps)?);
    a.extend(["-an", "-sn", "-pix_fmt", "bgra", "-f", "rawvideo", "pipe:1"].iter().map(|s| s.to_string()));
    Ok(a)
}

/// Extracts `WxH` from the output stream line of ffmpeg's banner
/// (`Stream #0:0: Video: rawvideo (BGRA / ...), bgra(progressive), 1280x720 [SAR ...]`).
pub fn parse_output_size(line: &str) -> Option<(u32, u32)> {
    if !(line.contains("Video: rawvideo") && line.contains("bgra")) {
        return None;
    }
    line.split(|c: char| c == ',' || c.is_whitespace()).find_map(|tok| {
        let (w, h) = tok.split_once('x')?;
        let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
        (w > 0 && h > 0).then_some((w, h))
    })
}

fn classify(stderr: &str) -> CaptureError {
    let low = stderr.to_ascii_lowercase();
    let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
    let (code, msg) = if low.contains("busy") || low.contains("in use") {
        (CaptureErrorCode::DeviceBusy, "the capture device is busy (in use by another application)")
    } else if low.contains("no such file") || low.contains("could not find") || low.contains("no such device") || low.contains("cannot open video device") {
        (CaptureErrorCode::SourceNotFound, "the capture device was not found")
    } else if low.contains("permission denied") || low.contains("not authorized") {
        (CaptureErrorCode::PermissionDenied, "permission to use the capture device was denied")
    } else {
        (CaptureErrorCode::SourceClosed, "the capture device produced no video")
    };
    CaptureError::new(code, format!("{msg}: {tail}"))
}

struct DeviceStream {
    child: Arc<Mutex<Option<Child>>>,
    stopping: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl StreamHandle for DeviceStream {
    fn stop(&mut self) -> CaptureResult<()> {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(mut c) = self.child.lock().take() {
            let _ = c.kill();
            let _ = wait_child(&mut c, Duration::from_secs(5));
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        Ok(())
    }
}

impl Drop for DeviceStream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Starts reading the device; returns once ffmpeg reported the output size (the first frame
/// follows shortly) or failed.
pub fn start_stream(ffmpeg: &Path, device_id: &str, fps: u32, sink: Arc<dyn VideoSink>) -> CaptureResult<Box<dyn StreamHandle>> {
    let args = ffmpeg_args(device_id, fps)?;
    let mut cmd = new_command(ffmpeg);
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    log::info!("device capture: {cmd:?}");
    let mut child = cmd.spawn().map_err(|e| CaptureError::new(CaptureErrorCode::EncoderUnavailable, format!("cannot start ffmpeg: {e}")))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));
    let stopping = Arc::new(AtomicBool::new(false));
    let tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (size_tx, size_rx) = crossbeam_channel::bounded::<(u32, u32)>(1);

    let mut handle = DeviceStream { child: child.clone(), stopping: stopping.clone(), threads: Vec::new() };
    let tail2 = tail.clone();
    let spawn = |name: &str, f: Box<dyn FnOnce() + Send>| std::thread::Builder::new().name(name.into()).spawn(f).map_err(|e| CaptureError::internal(format!("cannot spawn thread: {e}")));
    if let Some(err) = stderr {
        handle.threads.push(spawn(
            "device-stderr",
            Box::new(move || {
                let mut in_output = false;
                for line in std::io::BufReader::new(err).split(b'\n').map_while(Result::ok) {
                    let line = String::from_utf8_lossy(&line).trim_end().to_string();
                    log::debug!("ffmpeg(device): {line}");
                    if line.starts_with("Output #0") {
                        in_output = true;
                    } else if in_output {
                        if let Some(sz) = parse_output_size(&line) {
                            let _ = size_tx.try_send(sz);
                            in_output = false;
                        }
                    }
                    let mut t = tail2.lock();
                    if t.len() >= 20 {
                        t.remove(0);
                    }
                    t.push(line);
                }
            }),
        )?);
    }
    let (child2, stopping2, tail3) = (child.clone(), stopping.clone(), tail.clone());
    let (frame_size_tx, frame_size_rx) = crossbeam_channel::bounded::<(u32, u32)>(1);
    let sink2 = sink.clone();
    if let Some(mut out) = stdout {
        handle.threads.push(spawn(
            "device-reader",
            Box::new(move || {
                let Ok((w, h)) = frame_size_rx.recv() else { return };
                let mut buf = vec![0u8; w as usize * h as usize * 4];
                loop {
                    if out.read_exact(&mut buf).is_err() {
                        break;
                    }
                    match VideoFrame::new_bgra(w, h, w * 4, buf.clone()) {
                        Ok(f) => sink2.on_frame(f),
                        Err(e) => {
                            sink2.on_event(StreamEvent::Error(e));
                            return;
                        }
                    }
                }
                if stopping2.load(Ordering::SeqCst) {
                    return;
                }
                // EOF: the device went away (or ffmpeg failed mid-stream).
                if let Some(mut c) = child2.lock().take() {
                    let _ = c.kill();
                    let _ = wait_child(&mut c, Duration::from_secs(2));
                }
                std::thread::sleep(Duration::from_millis(50));
                let text = tail3.lock().join("\n");
                let low = text.to_ascii_lowercase();
                if low.contains("busy") {
                    sink2.on_event(StreamEvent::Error(classify(&text)));
                } else {
                    sink2.on_event(StreamEvent::SourceClosed);
                }
            }),
        )?);
    }

    // Wait for the output banner (size) or an early exit.
    let deadline = std::time::Instant::now() + START_TIMEOUT;
    loop {
        if let Ok(sz) = size_rx.recv_timeout(Duration::from_millis(50)) {
            let _ = frame_size_tx.send(sz);
            return Ok(Box::new(handle));
        }
        let exited = child.lock().as_mut().map(|c| c.try_wait().ok().flatten().is_some()).unwrap_or(true);
        if exited || std::time::Instant::now() >= deadline {
            // let the stderr reader drain
            std::thread::sleep(Duration::from_millis(100));
            let text = tail.lock().join("\n");
            let err = if exited { classify(&text) } else { CaptureError::new(CaptureErrorCode::DeviceBusy, format!("capture device did not start within {}s: {text}", START_TIMEOUT.as_secs())) };
            drop(frame_size_tx);
            let _ = handle.stop();
            return Err(err);
        }
    }
}

struct OneFrame(crossbeam_channel::Sender<Result<VideoFrame, CaptureError>>);

impl VideoSink for OneFrame {
    fn on_frame(&self, frame: VideoFrame) {
        let _ = self.0.try_send(Ok(frame));
    }
    fn on_event(&self, event: StreamEvent) {
        let err = match event {
            StreamEvent::Error(e) => e,
            StreamEvent::SourceClosed => CaptureError::new(CaptureErrorCode::SourceClosed, "the capture device closed before delivering a frame"),
            StreamEvent::Resized { .. } => return,
        };
        let _ = self.0.try_send(Err(err));
    }
}

/// Grabs a single frame from the device.
pub fn grab_frame(ffmpeg: &Path, device_id: &str) -> CaptureResult<VideoFrame> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let mut stream = start_stream(ffmpeg, device_id, 30, Arc::new(OneFrame(tx)))?;
    let res = rx.recv_timeout(START_TIMEOUT).unwrap_or_else(|_| Err(CaptureError::new(CaptureErrorCode::SourceClosed, "no frame received from the capture device")));
    let _ = stream.stop();
    res
}

/// True when this ffmpeg build has the OS device demuxer (or the test seam is active).
pub fn demuxer_available(ffmpeg: &Path) -> bool {
    if test_input().is_some() {
        return true;
    }
    let want = if cfg!(target_os = "linux") {
        "v4l2"
    } else if cfg!(target_os = "windows") {
        "dshow"
    } else {
        "avfoundation"
    };
    let mut cmd = new_command(ffmpeg);
    cmd.args(["-hide_banner", "-devices"]);
    let Ok(o) = run_with_timeout(cmd, Duration::from_secs(10)) else { return false };
    devices_list_has(&String::from_utf8_lossy(&o.stdout), want)
}

pub fn devices_list_has(text: &str, name: &str) -> bool {
    text.lines().any(|l| {
        let mut it = l.split_whitespace();
        match (it.next(), it.next()) {
            (Some(flags), Some(names)) => flags.contains('D') && flags.chars().all(|c| c.is_ascii_uppercase() || c == '.') && names.split(',').any(|n| n == name),
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DSHOW: &str = r#"[dshow @ 000001e5b3a4c0c0] DirectShow video devices (some may be both video and audio devices)
[dshow @ 000001e5b3a4c0c0]  "Integrated Camera" (video)
[dshow @ 000001e5b3a4c0c0]     Alternative name "@device_pnp_\\?\usb#vid_0bda&pid_5576&mi_00#6&1a#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global"
[dshow @ 000001e5b3a4c0c0]  "Elgato HD60 X" (video, audio)
[dshow @ 000001e5b3a4c0c0]     Alternative name "@device_pnp_\\?\usb#vid_0fd9"
[dshow @ 000001e5b3a4c0c0] DirectShow audio devices
[dshow @ 000001e5b3a4c0c0]  "Microphone (Realtek Audio)" (audio)
[dshow @ 000001e5b3a4c0c0]     Alternative name "@device_cm_{33D9A762}"
dummy: Immediate exit requested"#;

    const AVF: &str = r#"[AVFoundation indev @ 0x7f8b6a004380] AVFoundation video devices:
[AVFoundation indev @ 0x7f8b6a004380] [0] FaceTime HD Camera
[AVFoundation indev @ 0x7f8b6a004380] [1] USB3.0 Capture
[AVFoundation indev @ 0x7f8b6a004380] [2] Capture screen 0
[AVFoundation indev @ 0x7f8b6a004380] [3] Capture screen 1
[AVFoundation indev @ 0x7f8b6a004380] AVFoundation audio devices:
[AVFoundation indev @ 0x7f8b6a004380] [0] MacBook Pro Microphone
[AVFoundation indev @ 0x7f8b6a004380] [1] USB3.0 Capture
: Input/output error"#;

    #[test]
    fn dshow_parser_keeps_video_devices_only() {
        assert_eq!(parse_dshow_list(DSHOW), vec!["Integrated Camera".to_string(), "Elgato HD60 X".to_string()]);
        assert!(parse_dshow_list("").is_empty());
    }

    #[test]
    fn avfoundation_parser_skips_screens_and_audio() {
        assert_eq!(parse_avfoundation_list(AVF), vec![(0, "FaceTime HD Camera".to_string()), (1, "USB3.0 Capture".to_string())]);
    }

    #[test]
    fn sysfs_enumeration_skips_metadata_nodes() {
        let root = std::env::temp_dir().join(format!("captureone-v4l-{}", uuid::Uuid::new_v4()));
        for (n, name, idx) in [(0, "Cam Link 4K", "0"), (1, "Cam Link 4K", "1"), (10, "USB3 Video", "0"), (2, "HD Pro Webcam", "0")] {
            let d = root.join(format!("video{n}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("name"), format!("{name}\n")).unwrap();
            std::fs::write(d.join("index"), format!("{idx}\n")).unwrap();
        }
        std::fs::create_dir_all(root.join("v4l-subdev0")).unwrap();
        let l = list_v4l2(&root);
        let ids: Vec<_> = l.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["v4l2:/dev/video0", "v4l2:/dev/video2", "v4l2:/dev/video10"]);
        assert_eq!(l[0].name, "Cam Link 4K");
        assert_eq!(l[0].kind, SourceKind::Device);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(list_v4l2(&root).is_empty());
    }

    #[test]
    fn arg_builders() {
        let a = input_args("v4l2:/dev/video0", 30).unwrap();
        assert_eq!(a, ["-f", "v4l2", "-framerate", "30", "-i", "/dev/video0"]);
        let a = input_args("dshow:Elgato HD60 X", 60).unwrap();
        assert_eq!(a.last().unwrap(), "video=Elgato HD60 X");
        assert!(a.contains(&"dshow".to_string()));
        let a = input_args("avfoundation:1", 30).unwrap();
        assert_eq!(a.last().unwrap(), "1:none");
        assert!(input_args("v4l2:/etc/passwd", 30).is_err());
        assert!(input_args("avfoundation:x", 30).is_err());
        assert!(input_args("nope", 30).is_err());
        let full = ffmpeg_args("v4l2:/dev/video0", 25).unwrap();
        assert_eq!(full.last().unwrap(), "pipe:1");
        assert!(full.windows(2).any(|w| w == ["-pix_fmt", "bgra"]));
    }

    #[test]
    fn output_size_parsing() {
        let l = "  Stream #0:0: Video: rawvideo (BGRA / 0x41524742), bgra(progressive), 1280x720 [SAR 1:1 DAR 16:9], q=2-31, 110592 kb/s, 30 fps, 30 tbn";
        assert_eq!(parse_output_size(l), Some((1280, 720)));
        assert_eq!(parse_output_size("  Stream #0:0: Video: rawvideo (YUY2 / 0x32595559), yuyv422, 640x480"), None);
    }

    #[test]
    fn error_classification_and_device_list_detection() {
        assert_eq!(classify("/dev/video0: Device or resource busy").code, CaptureErrorCode::DeviceBusy);
        assert_eq!(classify("/dev/video9: No such file or directory").code, CaptureErrorCode::SourceNotFound);
        let t = "Devices:\n D. = Demuxing supported\n --\n D  lavfi           Libavfilter virtual input device\n DE video4linux2,v4l2 Video4Linux2 output device\n";
        assert!(devices_list_has(t, "v4l2"));
        assert!(!devices_list_has(t, "dshow"));
    }
}
