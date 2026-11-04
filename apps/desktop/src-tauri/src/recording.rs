//! Recording orchestration: countdown (cancellable, in the backend), engine session
//! start, listener → `capture://event` forwarding, library insertion and notifications.

use crate::events::{emit_capture, CaptureEvent};
use crate::state::{SessionCtl, Shared};
use crate::{tray, windows};
use capture_core::*;
use media_engine::{MediaEngine, RecorderListener};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

/// Applies capability gating so tray/shortcut recordings never request something the
/// device cannot do (the UI applies the same rules before calling `start_recording`).
pub fn apply_gating(cfg: &mut RecordingConfig, caps: &CapabilityReport) {
    if !caps.is_usable(FeatureKey::SystemAudio) {
        cfg.audio.system.enabled = false;
    }
    if !caps.is_usable(FeatureKey::MicrophoneAudio) {
        cfg.audio.microphone.enabled = false;
    }
    if cfg.audio.track_mode == TrackMode::Separate && !caps.is_usable(FeatureKey::SeparateAudioTracks) {
        cfg.audio.track_mode = TrackMode::Mixed;
    }
    let verified = |c: VideoCodec| caps.encoders.iter().any(|e| e.verified && e.codec == c);
    if !verified(cfg.video.codec) {
        if let Some(c) = [VideoCodec::H264, VideoCodec::H265].into_iter().find(|c| verified(*c)) {
            cfg.video.codec = c;
        }
    }
}

/// Primary display (or the first one).
pub fn primary_display(core: &Shared) -> CaptureResult<CaptureSource> {
    let displays = core.backend()?.list_sources(&[SourceKind::Display])?;
    displays
        .iter()
        .find(|d| d.is_primary == Some(true))
        .or_else(|| displays.first())
        .cloned()
        .ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "no display found"))
}

/// Capabilities including app-level features (cached after the first, slow, encoder probe).
pub fn capabilities(core: &Shared) -> CaptureResult<CapabilityReport> {
    if let Some(c) = core.capabilities.lock().clone() {
        return Ok(c);
    }
    let engine = core.engine()?;
    let report = crate::capabilities::with_app_features(engine.capabilities(), &core.app_version);
    *core.capabilities.lock() = Some(report.clone());
    Ok(report)
}

/// Config for recordings started from the tray or a global shortcut (blocking).
pub fn default_config(core: &Shared) -> CaptureResult<RecordingConfig> {
    let settings = core.settings();
    let target = match core.last_target.lock().clone() {
        Some(t) => t,
        None => CaptureTarget::Display { display_id: primary_display(core)?.id },
    };
    let mut cfg = settings.recording_config(target, Some(&core.output_dir().to_string_lossy()));
    apply_gating(&mut cfg, &capabilities(core)?);
    Ok(cfg)
}

/// Validates, registers the session and runs countdown + engine start on a worker thread.
/// Returns the session id immediately (the countdown is reported via events).
pub fn start(app: &AppHandle, core: &Shared, config: RecordingConfig) -> CaptureResult<String> {
    config.validate()?;
    let engine = core.engine()?;
    std::fs::create_dir_all(&config.output.directory)?;
    let id = uuid::Uuid::new_v4().to_string();
    let ctl = Arc::new(SessionCtl::new(id.clone(), config.clone(), RecordingState::Idle));
    {
        let mut active = core.active.lock();
        if active.is_some() {
            return Err(CaptureError::invalid_state("a recording is already in progress"));
        }
        *active = Some(ctl.clone());
    }
    *core.last_target.lock() = Some(config.target.clone());
    let (tx, rx) = crossbeam_channel::bounded::<()>(1);
    *ctl.cancel_tx.lock() = Some(tx);
    let (app2, core2) = (app.clone(), core.clone());
    let spawned = std::thread::Builder::new().name("recording-start".into()).spawn(move || run(app2, core2, engine, ctl, rx));
    if let Err(e) = spawned {
        core.active.lock().take();
        return Err(CaptureError::internal(format!("cannot spawn recording thread: {e}")));
    }
    Ok(id)
}

fn set_phase(ctl: &SessionCtl, to: RecordingState) -> RecordingState {
    std::mem::replace(&mut *ctl.phase.lock(), to)
}

fn clear_active(core: &Shared, ctl: &Arc<SessionCtl>) {
    let mut active = core.active.lock();
    if active.as_ref().is_some_and(|a| Arc::ptr_eq(a, ctl)) {
        *active = None;
    }
}

fn run(app: AppHandle, core: Shared, engine: MediaEngine, ctl: Arc<SessionCtl>, cancel_rx: crossbeam_channel::Receiver<()>) {
    let countdown = ctl.config.countdown_sec;
    let mut cancelled = false;
    if countdown > 0 {
        let from = set_phase(&ctl, RecordingState::Countdown);
        emit_capture(&app, CaptureEvent::StateChanged { session_id: ctl.id.clone(), from, to: RecordingState::Countdown, at: now_iso8601() });
        tray::refresh(&app, &core);
        for remaining in (1..=countdown).rev() {
            ctl.countdown_remaining.store(remaining, Ordering::SeqCst);
            emit_capture(&app, CaptureEvent::Countdown { session_id: ctl.id.clone(), remaining_sec: remaining });
            match cancel_rx.recv_timeout(Duration::from_secs(1)) {
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                _ => {
                    cancelled = true;
                    break;
                }
            }
        }
        ctl.countdown_remaining.store(0, Ordering::SeqCst);
    }
    ctl.cancel_tx.lock().take();
    if cancelled || ctl.cancel_requested.load(Ordering::SeqCst) {
        let from = set_phase(&ctl, RecordingState::Idle);
        emit_capture(&app, CaptureEvent::StateChanged { session_id: ctl.id.clone(), from, to: RecordingState::Idle, at: now_iso8601() });
        clear_active(&core, &ctl);
        tray::refresh(&app, &core);
        return;
    }
    let listener = Arc::new(Listener { app: app.clone(), core: core.clone(), ctl: ctl.clone() });
    match engine.start_recording(ctl.config.clone(), listener) {
        Ok(session) => {
            *ctl.session.lock() = Some(session.clone());
            if ctl.cancel_requested.load(Ordering::SeqCst) {
                if let Err(e) = session.cancel() {
                    log::warn!("cancel after start failed: {e}");
                }
            }
        }
        Err(e) => {
            // The engine already reported Starting → Failed and the error through the listener.
            log::error!("recording failed to start: {e}");
            clear_active(&core, &ctl);
            tray::refresh(&app, &core);
            notify(&app, "Recording could not start", &e.message);
        }
    }
}

pub struct Listener {
    app: AppHandle,
    core: Shared,
    ctl: Arc<SessionCtl>,
}

impl RecorderListener for Listener {
    fn on_state(&self, _engine_session: &str, _from: RecordingState, to: RecordingState) {
        let from = set_phase(&self.ctl, to);
        let cancelling = self.ctl.cancel_requested.load(Ordering::SeqCst);
        if cancelling && to == RecordingState::Failed {
            return; // reported as a return to idle on completion
        }
        emit_capture(&self.app, CaptureEvent::StateChanged { session_id: self.ctl.id.clone(), from, to, at: now_iso8601() });
        if to == RecordingState::Recording && from == RecordingState::Starting {
            let settings = self.core.settings();
            if settings.show_recording_indicator && !self.ctl.indicator_shown.swap(true, Ordering::SeqCst) {
                if let Err(e) = windows::open_indicator(&self.app) {
                    log::warn!("indicator window: {e}");
                }
            }
            if settings.minimize_on_record && windows::minimize_main(&self.app) {
                self.ctl.minimized_main.store(true, Ordering::SeqCst);
            }
        }
        tray::refresh(&self.app, &self.core);
        self.core.hub_status_changed(true);
    }

    fn on_status(&self, status: &RecordingStatus) {
        let mut st = status.clone();
        st.session_id = self.ctl.id.clone();
        tray::tooltip(&self.app, st.state, st.elapsed_ms);
        emit_capture(&self.app, CaptureEvent::Status { status: st });
        self.core.hub_status_changed(false);
    }

    fn on_warning(&self, _engine_session: &str, message: &str) {
        emit_capture(&self.app, CaptureEvent::Warning { session_id: Some(self.ctl.id.clone()), message: message.to_string() });
    }

    fn on_error(&self, _engine_session: &str, error: &CaptureError) {
        if self.ctl.cancel_requested.load(Ordering::SeqCst) {
            return;
        }
        *self.ctl.last_error.lock() = Some(error.clone());
        emit_capture(&self.app, CaptureEvent::Error { session_id: Some(self.ctl.id.clone()), error: error.clone() });
    }

    fn on_completed(&self, _engine_session: &str, reason: StopReason, result: &CaptureResult<MediaMetadata>) {
        clear_active(&self.core, &self.ctl);
        windows::close_indicator(&self.app);
        if self.ctl.minimized_main.swap(false, Ordering::SeqCst) {
            windows::show_main(&self.app);
        }
        tray::refresh(&self.app, &self.core);
        self.core.hub_status_changed(true);
        let cancelled = self.ctl.cancel_requested.load(Ordering::SeqCst) && matches!(result, Err(e) if e.code == CaptureErrorCode::Interrupted);
        if cancelled {
            let from = set_phase(&self.ctl, RecordingState::Idle);
            emit_capture(&self.app, CaptureEvent::StateChanged { session_id: self.ctl.id.clone(), from, to: RecordingState::Idle, at: now_iso8601() });
            return;
        }
        match result {
            Ok(media) => {
                let stored = match self.core.library.lock().upsert(media) {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("library insert failed: {e}");
                        media.clone()
                    }
                };
                let valid = stored.validation.as_ref().is_some_and(|v| v.valid);
                let duration = stored.duration_ms.map(|d| format_duration(d as u64)).unwrap_or_default();
                let body = format!("{} · {} · {}", stored.file_name, duration, format_bytes(stored.size_bytes));
                notify(&self.app, if valid { "Recording saved" } else { "Recording saved with problems" }, &body);
                emit_capture(&self.app, CaptureEvent::Completed { session_id: self.ctl.id.clone(), reason, media: stored });
                self.core.library_changed();
            }
            Err(e) => {
                let already = self.ctl.last_error.lock().as_ref() == Some(e);
                if !already {
                    emit_capture(&self.app, CaptureEvent::Error { session_id: Some(self.ctl.id.clone()), error: e.clone() });
                }
                notify(&self.app, "Recording failed", &e.message);
            }
        }
    }
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::info!("notification not shown ({e}): {title} — {body}");
    }
}

pub fn format_duration(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn format_bytes(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 || v >= 100.0 {
        format!("{} {}", v.round(), units[i])
    } else {
        format!("{v:.1} {}", units[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn caps(features: &[(FeatureKey, SupportLevel)], encoders: &[(VideoCodec, bool)]) -> CapabilityReport {
        CapabilityReport {
            schema_version: 1,
            platform: Platform::Linux,
            os_name: "Linux".into(),
            os_version: "6".into(),
            device_model: None,
            app_version: "0".into(),
            generated_at: "t".into(),
            features: features.iter().map(|(k, l)| (*k, FeatureSupport { level: *l, reason: None, requires: None, min_os_version: None })).collect::<BTreeMap<_, _>>(),
            encoders: encoders.iter().map(|(c, v)| EncoderInfo { codec: *c, name: format!("{c:?}"), hardware: false, verified: *v }).collect(),
            frame_rates: vec![30],
            screenshot_formats: vec![ImageFormat::Png],
            max_recording_resolution: None,
            notes: vec![],
        }
    }

    #[test]
    fn gating_disables_unusable_sources_and_falls_back_to_verified_codec() {
        let mut cfg = crate::settings::Settings::defaults("/v").recording_config(CaptureTarget::Display { display_id: "d".into() }, None);
        cfg.video.codec = VideoCodec::H265;
        cfg.audio.system.enabled = true;
        cfg.audio.microphone.enabled = true;
        cfg.audio.track_mode = TrackMode::Separate;
        apply_gating(&mut cfg, &caps(&[(FeatureKey::MicrophoneAudio, SupportLevel::Native), (FeatureKey::SystemAudio, SupportLevel::Unsupported)], &[(VideoCodec::H264, true), (VideoCodec::H265, false)]));
        assert_eq!(cfg.video.codec, VideoCodec::H264);
        assert!(!cfg.audio.system.enabled);
        assert!(cfg.audio.microphone.enabled);
        assert_eq!(cfg.audio.track_mode, TrackMode::Mixed);

        let mut cfg2 = cfg.clone();
        cfg2.audio.system.enabled = true;
        apply_gating(&mut cfg2, &caps(&[(FeatureKey::SystemAudio, SupportLevel::Partial)], &[]));
        assert!(cfg2.audio.system.enabled, "partial support stays usable");
        assert!(!cfg2.audio.microphone.enabled);
        assert_eq!(cfg2.video.codec, VideoCodec::H264, "no verified codec: leave as is, engine reports ENCODER_UNAVAILABLE");
    }

    #[test]
    fn human_formats() {
        assert_eq!(format_duration(65_000), "01:05");
        assert_eq!(format_duration(3_726_000), "1:02:06");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(5 * 1024 * 1024 + 300_000), "5.3 MB");
    }
}
