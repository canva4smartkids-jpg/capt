//! System audio via WASAPI loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`) on a render endpoint.
//!
//! A dedicated MTA thread owns the `IAudioClient`; every ~10 ms it drains all available packets,
//! converts them to interleaved f32 at the endpoint mix format and hands them to the sink with
//! the packet's QPC capture time. When nothing is playing WASAPI delivers no packets at all;
//! the engine's audio timeline fills that gap with silence based on `captured_at`.

use super::clock::instant_from_qpc_hns;
use super::os::wide;
use crate::pcm::{self, RawWaveFormat, StreamFormat};
use crate::timing::frames_duration;
use capture_core::{AudioBuffer, AudioSink, AudioStreamOptions, CaptureError, CaptureErrorCode, CaptureResult, StreamEvent, StreamHandle};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

/// HRESULT_FROM_WIN32(ERROR_NOT_FOUND): no endpoint of the requested kind.
const E_NOTFOUND: HRESULT = HRESULT(0x8007_0490_u32 as i32);
/// WASAPI shared-mode buffer: 200 ms (100 ns units). We poll every 10 ms.
const BUFFER_HNS: i64 = 2_000_000;
const POLL: Duration = Duration::from_millis(10);
/// How long to wait for a new default output device after the current one disappears.
const REOPEN_WINDOW: Duration = Duration::from_secs(3);

/// A render endpoint usable for loopback capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioEndpoint {
    /// `IMMDevice::GetId` string; pass as `AudioStreamOptions::device_id`.
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// MTA COM for the current thread, balanced on drop.
struct ComGuard(bool);

impl ComGuard {
    fn mta() -> Self {
        // SAFETY: balanced by Drop when it succeeded (S_OK or S_FALSE).
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        ComGuard(hr.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: matches the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Runs `f` on a fresh MTA thread (never alters the caller's COM apartment).
fn on_mta_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CaptureResult<T> {
    std::thread::Builder::new()
        .name("captureone-wasapi-query".into())
        .spawn(move || {
            let _com = ComGuard::mta();
            f()
        })
        .map_err(|e| CaptureError::internal(format!("cannot spawn audio query thread: {e}")))?
        .join()
        .map_err(|_| CaptureError::internal("audio query thread panicked"))
}

fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    // SAFETY: standard COM activation on an initialized thread.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn device_id(d: &IMMDevice) -> Option<String> {
    // SAFETY: GetId returns a CoTaskMem string that we free.
    unsafe {
        let p = d.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        s
    }
}

fn friendly_name(d: &IMMDevice) -> Option<String> {
    // SAFETY: property store access; the PROPVARIANT and the allocated string are released.
    unsafe {
        let store = d.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let name = PropVariantToStringAlloc(&value).ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
            s
        });
        let _ = PropVariantClear(&mut value);
        name.filter(|n| !n.is_empty())
    }
}

/// Active render endpoints (loopback sources).
pub fn list_render_endpoints() -> CaptureResult<Vec<AudioEndpoint>> {
    on_mta_thread(|| -> CaptureResult<Vec<AudioEndpoint>> {
        let en = enumerator().map_err(|e| CaptureError::internal(format!("MMDeviceEnumerator unavailable: {e}")))?;
        // SAFETY: COM calls on an MTA thread.
        unsafe {
            let default_id = en.GetDefaultAudioEndpoint(eRender, eConsole).ok().as_ref().and_then(device_id);
            let collection = en
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(|e| CaptureError::internal(format!("EnumAudioEndpoints failed: {e}")))?;
            let count = collection.GetCount().unwrap_or(0);
            let mut out = Vec::new();
            for i in 0..count {
                let Ok(dev) = collection.Item(i) else { continue };
                let Some(id) = device_id(&dev) else { continue };
                let name = friendly_name(&dev).unwrap_or_else(|| "Audio output".into());
                out.push(AudioEndpoint {
                    is_default: default_id.as_deref() == Some(id.as_str()),
                    id,
                    name,
                });
            }
            Ok(out)
        }
    })?
}

/// Friendly name of the default render endpoint, or why there is none (capability report).
pub(crate) fn default_render_name() -> Result<String, String> {
    on_mta_thread(|| -> Result<String, String> {
        let en = enumerator().map_err(|e| format!("MMDeviceEnumerator unavailable: {e}"))?;
        // SAFETY: COM calls on an MTA thread.
        match unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole) } {
            Ok(dev) => Ok(friendly_name(&dev).unwrap_or_else(|| "default output device".into())),
            Err(e) if e.code() == E_NOTFOUND => Err("no active audio output device".into()),
            Err(e) => Err(format!("default output device unavailable: {e}")),
        }
    })
    .map_err(|e| e.message)?
}

fn is_device_lost(e: &windows::core::Error) -> bool {
    let c = e.code();
    c == AUDCLNT_E_DEVICE_INVALIDATED || c == AUDCLNT_E_SERVICE_NOT_RUNNING || c == E_NOTFOUND
}

/// Reads the mix format fields (handles the packed, possibly extensible header).
///
/// # Safety
/// `mix` must point to a valid `WAVEFORMATEX` (+ `cbSize` extra bytes) from `GetMixFormat`.
unsafe fn read_raw_format(mix: *const WAVEFORMATEX) -> RawWaveFormat {
    // SAFETY: caller guarantees validity; the structs are packed so read unaligned copies.
    let wf = unsafe { std::ptr::read_unaligned(mix) };
    let (tag, cb) = (wf.wFormatTag, wf.cbSize);
    let (valid, sub) = if tag == pcm::WAVE_FORMAT_EXTENSIBLE && cb >= 22 {
        // SAFETY: cbSize >= 22 means the WAVEFORMATEXTENSIBLE tail is present.
        let ext = unsafe { std::ptr::read_unaligned(mix as *const WAVEFORMATEXTENSIBLE) };
        let samples = ext.Samples;
        let g = ext.SubFormat;
        // SAFETY: every union variant is a u16.
        (Some(unsafe { samples.wValidBitsPerSample }), Some((g.data1, g.data2, g.data3, g.data4)))
    } else {
        (None, None)
    };
    RawWaveFormat {
        format_tag: tag,
        channels: wf.nChannels,
        samples_per_sec: wf.nSamplesPerSec,
        block_align: wf.nBlockAlign,
        bits_per_sample: wf.wBitsPerSample,
        valid_bits_per_sample: valid,
        sub_format: sub,
    }
}

struct LoopbackSession {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    format: StreamFormat,
    endpoint: String,
}

impl LoopbackSession {
    fn open(device_id: Option<&str>) -> CaptureResult<Self> {
        let en = enumerator().map_err(|e| CaptureError::internal(format!("MMDeviceEnumerator unavailable: {e}")))?;
        // SAFETY: COM calls on the loopback (MTA) thread; the mix format is freed below.
        unsafe {
            let device = match device_id.filter(|id| !id.is_empty() && !id.eq_ignore_ascii_case("default")) {
                Some(id) => {
                    let w = wide(id);
                    en.GetDevice(PCWSTR(w.as_ptr()))
                        .map_err(|e| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("audio output device '{id}' not found: {e}")))?
                }
                None => en.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| {
                    if e.code() == E_NOTFOUND {
                        CaptureError::new(
                            CaptureErrorCode::SourceNotFound,
                            "no active audio output device for system audio (WASAPI loopback)",
                        )
                    } else {
                        CaptureError::internal(format!("default audio output device unavailable: {e}"))
                    }
                })?,
            };
            let endpoint = friendly_name(&device).unwrap_or_else(|| "audio output".into());
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| CaptureError::new(CaptureErrorCode::DeviceBusy, format!("cannot activate {endpoint}: {e}")))?;
            let mix = client.GetMixFormat().map_err(|e| CaptureError::internal(format!("GetMixFormat failed: {e}")))?;
            if mix.is_null() {
                return Err(CaptureError::internal("GetMixFormat returned NULL"));
            }
            let format = pcm::classify(&read_raw_format(mix));
            let init = match &format {
                Ok(_) => client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER_HNS, 0, mix, None),
                Err(_) => Ok(()),
            };
            CoTaskMemFree(Some(mix as *const core::ffi::c_void));
            let format = format.map_err(|e| CaptureError::unsupported(format!("{endpoint}: {e}")))?;
            init.map_err(|e| CaptureError::new(CaptureErrorCode::DeviceBusy, format!("WASAPI loopback Initialize failed on {endpoint}: {e}")))?;
            let capture: IAudioCaptureClient = client
                .GetService()
                .map_err(|e| CaptureError::internal(format!("IAudioCaptureClient unavailable: {e}")))?;
            client
                .Start()
                .map_err(|e| CaptureError::new(CaptureErrorCode::DeviceBusy, format!("WASAPI loopback Start failed: {e}")))?;
            log::info!(
                "WASAPI loopback on '{endpoint}': {} Hz, {} ch, {:?}",
                format.sample_rate,
                format.channels,
                format.encoding
            );
            Ok(Self {
                client,
                capture,
                format,
                endpoint,
            })
        }
    }

    /// Delivers every packet currently queued.
    fn drain(&mut self, sink: &dyn AudioSink) -> windows::core::Result<()> {
        let fmt = self.format;
        loop {
            // SAFETY: COM calls on the owning thread; the buffer is released before the next
            // GetBuffer and only read for `frames * block_align` bytes.
            unsafe {
                if self.capture.GetNextPacketSize()? == 0 {
                    return Ok(());
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let (mut frames, mut flags, mut qpc) = (0u32, 0u32, 0u64);
                self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))?;
                let now = Instant::now();
                let n = frames as usize;
                let mut samples = Vec::with_capacity(n * fmt.channels as usize);
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                    pcm::push_silence(n, fmt.channels, &mut samples);
                } else {
                    let bytes = std::slice::from_raw_parts(data as *const u8, n * fmt.block_align);
                    pcm::convert_to_f32(bytes, n, &fmt, &mut samples);
                }
                self.capture.ReleaseBuffer(frames)?;
                if n == 0 {
                    continue;
                }
                let captured_at = if flags & (AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32) != 0 || qpc == 0 {
                    now.checked_sub(frames_duration(n, fmt.sample_rate)).unwrap_or(now)
                } else {
                    instant_from_qpc_hns(qpc as i64)
                };
                sink.on_audio(AudioBuffer {
                    sample_rate: fmt.sample_rate,
                    channels: fmt.channels,
                    samples,
                    captured_at,
                });
            }
        }
    }
}

impl Drop for LoopbackSession {
    fn drop(&mut self) {
        // SAFETY: stopping an initialized client; COM releases the interfaces afterwards.
        let _ = unsafe { self.client.Stop() };
    }
}

/// Running loopback capture.
pub(crate) struct LoopbackStream {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl StreamHandle for LoopbackStream {
    fn stop(&mut self) -> CaptureResult<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.join().map_err(|_| CaptureError::internal("WASAPI loopback thread panicked"))?;
        }
        Ok(())
    }
}

impl Drop for LoopbackStream {
    fn drop(&mut self) {
        if let Err(e) = self.stop() {
            log::warn!("WASAPI loopback cleanup failed: {e}");
        }
    }
}

pub(crate) fn start_loopback(opts: &AudioStreamOptions, sink: Arc<dyn AudioSink>) -> CaptureResult<LoopbackStream> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::sync_channel::<CaptureResult<String>>(1);
    let device_id = opts.device_id.clone();
    let thread = std::thread::Builder::new()
        .name("captureone-wasapi-loopback".into())
        .spawn({
            let stop = stop.clone();
            move || {
                let sink_for_panic = sink.clone();
                let result = catch_unwind(AssertUnwindSafe(|| loopback_thread(device_id, sink, stop, ready_tx)));
                if result.is_err() {
                    let _ = catch_unwind(AssertUnwindSafe(|| {
                        sink_for_panic.on_event(StreamEvent::Error(CaptureError::internal("WASAPI loopback thread panicked")))
                    }));
                }
            }
        })
        .map_err(|e| CaptureError::internal(format!("cannot spawn WASAPI loopback thread: {e}")))?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(endpoint)) => {
            log::debug!("system audio capture started on {endpoint}");
            Ok(LoopbackStream { stop, thread: Some(thread) })
        }
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            // Leave the thread to exit on its own once WASAPI returns.
            stop.store(true, Ordering::Release);
            Err(CaptureError::new(CaptureErrorCode::DeviceBusy, "WASAPI loopback did not start within 5 s"))
        }
    }
}

fn loopback_thread(device_id: Option<String>, sink: Arc<dyn AudioSink>, stop: Arc<AtomicBool>, ready: mpsc::SyncSender<CaptureResult<String>>) {
    let _com = ComGuard::mta();
    let mut session = match LoopbackSession::open(device_id.as_deref()) {
        Ok(s) => {
            let _ = ready.send(Ok(s.endpoint.clone()));
            s
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let follow_default = device_id.as_deref().is_none_or(|id| id.is_empty() || id.eq_ignore_ascii_case("default"));
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(POLL);
        let err = match session.drain(&*sink) {
            Ok(()) => continue,
            Err(e) => e,
        };
        if is_device_lost(&err) && follow_default {
            log::warn!("system audio device '{}' went away ({err}); following the new default output", session.endpoint);
            drop(session);
            match reopen_default(&stop) {
                Some(s) => {
                    session = s;
                    continue;
                }
                None => {
                    if !stop.load(Ordering::Acquire) {
                        sink.on_event(StreamEvent::SourceClosed);
                    }
                    return;
                }
            }
        }
        if is_device_lost(&err) {
            sink.on_event(StreamEvent::SourceClosed);
        } else {
            sink.on_event(StreamEvent::Error(CaptureError::internal(format!("WASAPI loopback failed: {err}"))));
        }
        return;
    }
}

fn reopen_default(stop: &AtomicBool) -> Option<LoopbackSession> {
    let deadline = Instant::now() + REOPEN_WINDOW;
    while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
        match LoopbackSession::open(None) {
            Ok(s) => return Some(s),
            Err(e) => log::debug!("waiting for a default output device: {e}"),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    None
}
