//! Real ScreenCaptureKit tests. Compiled and run only on macOS (GitHub Actions `macos-14`).
//!
//! * The permission-free tests (backend creation, capability report, permission queries)
//!   always run.
//! * Live capture tests need the Screen Recording TCC grant for the test process (or its
//!   responsible app) and only run with `CAPTUREONE_LIVE_CAPTURE=1`; otherwise they print a
//!   notice and return, so `cargo test` stays green on machines without the grant.
#![cfg(target_os = "macos")]

use capture_core::{
    AudioBuffer, AudioSink, AudioStreamOptions, CaptureBackend, CaptureErrorCode, CaptureTarget, FeatureKey,
    PermissionKind, PermissionState, Platform, Rect, SourceKind, StreamEvent, VideoFrame, VideoSink, VideoStreamOptions,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn live() -> bool {
    let on = std::env::var("CAPTUREONE_LIVE_CAPTURE").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipping live capture test (set CAPTUREONE_LIVE_CAPTURE=1 on a Mac with the Screen Recording grant)");
    }
    on
}

fn backend() -> Arc<dyn CaptureBackend> {
    macos_capture::create_backend().expect("ScreenCaptureKit backend")
}

#[test]
fn reports_honest_capabilities() {
    let b = backend();
    assert_eq!(b.platform(), Platform::Macos);
    let caps = b.capabilities();
    println!("{}", serde_json::to_string_pretty(&caps).unwrap());
    assert_eq!(caps.os_name, "macOS");
    assert!(caps.is_usable(FeatureKey::DisplayCapture));
    assert!(caps.is_usable(FeatureKey::Screenshot));
    assert!(caps.is_usable(FeatureKey::SystemAudio), "SCK audio is available on every supported runner (macOS 13+)");
    assert!(caps.max_recording_resolution.is_some(), "runner should have at least one active display");
}

#[test]
fn permission_queries_do_not_prompt_or_panic() {
    let b = backend();
    for kind in [
        PermissionKind::ScreenCapture,
        PermissionKind::SystemAudio,
        PermissionKind::Microphone,
        PermissionKind::Storage,
        PermissionKind::Notifications,
        PermissionKind::LocalNetwork,
    ] {
        let s = b.permission_status(kind);
        println!("{kind:?}: {:?} (settings: {}) {:?}", s.state, s.requires_system_settings, s.message);
        assert_eq!(s.kind, kind);
    }
    let screen = b.permission_status(PermissionKind::ScreenCapture).state;
    assert!(matches!(screen, PermissionState::Granted | PermissionState::Prompt | PermissionState::Denied));
}

#[test]
fn unknown_sources_are_reported_as_not_found() {
    if !live() {
        return;
    }
    let b = backend();
    let err = b.screenshot(&CaptureTarget::Display { display_id: "4294967290".into() }, false).unwrap_err();
    assert_eq!(err.code, CaptureErrorCode::SourceNotFound, "{err:?}");
    let err = b.screenshot(&CaptureTarget::Window { window_id: "not-a-number".into() }, false).unwrap_err();
    assert_eq!(err.code, CaptureErrorCode::SourceNotFound, "{err:?}");
    let err = b.screenshot(&CaptureTarget::Device { device_id: "x".into() }, false).unwrap_err();
    assert_eq!(err.code, CaptureErrorCode::Unsupported);
}

fn non_blank(frame: &VideoFrame) -> bool {
    let packed = frame.packed_bgra();
    let first = &packed[0..3];
    (0..packed.len() / 4).any(|i| &packed[i * 4..i * 4 + 3] != first)
}

#[test]
fn live_sources_and_screenshots() {
    if !live() {
        return;
    }
    let b = backend();
    let sources = b.list_sources(&[SourceKind::Display, SourceKind::Window]).expect("list_sources");
    println!("{}", serde_json::to_string_pretty(&sources).unwrap());
    let display = sources.iter().find(|s| s.kind == SourceKind::Display).expect("at least one display");
    assert!(display.width > 0 && display.height > 0);

    let target = CaptureTarget::Display { display_id: display.id.clone() };
    let t0 = Instant::now();
    let frame = b.screenshot(&target, true).expect("display screenshot");
    println!("display screenshot {}x{} in {:?}", frame.width, frame.height, t0.elapsed());
    assert_eq!((frame.width, frame.height), (display.width, display.height));
    assert!(non_blank(&frame), "display screenshot is a single flat colour");

    let rect = Rect { x: 10, y: 20, width: 320, height: 200 };
    let region = b.screenshot(&CaptureTarget::Region { display_id: display.id.clone(), rect }, false).expect("region screenshot");
    assert_eq!((region.width, region.height), (320, 200));

    // CI runs this suite a second time with CAPTUREONE_MACOS_FORCE_STREAM_SCREENSHOT=1, which
    // exercises the macOS 12.3–13 path (single-frame SCStream) on the newer runner OS.
    let forced = std::env::var(macos_capture::FORCE_STREAM_SCREENSHOT_ENV).is_ok_and(|v| v == "1");
    println!("screenshot path: {}", if forced { "single-frame SCStream (forced)" } else { "default for this macOS" });

    if let Some(window) = sources.iter().find(|s| s.kind == SourceKind::Window) {
        match b.screenshot(&CaptureTarget::Window { window_id: window.id.clone() }, false) {
            Ok(f) => println!("window '{}' screenshot {}x{} (listed {}x{})", window.name, f.width, f.height, window.width, window.height),
            Err(e) => panic!("window screenshot failed: {e:?}"),
        }
    }
}

#[derive(Default)]
struct CountingVideo {
    frames: AtomicU64,
    size: Mutex<Option<(u32, u32)>>,
    events: Mutex<Vec<String>>,
}
impl VideoSink for CountingVideo {
    fn on_frame(&self, frame: VideoFrame) {
        self.frames.fetch_add(1, Ordering::SeqCst);
        *self.size.lock().unwrap() = Some((frame.width, frame.height));
    }
    fn on_event(&self, event: StreamEvent) {
        self.events.lock().unwrap().push(format!("{event:?}"));
    }
}

#[test]
fn live_video_stream() {
    if !live() {
        return;
    }
    let b = backend();
    let sources = b.list_sources(&[SourceKind::Display]).expect("list_sources");
    let display = &sources[0];
    for target in [
        CaptureTarget::Display { display_id: display.id.clone() },
        CaptureTarget::Region { display_id: display.id.clone(), rect: Rect { x: 0, y: 0, width: 640, height: 360 } },
    ] {
        let sink = Arc::new(CountingVideo::default());
        let mut handle = b.start_video(&target, &VideoStreamOptions { fps: 30, show_cursor: true }, sink.clone()).expect("start_video");
        std::thread::sleep(Duration::from_secs(2));
        handle.stop().expect("stop");
        let after_stop = sink.frames.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(sink.frames.load(Ordering::SeqCst), after_stop, "frames delivered after stop()");
        let size = *sink.size.lock().unwrap();
        println!("{target:?}: {after_stop} frames in 2 s, size {size:?}, events {:?}", sink.events.lock().unwrap());
        // SCK only delivers complete frames on change (plus the first one); a static desktop may yield few.
        assert!(after_stop >= 1, "no frames captured");
        if let CaptureTarget::Region { rect, .. } = target {
            assert_eq!(size, Some((rect.width, rect.height)));
        } else {
            assert_eq!(size, Some((display.width, display.height)));
        }
    }
}

#[derive(Default)]
struct CountingAudio {
    buffers: AtomicU64,
    frames: AtomicU64,
    format: Mutex<Option<(u32, u16)>>,
}
impl AudioSink for CountingAudio {
    fn on_audio(&self, buffer: AudioBuffer) {
        self.buffers.fetch_add(1, Ordering::SeqCst);
        self.frames.fetch_add(buffer.frames() as u64, Ordering::SeqCst);
        *self.format.lock().unwrap() = Some((buffer.sample_rate, buffer.channels));
    }
    fn on_event(&self, event: StreamEvent) {
        println!("audio event: {event:?}");
    }
}

#[test]
fn live_system_audio_stream() {
    if !live() {
        return;
    }
    let b = backend();
    let sink = Arc::new(CountingAudio::default());
    let mut handle = b
        .start_system_audio(&AudioStreamOptions { sample_rate: 48_000, channels: 2, device_id: None }, sink.clone())
        .expect("start_system_audio");
    std::thread::sleep(Duration::from_secs(2));
    handle.stop().expect("stop");
    let (buffers, frames) = (sink.buffers.load(Ordering::SeqCst), sink.frames.load(Ordering::SeqCst));
    let format = *sink.format.lock().unwrap();
    println!("system audio: {buffers} buffers, {frames} frames, format {format:?}");
    if let Some((rate, channels)) = format {
        assert_eq!(rate, 48_000);
        assert_eq!(channels, 2);
    }
}
