//! Managed application state shared by commands, the tray, shortcuts and listeners.

use crate::events::ShortcutAction;
use crate::library::Library;
use crate::settings::Settings;
use capture_core::*;
use media_engine::{MediaEngine, RecordingSession};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;

pub type Shared = Arc<AppCore>;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub db_path: PathBuf,
    pub default_output_dir: PathBuf,
}

/// One recording (from countdown until completion). The engine session exists once
/// the countdown has finished.
pub struct SessionCtl {
    /// Id handed to the UI by `start_recording` (stable across countdown and engine session).
    pub id: String,
    pub config: RecordingConfig,
    pub phase: Mutex<RecordingState>,
    pub countdown_remaining: AtomicU32,
    pub cancel_requested: AtomicBool,
    pub cancel_tx: Mutex<Option<crossbeam_channel::Sender<()>>>,
    pub session: Mutex<Option<RecordingSession>>,
    pub minimized_main: AtomicBool,
    pub indicator_shown: AtomicBool,
    pub last_error: Mutex<Option<CaptureError>>,
}

impl SessionCtl {
    pub fn new(id: String, config: RecordingConfig, initial: RecordingState) -> Self {
        Self {
            id,
            countdown_remaining: AtomicU32::new(config.countdown_sec),
            config,
            phase: Mutex::new(initial),
            cancel_requested: AtomicBool::new(false),
            cancel_tx: Mutex::new(None),
            session: Mutex::new(None),
            minimized_main: AtomicBool::new(false),
            indicator_shown: AtomicBool::new(false),
            last_error: Mutex::new(None),
        }
    }

    pub fn phase(&self) -> RecordingState {
        *self.phase.lock()
    }

    pub fn engine_session(&self) -> Option<RecordingSession> {
        self.session.lock().clone()
    }

    /// Status as the UI sees it (session id rewritten to the app id; countdown synthesized).
    pub fn status(&self) -> RecordingStatus {
        let phase = self.phase();
        if let Some(s) = self.engine_session() {
            let mut st = s.status();
            st.session_id = self.id.clone();
            return st;
        }
        RecordingStatus {
            session_id: self.id.clone(),
            state: phase,
            elapsed_ms: 0,
            countdown_remaining_sec: (phase == RecordingState::Countdown).then(|| self.countdown_remaining.load(std::sync::atomic::Ordering::SeqCst)),
            bytes_written: 0,
            frames_encoded: 0,
            frames_dropped: 0,
            estimated_bytes_per_minute: None,
            free_disk_bytes: None,
            output_path: None,
            encoder: None,
            warnings: vec![],
            error: self.last_error.lock().clone(),
        }
    }
}

pub struct AppCore {
    pub paths: AppPaths,
    pub app_version: String,
    pub backend: CaptureResult<Arc<dyn CaptureBackend>>,
    pub engine: CaptureResult<MediaEngine>,
    pub settings: RwLock<Settings>,
    pub library: Mutex<Library>,
    pub active: Mutex<Option<Arc<SessionCtl>>>,
    pub region: Mutex<Option<tokio::sync::oneshot::Sender<Option<Rect>>>>,
    pub shortcut_actions: Mutex<HashMap<u32, ShortcutAction>>,
    pub last_target: Mutex<Option<CaptureTarget>>,
    pub capabilities: Mutex<Option<CapabilityReport>>,
}

impl AppCore {
    pub fn engine(&self) -> CaptureResult<MediaEngine> {
        self.engine.clone()
    }

    pub fn backend(&self) -> CaptureResult<Arc<dyn CaptureBackend>> {
        self.backend.clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    pub fn output_dir(&self) -> PathBuf {
        let s = self.settings.read();
        if s.output_directory.trim().is_empty() {
            self.paths.default_output_dir.clone()
        } else {
            PathBuf::from(&s.output_directory)
        }
    }

    pub fn active(&self) -> Option<Arc<SessionCtl>> {
        self.active.lock().clone()
    }

    /// The active session if its id matches.
    pub fn session_by_id(&self, id: &str) -> CaptureResult<Arc<SessionCtl>> {
        match self.active() {
            Some(s) if s.id == id => Ok(s),
            _ => Err(CaptureError::invalid_state(format!("no active recording with id '{id}'"))),
        }
    }
}
