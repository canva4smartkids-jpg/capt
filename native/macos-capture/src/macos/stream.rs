//! `SCStream` lifecycle: the Objective-C output/delegate object, capture planning
//! (filter + configuration per target), the running-stream handle and source monitoring.
//!
//! Threading model:
//! * Sample buffers arrive on a private **serial** dispatch queue per stream, where they are
//!   converted and handed to the sink synchronously (so frames reach the sink in order).
//! * `stream:didStopWithError:` arrives on an SCK-internal queue.
//! * Terminal events (`SourceClosed` / `Error`) are delivered from a fresh thread, so a sink
//!   that calls `StreamHandle::stop` from `on_event` cannot deadlock against our queue.
//! * A monitor thread polls (1 s) whether the captured display/window still exists, because
//!   ScreenCaptureKit does not reliably stop a stream when a window closes.

use super::content::{cg_rect, find_display, find_window, own_application, window_display_scale, DisplayInfo};
use super::sample::{self, FrameError};
use super::util::{capture_error_from_ns, spawn_detached, wait, Shared};
use super::{content, host};
use crate::capabilities::{supports_sonoma_stream_options, OsVersion};
use crate::errors::classify_stream_stop;
use crate::geometry::{region_plan, window_pixel_size, PointRect};
use crate::timing::frame_interval;
use block2::RcBlock;
use capture_core::{
    AudioSink, CaptureError, CaptureErrorCode, CaptureResult, CaptureTarget, Rect, StreamEvent, StreamHandle, VideoSink,
};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_foundation::{CFArray, CFRetained};
use objc2_core_graphics::{kCGColorSpaceSRGB, CGWindowListCopyWindowInfo, CGWindowListOption};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::kCVPixelFormatType_32BGRA;
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCFrameStatus, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const MONITOR_INTERVAL: Duration = Duration::from_millis(1000);
/// Frames ScreenCaptureKit may hold for us before it starts dropping (Apple recommends ≤ 8).
pub(crate) const VIDEO_QUEUE_DEPTH: isize = 6;

/// What a stream's capture source is, for existence probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceProbe {
    Display(u32),
    Window(u32),
}

impl SourceProbe {
    /// True while the display is connected / the window exists. Uses only CoreGraphics, which
    /// is cheap and thread-safe.
    pub(crate) fn exists(self) -> bool {
        match self {
            SourceProbe::Display(id) => host::display_is_online(id),
            SourceProbe::Window(id) => {
                let info: Option<CFRetained<CFArray>> = CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, id);
                // If the window server cannot answer at all, assume the window still exists
                // rather than ending a recording on a transient failure.
                match info {
                    Some(list) => !list.is_empty(),
                    None => true,
                }
            }
        }
    }
}

pub(crate) enum Mode {
    Video(Arc<dyn VideoSink>),
    Audio(Arc<dyn AudioSink>),
}

/// State shared between the handle, the Objective-C output object and helper threads.
pub(crate) struct StreamShared {
    mode: Mode,
    probe: Option<SourceProbe>,
    /// A terminal event was delivered (or the stream was stopped): drop further samples.
    ended: AtomicBool,
    /// `stop()` is in progress: suppress events caused by our own stop.
    stopping: AtomicBool,
    last_size: Mutex<Option<(u32, u32)>>,
    transient_failures: AtomicU64,
}

impl StreamShared {
    pub(crate) fn new(mode: Mode, probe: Option<SourceProbe>) -> Arc<Self> {
        Arc::new(Self {
            mode,
            probe,
            ended: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            last_size: Mutex::new(None),
            transient_failures: AtomicU64::new(0),
        })
    }

    fn inactive(&self) -> bool {
        self.ended.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst)
    }

    /// Marks the stream ended. Returns true if the caller should deliver a terminal event.
    fn mark_ended(&self) -> bool {
        let was_ended = self.ended.swap(true, Ordering::SeqCst);
        !was_ended && !self.stopping.load(Ordering::SeqCst)
    }

    fn emit(&self, event: StreamEvent) {
        match &self.mode {
            Mode::Video(sink) => sink.on_event(event),
            Mode::Audio(sink) => sink.on_event(event),
        }
    }

    /// Ends the stream with `event`, delivered from a fresh thread.
    fn finish(self: &Arc<Self>, event: StreamEvent) {
        if self.mark_ended() {
            log::info!("macos-capture: stream ended: {event:?}");
            let me = Arc::clone(self);
            spawn_detached("sck-stream-event", move || me.emit(event));
        }
    }

    /// Probes the source off the SCK queue and ends the stream if it is gone; with `error`,
    /// always ends the stream (as `SourceClosed` if the source vanished, else `Error`).
    fn probe_and_finish(self: &Arc<Self>, error: Option<CaptureError>) {
        if self.inactive() {
            return;
        }
        let me = Arc::clone(self);
        spawn_detached("sck-stream-probe", move || {
            let exists = match me.probe {
                Some(probe) => probe.exists(),
                None => true,
            };
            let event = match error {
                Some(err) => Some(classify_stream_stop(err, exists)),
                None if !exists => Some(StreamEvent::SourceClosed),
                None => None,
            };
            if let Some(event) = event {
                me.finish(event);
            }
        });
    }

    fn handle_sample(self: &Arc<Self>, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
        if self.inactive() {
            return;
        }
        match (&self.mode, kind) {
            (Mode::Video(sink), SCStreamOutputType::Screen) => self.handle_video(sink, sample),
            (Mode::Audio(sink), SCStreamOutputType::Audio) => {
                // SAFETY: SCK hands us a valid audio sample buffer for the duration of the call.
                match unsafe { sample::audio_buffer(sample) } {
                    Ok(Some(buf)) => sink.on_audio(buf),
                    Ok(None) => {}
                    Err(e) => self.note_transient(&e),
                }
            }
            // e.g. the tiny placeholder video frames of an audio-only stream.
            _ => {}
        }
    }

    fn handle_video(self: &Arc<Self>, sink: &Arc<dyn VideoSink>, sample: &CMSampleBuffer) {
        // SAFETY: SCK hands us a valid sample buffer for the duration of the call.
        unsafe {
            if !sample.is_valid() {
                return;
            }
            match sample::frame_status(sample) {
                Some(SCFrameStatus::Complete) | None => {}
                Some(SCFrameStatus::Stopped) => {
                    self.probe_and_finish(None);
                    return;
                }
                // Idle = nothing changed, Blank/Suspended/Started = no displayable content.
                // Skip; the engine's pacer repeats the previous frame, so timing is kept.
                Some(_) => return,
            }
        }
        // SAFETY: as above.
        let Some(pixel_buffer) = (unsafe { sample.image_buffer() }) else { return };
        let captured_at = unsafe { sample::captured_at(sample) };
        match sample::copy_bgra(&pixel_buffer, captured_at) {
            Ok(frame) => {
                let size = (frame.width, frame.height);
                let resized = {
                    let mut last = self.last_size.lock().unwrap_or_else(|p| p.into_inner());
                    let resized = last.is_some_and(|l| l != size);
                    *last = Some(size);
                    resized
                };
                if resized {
                    sink.on_event(StreamEvent::Resized { width: size.0, height: size.1 });
                }
                sink.on_frame(frame);
            }
            Err(FrameError::Format(e)) => self.finish(StreamEvent::Error(e)),
            Err(FrameError::Transient(e)) => self.note_transient(&e),
        }
    }

    fn note_transient(&self, e: &CaptureError) {
        let n = self.transient_failures.fetch_add(1, Ordering::Relaxed);
        if n < 5 || n % 500 == 0 {
            log::warn!("macos-capture: skipped a sample ({} so far): {}", n + 1, e.message);
        }
    }
}

/// Instance variables of the Objective-C output/delegate object.
pub(crate) struct OutputIvars {
    shared: Arc<StreamShared>,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - The class does not implement Drop; its only ivar is an Arc.
    // - Thread kind is the default (any thread): all state is Send + Sync.
    #[unsafe(super(NSObject))]
    #[name = "CaptureOneSCStreamOutput"]
    #[ivars = OutputIvars]
    pub(crate) struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        // Called on our serial sample queue.
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output_sample_buffer(&self, _stream: &SCStream, sample_buffer: &CMSampleBuffer, kind: SCStreamOutputType) {
            let shared = &self.ivars().shared;
            // Never unwind into Objective-C/libdispatch frames.
            if catch_unwind(AssertUnwindSafe(|| shared.handle_sample(sample_buffer, kind))).is_err() {
                shared.finish(StreamEvent::Error(CaptureError::internal("panic while handling a captured sample")));
            }
        }
    }

    unsafe impl SCStreamDelegate for StreamOutput {
        // Called on an SCK-internal queue when the stream stops on its own (window closed,
        // display gone, permission revoked, stopped from the system menu, …).
        #[unsafe(method(stream:didStopWithError:))]
        fn stream_did_stop_with_error(&self, _stream: &SCStream, error: &NSError) {
            let shared = &self.ivars().shared;
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let err = capture_error_from_ns(error);
                log::warn!("macos-capture: stream stopped by the system: {}", err.message);
                shared.probe_and_finish(Some(err));
            }));
        }

        // macOS 14+: every shared window was closed.
        #[unsafe(method(streamDidBecomeInactive:))]
        fn stream_did_become_inactive(&self, _stream: &SCStream) {
            let shared = &self.ivars().shared;
            let _ = catch_unwind(AssertUnwindSafe(|| shared.probe_and_finish(None)));
        }
    }
);

impl StreamOutput {
    fn new(shared: Arc<StreamShared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars { shared });
        // SAFETY: plain `-[NSObject init]` on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

/// A running `SCStream`. Stopping (explicitly or by drop) stops capture, waits for SCK,
/// removes the outputs and drains the sample queue so no sample reaches the sink afterwards.
pub(crate) struct ScStream {
    stream: Option<Shared<SCStream>>,
    output: Retained<StreamOutput>,
    output_types: Vec<SCStreamOutputType>,
    queue: DispatchRetained<DispatchQueue>,
    shared: Arc<StreamShared>,
    monitor: Option<(SyncSender<()>, JoinHandle<()>)>,
}

fn completion_block(tx: SyncSender<Option<CaptureError>>) -> RcBlock<dyn Fn(*mut NSError)> {
    RcBlock::new(move |error: *mut NSError| {
        // SAFETY: SCK passes null or a valid NSError borrowed for the call.
        let result = unsafe { error.as_ref() }.map(capture_error_from_ns);
        let _ = tx.try_send(result);
    })
}

impl ScStream {
    /// Creates, wires up and starts a stream.
    pub(crate) fn start(
        filter: &SCContentFilter,
        config: &SCStreamConfiguration,
        shared: Arc<StreamShared>,
        output_types: &[SCStreamOutputType],
        queue_label: &str,
    ) -> CaptureResult<ScStream> {
        let output = StreamOutput::new(Arc::clone(&shared));
        let delegate: &ProtocolObject<dyn SCStreamDelegate> = ProtocolObject::from_ref(&*output);
        // SAFETY: valid filter/config; the delegate is kept alive by `ScStream::output` for the
        // stream's whole lifetime (SCK does not necessarily retain it).
        let stream = unsafe { SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), filter, config, Some(delegate)) };
        let queue = DispatchQueue::new(queue_label, DispatchQueueAttr::SERIAL);
        let mut handle = ScStream {
            stream: Some(Shared(stream)),
            output,
            output_types: Vec::new(),
            queue,
            shared,
            monitor: None,
        };
        for &kind in output_types {
            let out: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*handle.output);
            let stream = &handle.stream.as_ref().ok_or_else(|| CaptureError::internal("stream released"))?.0;
            // SAFETY: the output object and queue outlive the stream (both owned by `handle`).
            unsafe { stream.addStreamOutput_type_sampleHandlerQueue_error(out, kind, Some(&handle.queue)) }
                .map_err(|e| capture_error_from_ns(&e))?;
            handle.output_types.push(kind);
        }

        let (tx, rx) = mpsc::sync_channel(1);
        let block = completion_block(tx);
        if let Some(stream) = &handle.stream {
            // SAFETY: heap block copied by SCK; invoked once on an SCK queue.
            unsafe { stream.0.startCaptureWithCompletionHandler(Some(&block)) };
        }
        match wait(&rx, START_TIMEOUT, "ScreenCaptureKit to start capturing") {
            Ok(None) => Ok(handle),
            Ok(Some(err)) => {
                let _ = handle.shutdown();
                Err(err)
            }
            Err(timeout) => {
                let _ = handle.shutdown();
                Err(timeout)
            }
        }
    }

    /// Starts the source monitor thread.
    pub(crate) fn monitor(&mut self, probe: SourceProbe) -> CaptureResult<()> {
        let (tx, rx) = mpsc::sync_channel::<()>(0);
        let shared = Arc::clone(&self.shared);
        let handle = std::thread::Builder::new()
            .name("sck-source-monitor".into())
            .spawn(move || {
                // Ends when the handle drops the sender (Disconnected) or signals.
                while let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(MONITOR_INTERVAL) {
                    if shared.inactive() {
                        break;
                    }
                    if !probe.exists() {
                        shared.finish(StreamEvent::SourceClosed);
                        break;
                    }
                }
            })
            .map_err(|e| CaptureError::internal(format!("could not start the source monitor: {e}")))?;
        self.monitor = Some((tx, handle));
        Ok(())
    }

    fn shutdown(&mut self) -> CaptureResult<()> {
        let Some(stream) = self.stream.take() else { return Ok(()) };
        let ended_before = self.shared.ended.load(Ordering::SeqCst);
        self.shared.stopping.store(true, Ordering::SeqCst);
        if let Some((tx, thread)) = self.monitor.take() {
            drop(tx);
            let _ = thread.join();
        }

        let (tx, rx) = mpsc::sync_channel(1);
        let block = completion_block(tx);
        // SAFETY: heap block copied by SCK.
        unsafe { stream.0.stopCaptureWithCompletionHandler(Some(&block)) };
        let stop_result = wait(&rx, STOP_TIMEOUT, "ScreenCaptureKit to stop capturing");

        let out: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*self.output);
        for &kind in &self.output_types {
            // SAFETY: removes an output we added; failures (already removed) are harmless.
            let _ = unsafe { stream.0.removeStreamOutput_type_error(out, kind) };
        }
        // Barrier on the serial sample queue: samples already queued run (and are dropped
        // because `stopping` is set) before we return.
        let (btx, brx) = mpsc::sync_channel::<()>(1);
        self.queue.exec_async(move || {
            let _ = btx.try_send(());
        });
        let _ = brx.recv_timeout(DRAIN_TIMEOUT);
        self.shared.ended.store(true, Ordering::SeqCst);
        drop(stream);

        match stop_result {
            Ok(None) => Ok(()),
            // The stream had already stopped on its own (error already reported) or never
            // started: stopping it again reports a state error, which is not a failure.
            Ok(Some(e)) if ended_before || e.code == CaptureErrorCode::InvalidState => Ok(()),
            Ok(Some(e)) => Err(e),
            Err(e) => Err(e),
        }
    }
}

impl StreamHandle for ScStream {
    fn stop(&mut self) -> CaptureResult<()> {
        self.shutdown()
    }
}

impl Drop for ScStream {
    fn drop(&mut self) {
        if self.stream.is_some() {
            if let Err(e) = self.shutdown() {
                log::warn!("macos-capture: error while releasing a capture stream: {}", e.message);
            }
        }
    }
}

/// Filter, output size and options for a capture target.
pub(crate) struct VideoPlan {
    pub filter: Retained<SCContentFilter>,
    pub width: u32,
    pub height: u32,
    /// Display-local `sourceRect` in points (native region capture).
    pub source_rect: Option<PointRect>,
    pub is_window: bool,
    pub probe: SourceProbe,
    /// Pixel crop to apply to full-display frames (screenshots of regions).
    pub crop: Option<Rect>,
}

/// Display filter that keeps CaptureOne's own windows (and audio) out of the capture.
fn display_filter(content: &SCShareableContent, display: &SCDisplay) -> Retained<SCContentFilter> {
    let no_windows: Retained<NSArray<SCWindow>> = NSArray::new();
    // SAFETY: designated initialisers with valid SCK objects.
    unsafe {
        match own_application(content) {
            Some(app) => {
                let apps = NSArray::from_retained_slice(&[app]);
                SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(SCContentFilter::alloc(), display, &apps, &no_windows)
            }
            None => SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), display, &no_windows),
        }
    }
}

/// Plans a capture. `native_region`: true → regions use `sourceRect` (video); false → capture
/// the whole display and crop in pixels (screenshots, exact pixel semantics).
pub(crate) fn plan(os: OsVersion, content: &SCShareableContent, target: &CaptureTarget, native_region: bool) -> CaptureResult<VideoPlan> {
    match target {
        CaptureTarget::Display { display_id } => {
            let (display, info) = find_display(content, display_id)?;
            Ok(VideoPlan {
                filter: display_filter(content, &display),
                width: info.pixel_size.0,
                height: info.pixel_size.1,
                source_rect: None,
                is_window: false,
                probe: SourceProbe::Display(info.id),
                crop: None,
            })
        }
        CaptureTarget::Region { display_id, rect } => {
            let (display, info) = find_display(content, display_id)?;
            let region = region_plan(*rect, info.pixel_size, info.scale)?;
            let filter = display_filter(content, &display);
            let probe = SourceProbe::Display(info.id);
            Ok(if native_region {
                VideoPlan {
                    filter,
                    width: region.pixel_rect.width,
                    height: region.pixel_rect.height,
                    source_rect: Some(region.source_rect),
                    is_window: false,
                    probe,
                    crop: None,
                }
            } else {
                VideoPlan {
                    filter,
                    width: info.pixel_size.0,
                    height: info.pixel_size.1,
                    source_rect: None,
                    is_window: false,
                    probe,
                    crop: Some(region.pixel_rect),
                }
            })
        }
        CaptureTarget::Window { window_id } => {
            let window = find_window(content, window_id)?;
            // SAFETY: designated initialiser / getters on valid SCK objects.
            let filter = unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window) };
            let (id, frame) = unsafe { (window.windowID(), content::point_rect(window.frame())) };
            let scale = if supports_sonoma_stream_options(os) {
                // macOS 14+: exact points→pixels scale for this filter.
                // SAFETY: class method available on 14.0+ (gated above).
                let s = unsafe { SCShareableContent::infoForFilter(&filter).pointPixelScale() } as f64;
                if s.is_finite() && s > 0.0 {
                    s
                } else {
                    1.0
                }
            } else {
                let displays: Vec<DisplayInfo> = content::displays(content).into_iter().map(|(_, i)| i).collect();
                window_display_scale(&frame, &displays)
            };
            let (width, height) = window_pixel_size(&frame, scale);
            Ok(VideoPlan { filter, width, height, source_rect: None, is_window: true, probe: SourceProbe::Window(id), crop: None })
        }
        CaptureTarget::Device { .. } => Err(CaptureError::unsupported("capture devices are not supported by the macOS backend")),
    }
}

/// Stream configuration for video/screenshots: BGRA, sRGB, pixel-exact output size.
pub(crate) fn video_config(os: OsVersion, plan: &VideoPlan, fps: u32, show_cursor: bool, queue_depth: isize) -> Retained<SCStreamConfiguration> {
    let (value, timescale) = frame_interval(fps);
    // SAFETY: setters on a fresh configuration object; every selector used outside the
    // `supports_sonoma_stream_options` block exists since macOS 12.3.
    unsafe {
        let config = SCStreamConfiguration::new();
        config.setWidth(plan.width as usize);
        config.setHeight(plan.height as usize);
        config.setMinimumFrameInterval(CMTime::new(value, timescale));
        config.setPixelFormat(kCVPixelFormatType_32BGRA);
        config.setShowsCursor(show_cursor);
        config.setQueueDepth(queue_depth);
        config.setColorSpaceName(kCGColorSpaceSRGB);
        if let Some(rect) = plan.source_rect {
            config.setSourceRect(cg_rect(rect));
        }
        if plan.is_window {
            // Keep the output size constant when the window is resized: scale the content.
            config.setScalesToFit(true);
            if supports_sonoma_stream_options(os) {
                config.setPreservesAspectRatio(true);
                config.setIgnoreShadowsSingleWindow(true);
            }
        }
        config
    }
}

pub(crate) fn start_video_plan(os: OsVersion, plan: &VideoPlan, fps: u32, show_cursor: bool, sink: Arc<dyn VideoSink>, monitor: bool) -> CaptureResult<ScStream> {
    let config = video_config(os, plan, fps, show_cursor, VIDEO_QUEUE_DEPTH);
    let shared = StreamShared::new(Mode::Video(sink), Some(plan.probe));
    let mut stream = ScStream::start(&plan.filter, &config, shared, &[SCStreamOutputType::Screen], "com.captureone.sck.video")?;
    if monitor {
        stream.monitor(plan.probe)?;
    }
    Ok(stream)
}

pub(crate) fn start_video(os: OsVersion, target: &CaptureTarget, fps: u32, show_cursor: bool, sink: Arc<dyn VideoSink>) -> CaptureResult<Box<dyn StreamHandle>> {
    let content = content::shareable_content(false)?;
    let plan = plan(os, &content, target, true)?;
    log::info!(
        "macos-capture: starting video {}x{} @ {fps} fps (window: {}, sourceRect: {:?})",
        plan.width,
        plan.height,
        plan.is_window,
        plan.source_rect
    );
    let stream = start_video_plan(os, &plan, fps, show_cursor, sink, true)?;
    Ok(Box::new(stream))
}

/// System audio through a dedicated audio-only `SCStream` on the main display (2×2 px,
/// 1 fps placeholder video, which is discarded). A separate stream keeps audio independent of
/// the video target (window captures would otherwise only include that app's audio) and lets
/// media-engine start/stop audio and video separately.
pub(crate) fn start_system_audio(os: OsVersion, sample_rate: u32, channels: u16, sink: Arc<dyn AudioSink>) -> CaptureResult<Box<dyn StreamHandle>> {
    if !crate::capabilities::supports_system_audio(os) {
        return Err(CaptureError::unsupported(format!("ScreenCaptureKit system audio requires macOS 13.0 (this Mac runs {os})"))
            .with_detail("minOsVersion", "13.0"));
    }
    let content = content::shareable_content(true)?;
    let main = objc2_core_graphics::CGMainDisplayID();
    let displays = content::displays(&content);
    let (display, info) = displays
        .iter()
        .find(|(_, i)| i.id == main)
        .or_else(|| displays.first())
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "no display available for the system audio stream"))?;
    let no_windows: Retained<NSArray<SCWindow>> = NSArray::new();
    // SAFETY: designated initialiser with valid objects. Including every application means the
    // audio of every app is captured; our own audio is excluded via the configuration.
    let filter = unsafe { SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), display, &no_windows) };
    let rate = if [44_100, 48_000].contains(&sample_rate) { sample_rate } else { 48_000 };
    let channels = channels.clamp(1, 2);
    // SAFETY: setters on a fresh configuration; audio selectors exist since macOS 13 (checked).
    let config = unsafe {
        let c = SCStreamConfiguration::new();
        c.setWidth(2);
        c.setHeight(2);
        c.setMinimumFrameInterval(CMTime::new(1, 1));
        c.setPixelFormat(kCVPixelFormatType_32BGRA);
        c.setShowsCursor(false);
        c.setQueueDepth(3);
        c.setCapturesAudio(true);
        c.setSampleRate(rate as isize);
        c.setChannelCount(channels as isize);
        c.setExcludesCurrentProcessAudio(true);
        c
    };
    log::info!("macos-capture: starting system audio {rate} Hz x{channels} (display {})", info.id);
    // No source probe: the audio stream is not tied to a user-chosen source, so a stop is
    // reported as an error rather than `SourceClosed`.
    let shared = StreamShared::new(Mode::Audio(sink), None);
    // Register a screen output too: SCK logs and drops otherwise, and some versions refuse to
    // start a stream whose video frames have no consumer.
    let stream = ScStream::start(
        &filter,
        &config,
        shared,
        &[SCStreamOutputType::Screen, SCStreamOutputType::Audio],
        "com.captureone.sck.audio",
    )?;
    Ok(Box::new(stream))
}
