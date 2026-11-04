//! System audio on Linux: records the monitor source of the default output through the
//! PulseAudio simple API (works with PulseAudio and PipeWire's pulse server).

use capture_core::{AudioBuffer, AudioSink, AudioStreamOptions, CaptureError, CaptureErrorCode, CaptureResult, StreamEvent, StreamHandle};
use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DEFAULT_MONITOR: &str = "@DEFAULT_MONITOR@";

fn open(device: Option<&str>, rate: u32, channels: u8, fragsize: u32) -> Result<Simple, String> {
    let spec = Spec { format: Format::FLOAT32NE, channels, rate };
    if !spec.is_valid() {
        return Err(format!("invalid sample spec {rate} Hz x {channels}"));
    }
    let attr = BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize };
    let dev = device.filter(|d| !d.is_empty() && *d != "system-default").unwrap_or(DEFAULT_MONITOR);
    Simple::new(None, "CaptureOne", Direction::Record, Some(dev), "System audio capture", &spec, None, Some(&attr)).map_err(|e| format!("{e}"))
}

/// Checks that a pulse server is reachable and the default monitor can be opened.
pub fn probe() -> Result<(), String> {
    open(None, 48_000, 2, 48_000 / 50 * 8).map(|_| ())
}

pub struct PulseStream {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StreamHandle for PulseStream {
    fn stop(&mut self) -> CaptureResult<()> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        Ok(())
    }
}

impl Drop for PulseStream {
    fn drop(&mut self) {
        let _ = StreamHandle::stop(self);
    }
}

pub fn start(opts: &AudioStreamOptions, sink: Arc<dyn AudioSink>) -> CaptureResult<PulseStream> {
    let rate = opts.sample_rate.clamp(8_000, 192_000);
    let channels = opts.channels.clamp(1, 2) as u8;
    let frames = (rate / 50) as usize; // 20 ms reads
    let fragsize = (frames * channels as usize * 4) as u32;
    let device = opts.device_id.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    let thread = std::thread::Builder::new()
        .name("pulse-monitor".into())
        .spawn(move || {
            let s = match open(device.as_deref(), rate, channels, fragsize) {
                Ok(s) => {
                    let _ = ready_tx.send(Ok(()));
                    s
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let mut buf = vec![0u8; fragsize as usize];
            let dur = Duration::from_secs_f64(frames as f64 / rate as f64);
            while !stop2.load(Ordering::SeqCst) {
                if let Err(e) = s.read(&mut buf) {
                    sink.on_event(StreamEvent::Error(CaptureError::new(CaptureErrorCode::DeviceBusy, format!("PulseAudio read failed: {}", format_args!("{e}")))));
                    return;
                }
                let now = Instant::now();
                let samples: Vec<f32> = buf.chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect();
                sink.on_audio(AudioBuffer { sample_rate: rate, channels: channels as u16, samples, captured_at: now.checked_sub(dur).unwrap_or(now) });
            }
        })
        .map_err(|e| CaptureError::internal(format!("cannot spawn audio thread: {e}")))?;
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(PulseStream { stop, thread: Some(thread) }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(CaptureError::new(CaptureErrorCode::Unsupported, format!("cannot open the PulseAudio/PipeWire monitor source: {e}")))
        }
        Err(_) => {
            stop.store(true, Ordering::SeqCst);
            Err(CaptureError::new(CaptureErrorCode::DeviceBusy, "PulseAudio did not respond within 10 s"))
        }
    }
}
