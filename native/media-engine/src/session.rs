//! Recording sessions: capture -> pacer -> ffmpeg (video), capture -> timeline -> ffmpeg
//! (audio per source), limits/interruption monitoring, and finalization.

use crate::args::{audio_encode_args, expected_audio_streams, video_encode_args, AudioEncodeSpec, MuxAudioInput, MuxSpec, VideoEncodeSpec};
use crate::audio::{self, AudioMsg, AudioSourceKind, AudioWorkerResult, ChannelAudioSink};
use crate::clock::ActiveClock;
use crate::encoders::{is_hevc, select_encoder};
use crate::finalize::{mux_to_final, platform_string, video_metadata, VideoMetaInput};
use crate::manifest::*;
use crate::naming::candidate_names;
use crate::pacer::{run_pacer, run_writer, FramePrep, FrameQueue, LatestFrame, PacerControl, PacerCore};
use crate::probe::{default_tolerance, Expectation};
use crate::screenshot::{apply_region, resolve_target};
use crate::tools::{classify_ffmpeg_error, new_command, wait_child, StderrTail};
use crate::{EngineInner, RecorderListener};
use capture_core::presets::{resolve_output_size, video_bitrate_kbps};
use capture_core::state::StateMachine;
use capture_core::*;
use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use fs2::FileExt;
use parking_lot::{Condvar, Mutex};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;
/// File-size limits use decimal megabytes (stricter than MiB).
const MB: u64 = 1_000_000;
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const QUEUE_CAPACITY: usize = 4;

enum Cmd {
    Stop { reason: StopReason, error: Option<CaptureError> },
    Warning(String),
}

#[derive(Default)]
struct Live {
    bytes_written: u64,
    free_disk: Option<u64>,
    estimated: Option<u64>,
    warnings: Vec<String>,
    error: Option<CaptureError>,
    output_path: Option<String>,
    encoder: Option<String>,
}

pub(crate) struct Shared {
    id: String,
    listener: Arc<dyn RecorderListener>,
    sm: Mutex<StateMachine>,
    clock: Arc<Mutex<ActiveClock>>,
    cmd_tx: Sender<Cmd>,
    stop_sent: AtomicBool,
    cancel: AtomicBool,
    frames_encoded: Arc<AtomicU64>,
    frames_dropped: Arc<AtomicU64>,
    live: Mutex<Live>,
    done: Mutex<Option<CaptureResult<MediaMetadata>>>,
    done_cv: Condvar,
    max_duration: Option<f64>,
}

impl Shared {
    fn transition(&self, to: RecordingState) -> CaptureResult<()> {
        let from = self.sm.lock().transition(to)?;
        self.listener.on_state(&self.id, from, to);
        Ok(())
    }

    fn warn(&self, msg: &str) {
        log::warn!("[{}] {msg}", self.id);
        self.live.lock().warnings.push(msg.to_string());
        self.listener.on_warning(&self.id, msg);
    }

    fn request_stop(&self, reason: StopReason, error: Option<CaptureError>) {
        if !self.stop_sent.swap(true, Ordering::SeqCst) {
            let _ = self.cmd_tx.send(Cmd::Stop { reason, error });
        }
    }

    fn status(&self) -> RecordingStatus {
        let state = self.sm.lock().state();
        let elapsed = self.clock.lock().now();
        let live = self.live.lock();
        RecordingStatus {
            session_id: self.id.clone(),
            state,
            elapsed_ms: (elapsed * 1000.0) as u64,
            countdown_remaining_sec: None,
            bytes_written: live.bytes_written,
            frames_encoded: self.frames_encoded.load(Ordering::Relaxed),
            frames_dropped: self.frames_dropped.load(Ordering::Relaxed),
            estimated_bytes_per_minute: live.estimated,
            free_disk_bytes: live.free_disk,
            output_path: live.output_path.clone(),
            encoder: live.encoder.clone(),
            warnings: live.warnings.clone(),
            error: live.error.clone(),
        }
    }

    fn wait_done(&self) -> CaptureResult<MediaMetadata> {
        let mut g = self.done.lock();
        loop {
            if let Some(r) = g.as_ref() {
                return r.clone();
            }
            self.done_cv.wait(&mut g);
        }
    }

    fn finish(&self, reason: StopReason, result: CaptureResult<MediaMetadata>) {
        self.listener.on_status(&self.status());
        self.listener.on_completed(&self.id, reason, &result);
        *self.done.lock() = Some(result);
        self.done_cv.notify_all();
    }
}

/// Cheap clonable handle to a recording.
#[derive(Clone)]
pub struct RecordingSession {
    shared: Arc<Shared>,
}

impl RecordingSession {
    pub fn id(&self) -> &str {
        &self.shared.id
    }

    pub fn pause(&self) -> CaptureResult<()> {
        let mut sm = self.shared.sm.lock();
        if sm.state() != RecordingState::Recording {
            return Err(CaptureError::invalid_state(format!("cannot pause while {:?}", sm.state())));
        }
        let from = sm.transition(RecordingState::Paused)?;
        self.shared.clock.lock().pause(Instant::now());
        drop(sm);
        self.shared.listener.on_state(&self.shared.id, from, RecordingState::Paused);
        Ok(())
    }

    pub fn resume(&self) -> CaptureResult<()> {
        let mut sm = self.shared.sm.lock();
        if sm.state() != RecordingState::Paused {
            return Err(CaptureError::invalid_state(format!("cannot resume while {:?}", sm.state())));
        }
        let from = sm.transition(RecordingState::Recording)?;
        self.shared.clock.lock().resume(Instant::now());
        drop(sm);
        self.shared.listener.on_state(&self.shared.id, from, RecordingState::Recording);
        Ok(())
    }

    /// Blocks until the file is finalized and validated. If the session already ended
    /// (limit reached, source closed), returns that result.
    pub fn stop(&self) -> CaptureResult<MediaMetadata> {
        self.shared.request_stop(StopReason::User, None);
        self.shared.wait_done()
    }

    /// Stops and deletes all output (intermediates included).
    pub fn cancel(&self) -> CaptureResult<()> {
        if self.shared.done.lock().is_some() {
            return Err(CaptureError::invalid_state("recording already finished"));
        }
        self.shared.cancel.store(true, Ordering::SeqCst);
        self.shared.request_stop(StopReason::Cancelled, None);
        match self.shared.wait_done() {
            Err(e) if e.code == CaptureErrorCode::Interrupted => Ok(()),
            Err(e) => Err(e),
            Ok(_) => Err(CaptureError::invalid_state("recording finished before it could be cancelled")),
        }
    }

    pub fn status(&self) -> RecordingStatus {
        self.shared.status()
    }
}

struct SessionVideoSink {
    latest: Arc<LatestFrame>,
    shared: Arc<Shared>,
    ended: AtomicBool,
    end_error: Mutex<Option<CaptureError>>,
}

impl VideoSink for SessionVideoSink {
    fn on_frame(&self, frame: VideoFrame) {
        self.latest.put(frame);
    }
    fn on_event(&self, event: StreamEvent) {
        match event {
            StreamEvent::SourceClosed => {
                *self.end_error.lock() = Some(CaptureError::new(CaptureErrorCode::SourceClosed, "the captured source was closed"));
                self.ended.store(true, Ordering::SeqCst);
                self.shared.request_stop(StopReason::SourceClosed, None);
            }
            StreamEvent::Error(e) => {
                *self.end_error.lock() = Some(e.clone());
                self.ended.store(true, Ordering::SeqCst);
                self.shared.request_stop(StopReason::Error, Some(e));
            }
            StreamEvent::Resized { width, height } => log::info!("[{}] source resized to {width}x{height}; frames are cropped/padded to the original size", self.shared.id),
        }
    }
}

struct AudioRt {
    kind: AudioSourceKind,
    file: PathBuf,
    stream: Option<Box<dyn StreamHandle>>,
    tx: Sender<AudioMsg>,
    worker: Option<JoinHandle<AudioWorkerResult>>,
    child: Child,
    stderr: Option<StderrTail>,
}

#[derive(Default)]
struct Runtime {
    video_stream: Option<Box<dyn StreamHandle>>,
    video_child: Option<Child>,
    video_stderr: Option<StderrTail>,
    video_stdin: Option<ChildStdin>,
    pacer: Option<JoinHandle<u64>>,
    pacer_ctl: Arc<PacerControl>,
    queue: Option<Arc<FrameQueue<VideoFrame>>>,
    writer: Option<JoinHandle<()>>,
    audio: Vec<AudioRt>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.abort();
    }
}

impl Runtime {
    /// Tears everything down (idempotent; a no-op after a normal finalize).
    fn abort(&mut self) {
        if let Some(mut s) = self.video_stream.take() {
            let _ = s.stop();
        }
        for a in &mut self.audio {
            if let Some(mut s) = a.stream.take() {
                let _ = s.stop();
            }
        }
        self.pacer_ctl.finish(0.0);
        if let Some(p) = self.pacer.take() {
            let _ = p.join();
        }
        if let Some(q) = &self.queue {
            q.close();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        self.video_stdin.take();
        if let Some(c) = self.video_child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        for a in &mut self.audio {
            let _ = a.tx.send(AudioMsg::Finish(0.0));
            let _ = a.child.kill();
            if let Some(w) = a.worker.take() {
                let _ = w.join();
            }
            let _ = a.child.wait();
        }
    }
}

struct Plan {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    config: RecordingConfig,
    output_dir: PathBuf,
    session_dir: PathBuf,
    source_name: String,
    created_at: String,
    created_local: DateTime<Local>,
    output_size: Size,
    encoder: String,
    video_kbps: u32,
    audio_kinds: Vec<AudioSourceKind>,
    manifest: SessionManifest,
    lock: Option<File>,
}

fn spawn_thread<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> CaptureResult<JoinHandle<T>> {
    std::thread::Builder::new().name(name.into()).spawn(f).map_err(|e| CaptureError::internal(format!("cannot spawn thread {name}: {e}")))
}

pub(crate) fn start(engine: &Arc<EngineInner>, config: RecordingConfig, listener: Arc<dyn RecorderListener>) -> CaptureResult<RecordingSession> {
    config.validate()?;
    let id = uuid::Uuid::new_v4().to_string();
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    let shared = Arc::new(Shared {
        id: id.clone(),
        listener: listener.clone(),
        sm: Mutex::new(StateMachine::default()),
        clock: Arc::new(Mutex::new(ActiveClock::new())),
        cmd_tx,
        stop_sent: AtomicBool::new(false),
        cancel: AtomicBool::new(false),
        frames_encoded: Arc::new(AtomicU64::new(0)),
        frames_dropped: Arc::new(AtomicU64::new(0)),
        live: Mutex::new(Live::default()),
        done: Mutex::new(None),
        done_cv: Condvar::new(),
        max_duration: config.limits.max_duration_sec.map(|d| d as f64),
    });
    shared.transition(RecordingState::Starting)?;
    let mut rt = Runtime::default();
    let mut created_dir = None;
    let res = prepare(engine, &config, &shared, &mut rt, &mut created_dir).and_then(|plan| {
        let sh = shared.clone();
        spawn_thread("recording-supervisor", move || supervise(sh, rt, plan, cmd_rx))
    });
    match res {
        Ok(_) => Ok(RecordingSession { shared }),
        Err(e) => {
            // `prepare` already tore down what it started; if the supervisor thread could not
            // be spawned, dropping the closure dropped `rt`, whose Drop tears it down.
            if let Some(d) = created_dir {
                let _ = std::fs::remove_dir_all(d);
            }
            let _ = shared.transition(RecordingState::Failed);
            shared.live.lock().error = Some(e.clone());
            listener.on_error(&id, &e);
            Err(e)
        }
    }
}

fn prepare(engine: &Arc<EngineInner>, config: &RecordingConfig, shared: &Arc<Shared>, rt: &mut Runtime, created_dir: &mut Option<PathBuf>) -> CaptureResult<Plan> {
    match prepare_inner(engine, config, shared, rt, created_dir) {
        Ok(p) => Ok(p),
        Err(e) => {
            rt.abort();
            Err(e)
        }
    }
}

fn prepare_inner(engine: &Arc<EngineInner>, config: &RecordingConfig, shared: &Arc<Shared>, rt: &mut Runtime, created_dir: &mut Option<PathBuf>) -> CaptureResult<Plan> {
    let ffmpeg = engine.ffmpeg.clone().ok_or_else(|| {
        CaptureError::new(CaptureErrorCode::EncoderUnavailable, "ffmpeg was not found (bundle it next to the app, add it to PATH or set CAPTUREONE_FFMPEG)")
    })?;
    let ffprobe = engine.ffprobe.clone().ok_or_else(|| {
        CaptureError::new(CaptureErrorCode::EncoderUnavailable, "ffprobe was not found (needed to validate recordings; set CAPTUREONE_FFPROBE)")
    })?;
    let output_dir = PathBuf::from(&config.output.directory);
    std::fs::create_dir_all(&output_dir)?;
    if let Ok(free) = fs2::available_space(&output_dir) {
        if free < config.limits.min_free_disk_mb * MIB {
            return Err(CaptureError::new(
                CaptureErrorCode::StorageFull,
                format!("only {} MiB free in {}, at least {} MiB required", free / MIB, output_dir.display(), config.limits.min_free_disk_mb),
            ));
        }
        shared.live.lock().free_disk = Some(free);
    }
    let choice = select_encoder(config.video.codec, config.video.hardware_acceleration, &engine.encoders())?;
    let backend = engine.backend.clone();
    let target = resolve_target(backend.as_ref(), &config.target)?;

    let session_dir = session_dir(&output_dir, &shared.id);
    std::fs::create_dir_all(&session_dir)?;
    *created_dir = Some(session_dir.clone());
    let lock = File::create(session_dir.join(LOCK_FILE))?;
    lock.try_lock_exclusive().map_err(|e| CaptureError::io(format!("cannot lock session directory: {e}")))?;

    let created_local = Local::now();
    let created_at = now_iso8601();
    let template = config.output.file_name_template.clone();
    let planned_name = candidate_names(&template, &created_local, &target.source.name, "mp4").next().unwrap_or_else(|| "CaptureOne.mp4".into());
    {
        let mut live = shared.live.lock();
        live.output_path = Some(output_dir.join(&planned_name).to_string_lossy().to_string());
        live.encoder = Some(choice.encoder.name.clone());
    }
    let mut manifest = SessionManifest {
        version: 1,
        session_id: shared.id.clone(),
        state: ManifestState::Starting,
        app_version: APP_VERSION.into(),
        platform: platform_string(),
        created_at: created_at.clone(),
        created_local: created_local.to_rfc3339(),
        updated_at: created_at.clone(),
        source_name: target.source.name.clone(),
        file_name_template: template,
        config: config.clone(),
        video: None,
        audio: Vec::new(),
        track_mode: config.audio.track_mode,
        audio_bitrate_kbps: config.audio.bitrate_kbps,
        active_ms: 0,
        final_path: None,
        error: None,
    };
    write_manifest(&session_dir, &manifest)?;

    // Video capture: wait for the first frame to learn the real source size.
    let latest = Arc::new(LatestFrame::default());
    let sink = Arc::new(SessionVideoSink { latest: latest.clone(), shared: shared.clone(), ended: AtomicBool::new(false), end_error: Mutex::new(None) });
    let opts = VideoStreamOptions { fps: config.video.fps, show_cursor: config.video.show_cursor };
    rt.video_stream = Some(backend.start_video(&config.target, &opts, sink.clone())?);
    let first = latest.wait_first(FIRST_FRAME_TIMEOUT, &|| sink.ended.load(Ordering::SeqCst)).ok_or_else(|| {
        sink.end_error
            .lock()
            .clone()
            .unwrap_or_else(|| CaptureError::new(CaptureErrorCode::SourceClosed, format!("no frames received from '{}' within {}s", target.source.name, FIRST_FRAME_TIMEOUT.as_secs())))
    })?;
    let first = apply_region(first, target.region)?;
    let input = Size { width: first.width, height: first.height };
    let output_size = resolve_output_size(input, config.video.resolution);
    let video_kbps = video_bitrate_kbps(output_size, config.video.fps, config.video.codec, config.video.quality, config.video.bitrate_kbps);
    let spec = VideoEncodeSpec {
        input,
        output: output_size,
        fps: config.video.fps,
        encoder: choice.encoder.name.clone(),
        bitrate_kbps: video_kbps,
        keyframe_interval_sec: config.video.keyframe_interval_sec,
        out_path: session_dir.join(VIDEO_FILE),
    };
    let mut cmd = new_command(&ffmpeg);
    cmd.args(video_encode_args(&spec)).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    log::info!("[{}] video: {:?}", shared.id, cmd);
    let mut child = cmd.spawn().map_err(|e| CaptureError::new(CaptureErrorCode::EncoderUnavailable, format!("cannot start ffmpeg: {e}")))?;
    rt.video_stderr = child.stderr.take().map(|e| StderrTail::spawn(e, 40));
    rt.video_stdin = child.stdin.take();
    rt.video_child = Some(child);
    manifest.video = Some(ManifestVideo {
        file: VIDEO_FILE.into(),
        input_width: input.width,
        input_height: input.height,
        width: output_size.width,
        height: output_size.height,
        fps: config.video.fps,
        codec: config.video.codec,
        encoder: choice.encoder.name.clone(),
        bitrate_kbps: video_kbps,
    });

    // Audio sources.
    let mut sources = Vec::new();
    if config.audio.microphone.enabled {
        sources.push((AudioSourceKind::Microphone, config.audio.microphone.clone()));
    }
    if config.audio.system.enabled {
        sources.push((AudioSourceKind::System, config.audio.system.clone()));
    }
    let mut audio_kinds = Vec::new();
    for (kind, settings) in sources {
        let file = session_dir.join(kind.file_name());
        let mut cmd = new_command(&ffmpeg);
        cmd.args(audio_encode_args(&AudioEncodeSpec { sample_rate: config.audio.sample_rate, channels: 2, bitrate_kbps: config.audio.bitrate_kbps, out_path: file.clone() }))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| CaptureError::new(CaptureErrorCode::EncoderUnavailable, format!("cannot start ffmpeg for audio: {e}")))?;
        let stderr = child.stderr.take().map(|e| StderrTail::spawn(e, 20));
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CaptureError::internal("ffmpeg audio stdin unavailable"));
        };
        let (tx, rx) = crossbeam_channel::bounded(2048);
        let sink: Arc<dyn AudioSink> = Arc::new(ChannelAudioSink::new(tx.clone()));
        let (clock, rate, gain, sh) = (shared.clock.clone(), config.audio.sample_rate, settings.gain as f32, shared.clone());
        let worker = match spawn_thread(&format!("audio-{}", kind.label()), move || {
            audio::run_audio_worker(rx, stdin, clock, rate, gain, move |m| {
                let _ = sh.cmd_tx.send(Cmd::Warning(format!("{}: {m}", kind.label())));
            })
        }) {
            Ok(w) => w,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        rt.audio.push(AudioRt { kind, file, stream: None, tx, worker: Some(worker), child, stderr });
        let stream = match kind {
            AudioSourceKind::Microphone => audio::start_microphone(settings.device_id.as_deref(), sink).map(|m| Box::new(m) as Box<dyn StreamHandle>),
            AudioSourceKind::System => backend.start_system_audio(&AudioStreamOptions { sample_rate: rate, channels: 2, device_id: settings.device_id.clone() }, sink),
        }?;
        if let Some(last) = rt.audio.last_mut() {
            last.stream = Some(stream);
        }
        manifest.audio.push(ManifestAudio { file: kind.file_name().into(), source: kind, label: kind.label().into() });
        audio_kinds.push(kind);
    }
    manifest.state = ManifestState::Recording;
    write_manifest(&session_dir, &manifest)?;

    // Go: start the active clock, then the pacer and the writer.
    let queue = Arc::new(FrameQueue::new(QUEUE_CAPACITY));
    rt.queue = Some(queue.clone());
    shared.clock.lock().start(Instant::now());
    shared.transition(RecordingState::Recording)?;
    {
        let core = PacerCore::new(config.video.fps, shared.max_duration);
        let (clock, latest, q, ctl, dropped) = (shared.clock.clone(), latest.clone(), queue.clone(), rt.pacer_ctl.clone(), shared.frames_dropped.clone());
        rt.pacer = Some(spawn_thread("video-pacer", move || run_pacer(core, clock, latest, q, ctl, dropped))?);
    }
    {
        let stdin = rt.video_stdin.take().ok_or_else(|| CaptureError::internal("ffmpeg video stdin unavailable"))?;
        let prep = FramePrep { input, region: target.region };
        let (q, encoded, sh) = (queue, shared.frames_encoded.clone(), shared.clone());
        rt.writer = Some(spawn_thread("video-writer", move || {
            run_writer(q, stdin, prep, encoded, move |e| {
                sh.request_stop(StopReason::Error, Some(CaptureError::new(CaptureErrorCode::EncoderFailed, format!("video encoder stopped accepting frames: {e}"))));
            })
        })?);
    }
    if let Some(w) = &choice.warning {
        shared.warn(w);
    }
    shared.listener.on_status(&shared.status());
    Ok(Plan {
        ffmpeg,
        ffprobe,
        config: config.clone(),
        output_dir,
        session_dir,
        source_name: target.source.name,
        created_at,
        created_local,
        output_size,
        encoder: choice.encoder.name,
        video_kbps,
        audio_kinds,
        manifest,
        lock: Some(lock),
    })
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn refresh_disk(shared: &Shared, plan: &Plan) {
    let mut bytes = file_len(&plan.session_dir.join(VIDEO_FILE));
    for k in &plan.audio_kinds {
        bytes += file_len(&plan.session_dir.join(k.file_name()));
    }
    let free = fs2::available_space(&plan.output_dir).ok();
    let active = shared.clock.lock().now();
    let mut live = shared.live.lock();
    live.bytes_written = bytes;
    if free.is_some() {
        live.free_disk = free;
    }
    if active > 1.0 && bytes > 0 {
        let rate = bytes as f64 / active;
        let horizon = plan.config.limits.max_duration_sec.map_or(60.0, |d| d as f64);
        live.estimated = Some((rate * horizon) as u64);
    }
}

/// Bytes the final file will have reached by now: fragments land on disk at most every
/// `FRAG_SEC` (or keyframe), so add the not-yet-flushed part estimated from the observed
/// (or, before the first fragments, the target) bitrate.
fn estimated_size(bytes_on_disk: u64, active: f64, plan: &Plan) -> u64 {
    // not yet on disk: the open fragment + frames inside the encoder (lookahead / frame
    // threads, ~0.5 s) + one supervisor tick until the stop takes effect
    let lag = crate::args::FRAG_SEC.min(plan.config.video.keyframe_interval_sec) + ENCODER_DELAY_SEC + 0.1;
    let target_rate = (plan.video_kbps as f64 + plan.config.audio.bitrate_kbps as f64 * plan.audio_kinds.len() as f64) * 1000.0 / 8.0;
    if active <= 2.0 * lag || bytes_on_disk == 0 {
        return (target_rate * active * 1.3) as u64;
    }
    let observed = bytes_on_disk as f64 / (active - lag).max(0.25);
    bytes_on_disk + (observed * lag * 1.5) as u64 + bytes_on_disk / 50 + 64 * 1024
}

/// Typical encoder pipeline delay (x264 lookahead / frame threads, hardware queues).
const ENCODER_DELAY_SEC: f64 = 0.6;

fn check_limits(shared: &Shared, plan: &Plan) -> Option<StopReason> {
    let lim = &plan.config.limits;
    let (active, paused) = {
        let c = shared.clock.lock();
        (c.now(), c.is_paused())
    };
    if let Some(max) = lim.max_duration_sec {
        if active >= max as f64 {
            return Some(StopReason::MaxDuration);
        }
    }
    let live = shared.live.lock();
    if let Some(max_mb) = lim.max_file_size_mb {
        if !paused && estimated_size(live.bytes_written, active, plan) >= max_mb * MB {
            return Some(StopReason::MaxFileSize);
        }
    }
    if let Some(free) = live.free_disk {
        if free < low_disk_threshold(lim.min_free_disk_mb, live.bytes_written) {
            return Some(StopReason::LowDisk);
        }
    }
    None
}

/// Finalizing copies the intermediates into the final MP4, so the recording stops while
/// there is still room for that copy *and* the configured minimum afterwards.
fn low_disk_threshold(min_free_mb: u64, bytes_written: u64) -> u64 {
    min_free_mb * MIB + bytes_written + bytes_written / 20 + 8 * MIB
}

fn supervise(shared: Arc<Shared>, mut rt: Runtime, mut plan: Plan, rx: Receiver<Cmd>) {
    let mut last_status = Instant::now();
    let mut last_manifest = Instant::now();
    let mut last_disk = Instant::now();
    let (reason, error) = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Cmd::Stop { reason, error }) => break (reason, error),
            Ok(Cmd::Warning(m)) => shared.warn(&m),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break (StopReason::Interrupted, None),
        }
        if last_disk.elapsed() >= Duration::from_millis(250) {
            refresh_disk(&shared, &plan);
            last_disk = Instant::now();
        }
        if let Some(reason) = check_limits(&shared, &plan) {
            shared.stop_sent.store(true, Ordering::SeqCst);
            if reason == StopReason::LowDisk {
                shared.warn("free disk space is running out (minimum free space plus room to finalize); stopping");
            }
            break (reason, None);
        }
        if last_status.elapsed() >= Duration::from_millis(500) {
            shared.listener.on_status(&shared.status());
            last_status = Instant::now();
        }
        if last_manifest.elapsed() >= Duration::from_secs(2) {
            plan.manifest.active_ms = (shared.clock.lock().now() * 1000.0) as u64;
            let _ = write_manifest(&plan.session_dir, &plan.manifest);
            last_manifest = Instant::now();
        }
    };
    // Drain warnings that raced with the stop.
    while let Ok(cmd) = rx.try_recv() {
        if let Cmd::Warning(m) = cmd {
            shared.warn(&m);
        }
    }
    let result = finalize(&shared, &mut rt, &mut plan, reason, error);
    shared.finish(reason, result);
}

fn fail(shared: &Shared, plan: &mut Plan, e: CaptureError) -> CaptureResult<MediaMetadata> {
    plan.manifest.state = ManifestState::Failed;
    plan.manifest.error = Some(e.message.clone());
    let _ = write_manifest(&plan.session_dir, &plan.manifest);
    shared.live.lock().error = Some(e.clone());
    shared.listener.on_error(&shared.id, &e);
    let _ = shared.transition(RecordingState::Failed);
    Err(e)
}

fn remove_session_dir(plan: &mut Plan) -> std::io::Result<()> {
    if let Some(l) = plan.lock.take() {
        let _ = l.unlock();
        drop(l);
    }
    std::fs::remove_dir_all(&plan.session_dir)
}

fn finalize(shared: &Arc<Shared>, rt: &mut Runtime, plan: &mut Plan, reason: StopReason, error: Option<CaptureError>) -> CaptureResult<MediaMetadata> {
    // Recording/Paused -> Stopping; freeze the clock.
    let total = {
        let mut sm = shared.sm.lock();
        let from = sm.state();
        let moved = sm.transition(RecordingState::Stopping).is_ok();
        let mut c = shared.clock.lock();
        let mut t = c.now();
        c.pause(Instant::now());
        drop(c);
        drop(sm);
        if moved {
            shared.listener.on_state(&shared.id, from, RecordingState::Stopping);
        }
        if let Some(m) = shared.max_duration {
            t = t.min(m);
        }
        t
    };
    if let Some(e) = &error {
        shared.live.lock().error = Some(e.clone());
        shared.listener.on_error(&shared.id, e);
    }
    shared.listener.on_status(&shared.status());

    // Stop capture.
    if let Some(mut s) = rt.video_stream.take() {
        if let Err(e) = s.stop() {
            shared.warn(&format!("stopping video capture failed: {}", e.message));
        }
    }
    for a in &mut rt.audio {
        if let Some(mut s) = a.stream.take() {
            if let Err(e) = s.stop() {
                shared.warn(&format!("stopping {} failed: {}", a.kind.label(), e.message));
            }
        }
    }

    // Drain video: remaining frames for `total`, then EOF to ffmpeg.
    rt.pacer_ctl.finish(total);
    let emitted = rt.pacer.take().and_then(|h| h.join().ok()).unwrap_or(0);
    if let Some(q) = &rt.queue {
        q.close();
    }
    if let Some(w) = rt.writer.take() {
        let _ = w.join();
    }
    rt.video_stdin.take();
    let cancel = shared.cancel.load(Ordering::SeqCst);
    let video_status = match rt.video_child.as_mut() {
        Some(c) => {
            if cancel {
                let _ = c.kill();
            }
            wait_child(c, Duration::from_secs(180)).ok()
        }
        None => None,
    };
    let video_stderr = rt.video_stderr.take().map(|t| t.finish()).unwrap_or_default();
    log::info!(
        "[{}] stop ({reason:?}): {emitted} frames emitted, {} written, {} dropped, active {total:.3}s",
        shared.id,
        shared.frames_encoded.load(Ordering::Relaxed),
        shared.frames_dropped.load(Ordering::Relaxed)
    );

    // Drain audio: pad each track to `total`.
    let mut audio_ok: Vec<(AudioSourceKind, PathBuf)> = Vec::new();
    for a in &mut rt.audio {
        let _ = a.tx.send(AudioMsg::Finish(total));
        let res = a.worker.take().and_then(|h| h.join().ok()).unwrap_or_default();
        if cancel {
            let _ = a.child.kill();
        }
        let st = wait_child(&mut a.child, Duration::from_secs(60)).ok();
        let err = a.stderr.take().map(|t| t.finish()).unwrap_or_default();
        log::info!(
            "[{}] {}: {} buffers, {} frames written, {} silence frames inserted, {} frames dropped",
            shared.id,
            a.kind.label(),
            res.buffers_received,
            res.written_frames,
            res.silence_inserted_frames,
            res.dropped_frames
        );
        let ok = st.map(|s| s.success()).unwrap_or(false) && res.write_error.is_none() && file_len(&a.file) > 0;
        if ok {
            if res.buffers_received == 0 {
                shared.warn(&format!("{} delivered no audio; the track contains silence", a.kind.label()));
            }
            audio_ok.push((a.kind, a.file.clone()));
        } else if !cancel {
            shared.warn(&format!("{} track could not be encoded and was left out: {}", a.kind.label(), res.write_error.clone().unwrap_or(err)));
        }
    }

    if cancel {
        let _ = shared.transition(RecordingState::Failed);
        let _ = remove_session_dir(plan);
        return Err(CaptureError::new(CaptureErrorCode::Interrupted, "recording cancelled"));
    }

    let _ = shared.transition(RecordingState::Finalizing);
    plan.manifest.state = ManifestState::Finalizing;
    plan.manifest.active_ms = (total * 1000.0) as u64;
    let _ = write_manifest(&plan.session_dir, &plan.manifest);
    shared.listener.on_status(&shared.status());

    let video_file = plan.session_dir.join(VIDEO_FILE);
    let video_ok = video_status.map(|s| s.success()).unwrap_or(false);
    if file_len(&video_file) == 0 {
        let code = classify_ffmpeg_error(&video_stderr, CaptureErrorCode::EncoderFailed);
        return fail(shared, plan, CaptureError::new(code, format!("video encoder produced no output: {video_stderr}")));
    }
    if !video_ok {
        let code = classify_ffmpeg_error(&video_stderr, CaptureErrorCode::EncoderFailed);
        let e = CaptureError::new(code, format!("video encoder exited with an error: {video_stderr}"));
        shared.live.lock().error = Some(e.clone());
        shared.listener.on_error(&shared.id, &e);
        shared.warn("finalizing the frames that were encoded before the encoder failed");
    }

    let spec = MuxSpec {
        video: video_file,
        audio: audio_ok.iter().map(|(k, p)| MuxAudioInput { path: p.clone(), label: k.label().into() }).collect(),
        track_mode: plan.config.audio.track_mode,
        audio_bitrate_kbps: plan.config.audio.bitrate_kbps,
        hevc: is_hevc(&plan.encoder),
        title: None,
        out_path: PathBuf::new(),
    };
    let expectation = Expectation {
        duration_sec: Some(total),
        duration_tolerance_sec: default_tolerance(total),
        size: Some(plan.output_size),
        codec: Some(plan.config.video.codec),
        audio_streams: Some(expected_audio_streams(plan.audio_kinds.len(), plan.config.audio.track_mode)),
    };
    let names = candidate_names(&plan.config.output.file_name_template, &plan.created_local, &plan.source_name, "mp4");
    let timeout = Duration::from_secs(600 + total as u64);
    let outcome = match mux_to_final(&plan.ffmpeg, &plan.ffprobe, &spec, &plan.output_dir, &shared.id, names, &expectation, timeout) {
        Ok(o) => o,
        Err(e) => return fail(shared, plan, e),
    };
    let meta = video_metadata(VideoMetaInput {
        id: &shared.id,
        path: &outcome.path,
        size_bytes: outcome.size_bytes,
        probe: &outcome.probe,
        validation: Some(outcome.validation.clone()),
        created_at: plan.created_at.clone(),
        source_name: Some(plan.source_name.clone()),
        platform: Some(platform_string()),
        thumbnail: outcome.thumbnail.clone(),
    });
    shared.live.lock().output_path = Some(meta.path.clone());
    if outcome.validation.valid {
        if let Err(e) = remove_session_dir(plan) {
            shared.warn(&format!("could not remove session data {}: {e}", plan.session_dir.display()));
        }
    } else {
        let issues: Vec<String> = outcome.validation.issues.iter().map(|i| format!("{}: {}", i.code, i.message)).collect();
        plan.manifest.state = ManifestState::FailedValidation;
        plan.manifest.final_path = Some(meta.path.clone());
        let _ = write_manifest(&plan.session_dir, &plan.manifest);
        shared.warn(&format!("recording failed validation ({}); intermediate data kept in {}", issues.join("; "), plan.session_dir.display()));
    }
    let _ = shared.transition(RecordingState::Completed);
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_for(video_kbps: u32, kf: f64) -> Plan {
        let cfg: RecordingConfig = serde_json::from_value(serde_json::json!({
            "target": {"kind": "display", "displayId": "0"},
            "video": {"codec": "h264", "resolution": {"mode": "source"}, "fps": 30, "quality": "medium", "keyframeIntervalSec": kf},
            "audio": {"microphone": {"enabled": false}, "system": {"enabled": false}},
            "output": {"directory": "/tmp"},
            "limits": {"maxFileSizeMb": 10}
        }))
        .unwrap();
        Plan {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            output_dir: "/tmp".into(),
            session_dir: "/tmp/x".into(),
            source_name: "s".into(),
            created_at: String::new(),
            created_local: Local::now(),
            output_size: Size { width: 2, height: 2 },
            encoder: "libx264".into(),
            video_kbps,
            audio_kinds: vec![],
            manifest: SessionManifest {
                version: 1,
                session_id: "x".into(),
                state: ManifestState::Recording,
                app_version: String::new(),
                platform: String::new(),
                created_at: String::new(),
                created_local: String::new(),
                updated_at: String::new(),
                source_name: String::new(),
                file_name_template: String::new(),
                config: cfg.clone(),
                video: None,
                audio: vec![],
                track_mode: TrackMode::Mixed,
                audio_bitrate_kbps: 160,
                active_ms: 0,
                final_path: None,
                error: None,
            },
            config: cfg,
            lock: None,
        }
    }

    #[test]
    fn low_disk_reserves_room_for_finalizing() {
        assert_eq!(low_disk_threshold(500, 0), 508 * MIB);
        assert_eq!(low_disk_threshold(0, 100 * MIB), 100 * MIB + 5 * MIB + 8 * MIB);
    }

    #[test]
    fn size_estimate_accounts_for_unflushed_fragment() {
        let p = plan_for(8000, 2.0); // 1 MB/s target
        // early: projected from the target bitrate
        assert_eq!(estimated_size(0, 1.0, &p), 1_300_000);
        // later: observed rate (1 MB on disk covering ~3.8 s) plus ~1.2 s unflushed
        let e = estimated_size(1_000_000, 5.0, &p);
        assert!(e > 1_350_000 && e < 1_700_000, "{e}");
    }
}
