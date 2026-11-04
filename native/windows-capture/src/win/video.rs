//! Windows Graphics Capture sessions: screenshots (first frame) and video streams.
//!
//! The `windows-capture` crate owns the capture thread, its DispatcherQueue/message loop, the
//! D3D11 device, the frame pool (re-created on size changes) and the item `Closed` event. This
//! module implements its handler: every frame is copied on the GPU into a reusable staging
//! texture, then mapped and packed (row padding removed) into a BGRA `VideoFrame`.
//!
//! Pacing: WGC raises a frame per compositor update (up to the display refresh rate). Frames are
//! read back at most at ~`fps`; a skipped frame stays in the staging texture and a thread timer
//! delivers it once the interval elapses, so the final state after a burst of changes is never
//! lost even though WGC sends nothing more until the next change.

use super::clock::instant_from_qpc_hns;
use crate::gating::{WgcFeatures, WgcProbe};
use crate::pixels::{pack_rows, required_len};
use crate::timing::DeliveryPacer;
use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, StreamEvent, StreamHandle, VideoFrame, VideoSink};
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
use std::thread::ThreadId;
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostQuitMessage, SetTimer};
use windows_capture::capture::{CaptureControl, CaptureControlError, Context, GraphicsCaptureApiError, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::{self, GraphicsCaptureApi, InternalCaptureControl};
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

/// How long a screenshot waits for the first WGC frame.
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(5);

/// What to capture.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CaptureItem {
    Monitor(HMONITOR),
    Window(HWND),
}

/// Runtime WGC feature probes (`ApiInformation` + `GraphicsCaptureSession::IsSupported`).
pub(crate) fn probe_wgc() -> WgcProbe {
    // Probe on a fresh MTA thread: activation factories need COM, and the caller's apartment
    // must not be changed.
    std::thread::Builder::new()
        .name("captureone-wgc-probe".into())
        .spawn(|| {
            // SAFETY: balanced below.
            let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let probe = WgcProbe {
                api_supported: GraphicsCaptureApi::is_supported().ok(),
                cursor_toggle: GraphicsCaptureApi::is_cursor_settings_supported().ok(),
                border_toggle: GraphicsCaptureApi::is_border_settings_supported().ok(),
                min_update_interval: GraphicsCaptureApi::is_minimum_update_interval_supported().ok(),
            };
            if init.is_ok() {
                // SAFETY: matches the successful CoInitializeEx.
                unsafe { CoUninitialize() };
            }
            probe
        })
        .ok()
        .and_then(|h| h.join().ok())
        .unwrap_or_default()
}

/// Windows 11 asks unpackaged apps to request borderless capture once per process; for desktop
/// apps the request is granted without UI. Failures are ignored (the session start falls back
/// to the default border).
fn request_borderless_access() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("captureone-wgc-access".into()).spawn(move || {
            // SAFETY: balanced below.
            let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let result = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless).and_then(|op| op.join());
            if init.is_ok() {
                // SAFETY: matches the successful CoInitializeEx.
                unsafe { CoUninitialize() };
            }
            let _ = tx.send(result.map(|s| s.0));
        });
        if spawned.is_ok() {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(Ok(status)) => log::debug!("borderless capture access status: {status}"),
                Ok(Err(e)) => log::debug!("borderless capture access request failed: {e}"),
                Err(_) => log::debug!("borderless capture access request timed out"),
            }
        }
    });
}

enum Delivery {
    /// Video stream: every (paced) frame goes to the sink.
    Stream(Arc<dyn VideoSink>),
    /// Screenshot: the first frame (or error) is sent once.
    Once(Option<mpsc::SyncSender<CaptureResult<VideoFrame>>>),
}

pub(crate) struct HandlerFlags {
    delivery: Delivery,
    fps: u32,
    /// Set by `StreamHandle::stop` (or after a terminal event) so nothing is delivered later.
    terminal: Arc<AtomicBool>,
    /// Filled in on the capture thread so `stop()` can detect being called from a sink callback.
    capture_thread: Arc<OnceLock<ThreadId>>,
}

struct Staging {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

enum Flow {
    Continue,
    Stop,
}

struct Inner {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    staging: Option<Staging>,
    /// Capture time of content copied into `staging` but not yet delivered.
    pending: Option<Instant>,
    pacer: DeliveryPacer,
    last_size: Option<(u32, u32)>,
    delivery: Delivery,
    terminal: Arc<AtomicBool>,
    done: bool,
}

struct Shared(Mutex<Inner>);

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

thread_local! {
    /// The session served by this capture thread (each WGC session has its own thread).
    static TIMER_TARGET: RefCell<Weak<Shared>> = const { RefCell::new(Weak::new()) };
}

unsafe extern "system" fn flush_timer(_: HWND, _: u32, _: usize, _: u32) {
    let target = TIMER_TARGET.with(|t| t.borrow().upgrade());
    if let Some(shared) = target {
        // try_lock: never block the capture thread here (the item `Closed` handler may hold the
        // lock on another thread); a busy lock just means the next tick flushes.
        let stop = catch_unwind(AssertUnwindSafe(|| match shared.0.try_lock() {
            Ok(mut inner) => matches!(inner.on_timer(), Flow::Stop),
            Err(std::sync::TryLockError::Poisoned(p)) => matches!(p.into_inner().on_timer(), Flow::Stop),
            Err(std::sync::TryLockError::WouldBlock) => false,
        }))
        .unwrap_or(true);
        if stop {
            // Ends the windows-capture message loop on this (capture) thread.
            // SAFETY: plain Win32 call on the current thread.
            unsafe { PostQuitMessage(0) };
        }
    }
}

fn d3d_error(what: &str, e: &windows::core::Error) -> CaptureError {
    let code = e.code();
    if code == DXGI_ERROR_DEVICE_REMOVED || code == DXGI_ERROR_DEVICE_RESET {
        CaptureError::new(CaptureErrorCode::Interrupted, format!("{what}: the GPU device was removed or reset ({e})"))
    } else {
        CaptureError::internal(format!("{what}: {e}"))
    }
}

impl Inner {
    fn ensure_staging(&mut self, width: u32, height: u32, format: DXGI_FORMAT) -> CaptureResult<()> {
        if self
            .staging
            .as_ref()
            .is_some_and(|s| s.width == width && s.height == height && s.format == format)
        {
            return Ok(());
        }
        self.staging = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: valid descriptor and out pointer.
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut texture)) }.map_err(|e| d3d_error("cannot create the staging texture", &e))?;
        let texture = texture.ok_or_else(|| CaptureError::internal("CreateTexture2D returned no texture"))?;
        self.staging = Some(Staging {
            texture,
            width,
            height,
            format,
        });
        Ok(())
    }

    /// Maps the staging texture and copies it into a tightly packed BGRA frame.
    fn read_staged(&self) -> CaptureResult<VideoFrame> {
        let st = self.staging.as_ref().ok_or_else(|| CaptureError::internal("no staged frame"))?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture was created with CPU read access on this device.
        unsafe { self.context.Map(&st.texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.map_err(|e| d3d_error("cannot map the captured frame", &e))?;
        let pitch = mapped.RowPitch as usize;
        let row = st.width as usize * 4;
        let packed = if mapped.pData.is_null() {
            Err("Map returned a null pointer".to_string())
        } else {
            // SAFETY: a mapped subresource of `height` rows is readable for
            // pitch * (height - 1) + row bytes; it stays mapped until Unmap below.
            let src = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, required_len(pitch, row, st.height as usize)) };
            pack_rows(src, pitch, row, st.height as usize)
        };
        // SAFETY: matches the successful Map above.
        unsafe { self.context.Unmap(&st.texture, 0) };
        let data = packed.map_err(|e| CaptureError::internal(format!("frame readback failed: {e}")))?;
        VideoFrame::new_bgra(st.width, st.height, st.width * 4, data)
    }

    fn on_frame(&mut self, frame: &mut Frame) -> Flow {
        if self.done {
            return Flow::Stop;
        }
        if self.terminal.load(Ordering::Acquire) {
            return Flow::Continue; // a stop is in progress; windows-capture ends the loop
        }
        let desc = *frame.desc();
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return self.fail(CaptureError::internal(format!("unexpected WGC pixel format {}", desc.Format.0)));
        }
        let captured_at = frame.timestamp().map(|t| instant_from_qpc_hns(t.Duration)).unwrap_or_else(|_| Instant::now());
        if let Err(e) = self.ensure_staging(desc.Width, desc.Height, desc.Format) {
            return self.fail(e);
        }
        if let Some(st) = &self.staging {
            // SAFETY: both textures belong to this device and have identical size/format.
            unsafe { self.context.CopyResource(&st.texture, frame.as_raw_texture()) };
        }
        self.pending = Some(captured_at);
        if matches!(self.delivery, Delivery::Once(_)) {
            let result = self.read_staged().map(|mut f| {
                f.captured_at = captured_at;
                f
            });
            if let Delivery::Once(tx) = &mut self.delivery {
                if let Some(tx) = tx.take() {
                    let _ = tx.send(result);
                }
            }
            self.done = true;
            return Flow::Stop;
        }
        if self.pacer.due(Instant::now()) {
            self.deliver()
        } else {
            // Submit the copy now; the timer delivers it if no newer frame arrives.
            // SAFETY: plain immediate-context call on the capture thread.
            unsafe { self.context.Flush() };
            Flow::Continue
        }
    }

    fn on_timer(&mut self) -> Flow {
        if self.done || self.terminal.load(Ordering::Acquire) || self.pending.is_none() {
            return Flow::Continue;
        }
        if self.pacer.due(Instant::now()) {
            self.deliver()
        } else {
            Flow::Continue
        }
    }

    fn deliver(&mut self) -> Flow {
        let Some(captured_at) = self.pending.take() else {
            return Flow::Continue;
        };
        let mut frame = match self.read_staged() {
            Ok(f) => f,
            Err(e) => return self.fail(e),
        };
        frame.captured_at = captured_at;
        self.pacer.mark_delivered(Instant::now());
        let size = (frame.width, frame.height);
        let resized = self.last_size.is_some_and(|prev| prev != size);
        self.last_size = Some(size);
        if let Delivery::Stream(sink) = &self.delivery {
            if resized {
                sink.on_event(StreamEvent::Resized { width: size.0, height: size.1 });
            }
            sink.on_frame(frame);
        }
        Flow::Continue
    }

    fn fail(&mut self, err: CaptureError) -> Flow {
        if !self.done {
            self.done = true;
            match &mut self.delivery {
                Delivery::Once(tx) => {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(Err(err));
                    }
                }
                Delivery::Stream(sink) => {
                    if !self.terminal.swap(true, Ordering::AcqRel) {
                        sink.on_event(StreamEvent::Error(err));
                    }
                }
            }
        }
        Flow::Stop
    }

    fn closed(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        match &mut self.delivery {
            Delivery::Once(tx) => {
                if let Some(tx) = tx.take() {
                    let _ = tx.send(Err(CaptureError::new(CaptureErrorCode::SourceClosed, "the captured window/display was closed")));
                }
            }
            Delivery::Stream(sink) => {
                if !self.terminal.swap(true, Ordering::AcqRel) {
                    sink.on_event(StreamEvent::SourceClosed);
                }
            }
        }
    }
}

/// `windows-capture` handler; created on the capture thread.
pub(crate) struct WgcHandler {
    shared: Arc<Shared>,
}

impl GraphicsCaptureApiHandler for WgcHandler {
    type Flags = HandlerFlags;
    type Error = CaptureError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let HandlerFlags {
            delivery,
            fps,
            terminal,
            capture_thread,
        } = ctx.flags;
        let _ = capture_thread.set(std::thread::current().id());
        let streaming = matches!(delivery, Delivery::Stream(_));
        let pacer = DeliveryPacer::new(if streaming { fps } else { 0 });
        let period = pacer.flush_period_ms();
        let shared = Arc::new(Shared(Mutex::new(Inner {
            device: ctx.device,
            context: ctx.device_context,
            staging: None,
            pending: None,
            pacer,
            last_size: None,
            delivery,
            terminal,
            done: false,
        })));
        if streaming && fps > 0 {
            TIMER_TARGET.with(|t| *t.borrow_mut() = Arc::downgrade(&shared));
            // Thread timer: WM_TIMER is dispatched by windows-capture's message loop on this
            // thread, which calls `flush_timer`. It dies with the thread.
            // SAFETY: plain Win32 call; the callback is a valid extern "system" fn.
            let id = unsafe { SetTimer(None, 0, period, Some(flush_timer)) };
            if id == 0 {
                log::warn!("SetTimer failed; paced frames are flushed only when new frames arrive");
            }
        }
        Ok(Self { shared })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, capture_control: InternalCaptureControl) -> Result<(), Self::Error> {
        let flow = catch_unwind(AssertUnwindSafe(|| self.shared.lock().on_frame(frame)))
            .unwrap_or_else(|_| self.shared.lock().fail(CaptureError::internal("panic while handling a captured frame")));
        if matches!(flow, Flow::Stop) {
            capture_control.stop();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        let _ = catch_unwind(AssertUnwindSafe(|| self.shared.lock().closed()));
        Ok(())
    }
}

type Control = CaptureControl<WgcHandler, CaptureError>;

fn map_start_error(e: GraphicsCaptureApiError<CaptureError>) -> CaptureError {
    match e {
        GraphicsCaptureApiError::ItemConvertFailed => CaptureError::new(
            CaptureErrorCode::SourceNotFound,
            "the display/window cannot be captured (it may have been closed or is not capturable)",
        ),
        GraphicsCaptureApiError::NewHandlerError(e) | GraphicsCaptureApiError::FrameHandlerError(e) => e,
        GraphicsCaptureApiError::GraphicsCaptureApiError(graphics_capture_api::Error::Unsupported) => {
            CaptureError::unsupported("Windows Graphics Capture is not supported on this system")
        }
        GraphicsCaptureApiError::DirectXError(d) => {
            CaptureError::new(CaptureErrorCode::DeviceBusy, format!("cannot create a Direct3D 11 device for capture: {d}"))
        }
        other => CaptureError::internal(format!("Windows Graphics Capture failed to start: {other}")),
    }
}

fn map_control_error(e: CaptureControlError<CaptureError>) -> CaptureError {
    match e {
        CaptureControlError::StoppedHandlerError(e) => e,
        CaptureControlError::GraphicsCaptureApiError(inner) => map_start_error(inner),
        other => CaptureError::internal(format!("stopping the capture thread failed: {other}")),
    }
}

/// Starts a WGC session on its own thread. `make_delivery` may be called twice (border fallback).
fn start_session(
    item: CaptureItem,
    wgc: &WgcFeatures,
    show_cursor: bool,
    fps: u32,
    make_delivery: &dyn Fn() -> Delivery,
    terminal: &Arc<AtomicBool>,
    capture_thread: &Arc<OnceLock<ThreadId>>,
) -> CaptureResult<Control> {
    if !wgc.capture {
        return Err(CaptureError::unsupported(
            "Windows Graphics Capture is unavailable (requires Windows 10 1903+ with an interactive desktop session)",
        ));
    }
    let cursor = match (wgc.cursor_toggle, show_cursor) {
        (true, true) => CursorCaptureSettings::WithCursor,
        (true, false) => CursorCaptureSettings::WithoutCursor,
        (false, _) => CursorCaptureSettings::Default,
    };
    let min_interval = if wgc.min_update_interval && fps > 0 {
        MinimumUpdateIntervalSettings::Custom(Duration::from_nanos(1_000_000_000 / fps as u64))
    } else {
        MinimumUpdateIntervalSettings::Default
    };
    let borders: &[DrawBorderSettings] = if wgc.border_toggle {
        request_borderless_access();
        &[DrawBorderSettings::WithoutBorder, DrawBorderSettings::Default]
    } else {
        &[DrawBorderSettings::Default]
    };
    let mut last_err = None;
    for &border in borders {
        let flags = HandlerFlags {
            delivery: make_delivery(),
            fps,
            terminal: terminal.clone(),
            capture_thread: capture_thread.clone(),
        };
        let started = match item {
            CaptureItem::Monitor(h) => WgcHandler::start_free_threaded(Settings::new(
                Monitor::from_raw_hmonitor(h.0),
                cursor,
                border,
                SecondaryWindowSettings::Default,
                min_interval,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                flags,
            )),
            CaptureItem::Window(h) => WgcHandler::start_free_threaded(Settings::new(
                Window::from_raw_hwnd(h.0),
                cursor,
                border,
                SecondaryWindowSettings::Default,
                min_interval,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                flags,
            )),
        };
        match started {
            Ok(control) => return Ok(control),
            Err(e) => {
                let retry = border == DrawBorderSettings::WithoutBorder && matches!(e, GraphicsCaptureApiError::GraphicsCaptureApiError(_));
                log::warn!("WGC start failed (border {border:?}): {e}");
                last_err = Some(map_start_error(e));
                if !retry {
                    break;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| CaptureError::internal("Windows Graphics Capture did not start")))
}

/// Captures one frame of `item`.
pub(crate) fn screenshot(item: CaptureItem, wgc: &WgcFeatures, show_cursor: bool) -> CaptureResult<VideoFrame> {
    let (tx, rx) = mpsc::sync_channel::<CaptureResult<VideoFrame>>(1);
    let terminal = Arc::new(AtomicBool::new(false));
    let control = {
        let make = || Delivery::Once(Some(tx.clone()));
        start_session(item, wgc, show_cursor, 0, &make, &terminal, &Arc::new(OnceLock::new()))?
    };
    drop(tx);
    let received = rx.recv_timeout(SCREENSHOT_TIMEOUT);
    terminal.store(true, Ordering::Release);
    let stopped = control.stop();
    match received {
        Ok(result) => {
            if let Err(e) = stopped {
                log::warn!("screenshot capture thread did not stop cleanly: {}", map_control_error(e));
            }
            result
        }
        Err(mpsc::RecvTimeoutError::Timeout) => Err(CaptureError::new(
            CaptureErrorCode::Interrupted,
            format!(
                "no frame received within {} s (is the window minimized or the display off?)",
                SCREENSHOT_TIMEOUT.as_secs()
            ),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(match stopped {
            Err(e) => map_control_error(e),
            Ok(()) => CaptureError::internal("the capture thread ended without delivering a frame"),
        }),
    }
}

/// A running WGC video stream.
pub(crate) struct WgcStream {
    control: Option<Control>,
    terminal: Arc<AtomicBool>,
    capture_thread: Arc<OnceLock<ThreadId>>,
}

impl StreamHandle for WgcStream {
    fn stop(&mut self) -> CaptureResult<()> {
        self.terminal.store(true, Ordering::Release);
        let Some(control) = self.control.take() else {
            return Ok(());
        };
        if self.capture_thread.get() == Some(&std::thread::current().id()) {
            // stop() called from a sink callback on the capture thread: joining it here would
            // deadlock, so the join happens on a helper thread once the callback returns.
            std::thread::spawn(move || {
                if let Err(e) = control.stop() {
                    log::warn!("deferred WGC stop failed: {}", map_control_error(e));
                }
            });
            return Ok(());
        }
        // Posts WM_QUIT to the capture thread and joins it; the session, frame pool, D3D device
        // and our staging texture are released when the thread and handler drop.
        control.stop().map_err(map_control_error)
    }
}

impl Drop for WgcStream {
    fn drop(&mut self) {
        if let Err(e) = self.stop() {
            log::warn!("WGC stream cleanup failed: {e}");
        }
    }
}

pub(crate) fn start_stream(item: CaptureItem, wgc: &WgcFeatures, show_cursor: bool, fps: u32, sink: Arc<dyn VideoSink>) -> CaptureResult<WgcStream> {
    let terminal = Arc::new(AtomicBool::new(false));
    let capture_thread = Arc::new(OnceLock::new());
    let make = || Delivery::Stream(sink.clone());
    let control = start_session(item, wgc, show_cursor, fps, &make, &terminal, &capture_thread)?;
    Ok(WgcStream {
        control: Some(control),
        terminal,
        capture_thread,
    })
}
