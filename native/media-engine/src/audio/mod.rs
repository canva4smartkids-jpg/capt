//! Audio capture (microphone via cpal; system audio comes from the backend), placement on
//! the active clock, resampling and feeding the AAC encoder.

pub mod resample;
pub mod timeline;

use crate::clock::ActiveClock;
use crate::AudioDeviceInfo;
use capture_core::{AudioBuffer, AudioSink, CaptureError, CaptureErrorCode, CaptureResult, StreamEvent, StreamHandle};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use resample::Resampler;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use timeline::AudioTimeline;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSourceKind {
    Microphone,
    System,
}

impl AudioSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            AudioSourceKind::Microphone => "Microphone",
            AudioSourceKind::System => "System audio",
        }
    }
    pub fn file_name(self) -> &'static str {
        match self {
            AudioSourceKind::Microphone => "audio-mic.m4a",
            AudioSourceKind::System => "audio-system.m4a",
        }
    }
}

pub enum AudioMsg {
    Buffer(AudioBuffer),
    Event(StreamEvent),
    /// Pad to the final active duration and stop.
    Finish(f64),
}

/// AudioSink that forwards buffers to a worker thread without blocking the capture thread.
pub struct ChannelAudioSink {
    tx: Sender<AudioMsg>,
    pub dropped_buffers: AtomicU64,
}

impl ChannelAudioSink {
    pub fn new(tx: Sender<AudioMsg>) -> Self {
        Self { tx, dropped_buffers: AtomicU64::new(0) }
    }
}

impl AudioSink for ChannelAudioSink {
    fn on_audio(&self, buffer: AudioBuffer) {
        if self.tx.try_send(AudioMsg::Buffer(buffer)).is_err() {
            self.dropped_buffers.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn on_event(&self, event: StreamEvent) {
        let _ = self.tx.try_send(AudioMsg::Event(event));
    }
}

#[derive(Debug, Default)]
pub struct AudioWorkerResult {
    pub written_frames: u64,
    pub silence_inserted_frames: u64,
    pub dropped_frames: u64,
    pub buffers_received: u64,
    pub write_error: Option<String>,
}

fn write_pcm<W: Write>(out: &mut W, pcm: &[f32], scratch: &mut Vec<u8>) -> std::io::Result<()> {
    if pcm.is_empty() {
        return Ok(());
    }
    scratch.clear();
    scratch.reserve(pcm.len() * 4);
    for s in pcm {
        scratch.extend_from_slice(&s.to_le_bytes());
    }
    out.write_all(scratch)
}

/// Worker thread body: places buffers on the active clock (see `timeline`), resamples to
/// `out_rate` stereo, applies `gain` and writes f32le PCM to `out` (the AAC encoder's stdin).
pub fn run_audio_worker<W: Write>(
    rx: Receiver<AudioMsg>,
    mut out: W,
    clock: Arc<Mutex<ActiveClock>>,
    out_rate: u32,
    gain: f32,
    on_warning: impl Fn(String),
) -> AudioWorkerResult {
    let mut timeline = AudioTimeline::new(out_rate, 2);
    let mut resampler = Resampler::new(out_rate);
    let mut scratch = Vec::new();
    let mut res = AudioWorkerResult::default();
    let mut warned = false;
    let mut failed = false;
    let mut emit = |pcm: Vec<f32>, out: &mut W, res: &mut AudioWorkerResult, failed: &mut bool| {
        if *failed || pcm.is_empty() {
            return;
        }
        if let Err(e) = write_pcm(out, &pcm, &mut scratch) {
            *failed = true;
            res.write_error = Some(e.to_string());
        }
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(AudioMsg::Buffer(b)) => {
                res.buffers_received += 1;
                let pcm = resampler.process(&b.samples, b.channels, b.sample_rate, gain);
                let (start, paused) = {
                    let c = clock.lock();
                    (c.active_at(b.captured_at), c.is_paused())
                };
                if let (Some(start), false) = (start, paused) {
                    let placed = timeline.push(&pcm, start);
                    emit(placed, &mut out, &mut res, &mut failed);
                }
            }
            Ok(AudioMsg::Event(ev)) => {
                if !warned {
                    warned = true;
                    let msg = match ev {
                        StreamEvent::SourceClosed => "audio source closed; continuing with silence".to_string(),
                        StreamEvent::Error(e) => format!("audio source error ({}); continuing with silence", e.message),
                        StreamEvent::Resized { .. } => String::new(),
                    };
                    if !msg.is_empty() {
                        on_warning(msg);
                    } else {
                        warned = false;
                    }
                }
            }
            Ok(AudioMsg::Finish(total)) => {
                let tail = timeline.finish(total);
                emit(tail, &mut out, &mut res, &mut failed);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let (now, paused, started) = {
            let c = clock.lock();
            (c.now(), c.is_paused(), c.started())
        };
        if started && !paused {
            let fill = timeline.fill_stall(now);
            emit(fill, &mut out, &mut res, &mut failed);
        }
    }
    let _ = out.flush();
    res.written_frames = timeline.written_frames();
    res.silence_inserted_frames = timeline.silence_inserted_frames;
    res.dropped_frames = timeline.dropped_frames;
    res
}

fn map_cpal_err(context: &str, e: cpal::Error) -> CaptureError {
    let code = match e.kind() {
        cpal::ErrorKind::PermissionDenied => CaptureErrorCode::PermissionDenied,
        cpal::ErrorKind::DeviceBusy => CaptureErrorCode::DeviceBusy,
        cpal::ErrorKind::DeviceNotAvailable => CaptureErrorCode::SourceNotFound,
        cpal::ErrorKind::UnsupportedConfig | cpal::ErrorKind::UnsupportedOperation => CaptureErrorCode::Unsupported,
        _ => CaptureErrorCode::Internal,
    };
    CaptureError::new(code, format!("{context}: {e}"))
}

fn device_id_string(d: &cpal::Device) -> String {
    d.id().map(|i| i.to_string()).unwrap_or_else(|_| d.to_string())
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| d.to_string())
}

/// Input devices visible to cpal (ALSA on Linux, WASAPI on Windows, CoreAudio on macOS).
pub fn list_input_devices() -> CaptureResult<Vec<AudioDeviceInfo>> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().map(|d| device_id_string(&d));
    let devices = host.input_devices().map_err(|e| map_cpal_err("cannot enumerate audio input devices", e))?;
    let mut out: Vec<AudioDeviceInfo> = devices
        .map(|d| {
            let id = device_id_string(&d);
            AudioDeviceInfo { is_default: Some(&id) == default_id.as_ref(), name: device_name(&d), id, kind: "input".into() }
        })
        .collect();
    if let (Some(def), false) = (&default_id, out.iter().any(|d| d.is_default)) {
        if let Some(d) = host.default_input_device() {
            out.insert(0, AudioDeviceInfo { id: def.clone(), name: device_name(&d), kind: "input".into(), is_default: true });
        }
    }
    out.sort_by_key(|d| !d.is_default);
    Ok(out)
}

pub fn has_input_device() -> bool {
    cpal::default_host().default_input_device().is_some()
}

fn find_input_device(host: &cpal::Host, id: Option<&str>) -> CaptureResult<cpal::Device> {
    let Some(id) = id else {
        return host.default_input_device().ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "no default microphone / input device found"));
    };
    if let Ok(did) = id.parse::<cpal::DeviceId>() {
        if let Some(d) = host.device_by_id(&did) {
            return Ok(d);
        }
    }
    host.input_devices()
        .map_err(|e| map_cpal_err("cannot enumerate audio input devices", e))?
        .find(|d| device_id_string(d) == id || device_name(d) == id || d.to_string() == id)
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("microphone '{id}' not found")))
}

fn data_callback<T>(sink: Arc<dyn AudioSink>, rate: u32, channels: u16) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    move |data: &[T], _info: &cpal::InputCallbackInfo| {
        let samples: Vec<f32> = data.iter().map(|s| <f32 as cpal::FromSample<T>>::from_sample_(*s)).collect();
        let frames = samples.len() / channels.max(1) as usize;
        let dur = Duration::from_secs_f64(frames as f64 / rate.max(1) as f64);
        let now = Instant::now();
        sink.on_audio(AudioBuffer { sample_rate: rate, channels, samples, captured_at: now.checked_sub(dur).unwrap_or(now) });
    }
}

fn build_mic_stream(id: Option<&str>, sink: Arc<dyn AudioSink>) -> CaptureResult<cpal::Stream> {
    let host = cpal::default_host();
    let dev = find_input_device(&host, id)?;
    let supported = dev.default_input_config().map_err(|e| map_cpal_err("microphone configuration", e))?;
    let config = supported.config();
    let (rate, channels) = (config.sample_rate, config.channels);
    let err_sink = sink.clone();
    let on_err = move |e: cpal::Error| {
        err_sink.on_event(StreamEvent::Error(map_cpal_err("microphone stream error", e)));
    };
    use cpal::SampleFormat as F;
    let stream = match supported.sample_format() {
        F::F32 => dev.build_input_stream::<f32, _, _>(config, data_callback::<f32>(sink, rate, channels), on_err, None),
        F::I16 => dev.build_input_stream::<i16, _, _>(config, data_callback::<i16>(sink, rate, channels), on_err, None),
        F::I32 => dev.build_input_stream::<i32, _, _>(config, data_callback::<i32>(sink, rate, channels), on_err, None),
        F::U16 => dev.build_input_stream::<u16, _, _>(config, data_callback::<u16>(sink, rate, channels), on_err, None),
        F::U8 => dev.build_input_stream::<u8, _, _>(config, data_callback::<u8>(sink, rate, channels), on_err, None),
        F::I8 => dev.build_input_stream::<i8, _, _>(config, data_callback::<i8>(sink, rate, channels), on_err, None),
        F::F64 => dev.build_input_stream::<f64, _, _>(config, data_callback::<f64>(sink, rate, channels), on_err, None),
        F::U32 => dev.build_input_stream::<u32, _, _>(config, data_callback::<u32>(sink, rate, channels), on_err, None),
        other => return Err(CaptureError::unsupported(format!("microphone sample format {other:?} is not supported"))),
    }
    .map_err(|e| map_cpal_err("cannot open microphone", e))?;
    stream.play().map_err(|e| map_cpal_err("cannot start microphone", e))?;
    Ok(stream)
}

/// A running microphone capture. cpal streams are not `Send` on every platform, so the
/// stream lives on its own thread until stopped.
pub struct MicCapture {
    stop_tx: Option<Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StreamHandle for MicCapture {
    fn stop(&mut self) -> CaptureResult<()> {
        self.stop_tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        Ok(())
    }
}

impl Drop for MicCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub fn start_microphone(device_id: Option<&str>, sink: Arc<dyn AudioSink>) -> CaptureResult<MicCapture> {
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<CaptureResult<()>>(1);
    let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
    let id = device_id.map(str::to_string);
    let thread = std::thread::Builder::new()
        .name("mic-capture".into())
        .spawn(move || match build_mic_stream(id.as_deref(), sink) {
            Ok(stream) => {
                let _ = ready_tx.send(Ok(()));
                let _ = stop_rx.recv(); // returns when the sender is dropped
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        })
        .map_err(CaptureError::io)?;
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(MicCapture { stop_tx: Some(stop_tx), thread: Some(thread) }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            drop(stop_tx);
            Err(CaptureError::new(CaptureErrorCode::DeviceBusy, "microphone did not start within 10 s"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_produces_exact_duration_with_pause_and_late_start() {
        let clock = Arc::new(Mutex::new(ActiveClock::new()));
        let (tx, rx) = crossbeam_channel::unbounded();
        let t0 = Instant::now();
        clock.lock().start(t0);
        // 44.1 kHz mono device, first buffer 100 ms after start, 10 ms buffers for 1 s
        for i in 0..100u32 {
            let at = t0 + Duration::from_millis(100 + i as u64 * 10);
            tx.send(AudioMsg::Buffer(AudioBuffer { sample_rate: 44_100, channels: 1, samples: vec![0.25; 441], captured_at: at })).unwrap();
        }
        tx.send(AudioMsg::Finish(1.5)).unwrap();
        let mut out = Vec::new();
        let res = run_audio_worker(rx, &mut out, clock, 48_000, 1.0, |_| {});
        assert_eq!(res.written_frames, 72_000, "padded to exactly 1.5 s");
        assert_eq!(out.len(), 72_000 * 2 * 4);
        let first: Vec<f32> = out.chunks_exact(4).take(10).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        assert!(first.iter().all(|&s| s == 0.0), "leading silence");
        assert!(res.silence_inserted_frames >= 4_800);
    }

    #[test]
    fn worker_drops_buffers_while_paused_or_not_started() {
        let clock = Arc::new(Mutex::new(ActiveClock::new()));
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(AudioMsg::Buffer(AudioBuffer { sample_rate: 48_000, channels: 2, samples: vec![0.5; 960], captured_at: Instant::now() })).unwrap();
        let c2 = clock.clone();
        let h = std::thread::spawn(move || {
            let mut out = Vec::new();
            run_audio_worker(rx, &mut out, c2, 48_000, 1.0, |_| {})
        });
        std::thread::sleep(Duration::from_millis(100));
        let t0 = Instant::now();
        {
            let mut c = clock.lock();
            c.start(t0);
            c.pause(t0);
        }
        tx.send(AudioMsg::Buffer(AudioBuffer { sample_rate: 48_000, channels: 2, samples: vec![0.5; 960], captured_at: Instant::now() })).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        tx.send(AudioMsg::Finish(0.0)).unwrap();
        let res = h.join().unwrap();
        assert_eq!(res.buffers_received, 2);
        assert_eq!(res.written_frames, 0);
    }
}
