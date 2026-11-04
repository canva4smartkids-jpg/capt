//! CaptureOne desktop app (Tauri 2). The React UI lives in `apps/desktop/src`; this crate
//! implements the IPC contract in packages/capture-contracts/src/desktop-ipc.ts on top of
//! `media-engine` and the per-OS capture backend.

mod actions;
pub mod capabilities;
mod commands;
pub mod events;
pub mod library;
mod platform;
mod recording;
pub mod region;
pub mod settings;
mod shortcuts;
pub mod state;
mod tray;
pub mod util;
mod windows;

use media_engine::{EngineConfig, MediaEngine};
use parking_lot::{Mutex, RwLock};
use state::{AppCore, AppPaths, Shared};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WindowEvent};

pub use commands::COMMANDS;

pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(shortcuts::plugin())
        .manage(tray::TrayState::default())
        .setup(|app| {
            setup(app.handle())?;
            Ok(())
        })
        .on_window_event(on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::get_capabilities,
            commands::get_permission_status,
            commands::request_permission,
            commands::open_permission_settings,
            commands::list_sources,
            commands::get_source_thumbnail,
            commands::list_audio_devices,
            commands::take_screenshot,
            commands::start_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::stop_recording,
            commands::cancel_recording,
            commands::get_recording_status,
            commands::get_active_session,
            commands::select_region,
            commands::region_selection_done,
            commands::library_list,
            commands::library_rename,
            commands::library_delete,
            commands::library_export,
            commands::library_reveal,
            commands::library_validate,
            commands::library_storage,
            commands::recover_recordings,
            commands::get_settings,
            commands::save_settings,
            commands::get_diagnostics,
            commands::hub_status,
            commands::hub_set_enabled,
            commands::hub_regenerate_code,
            commands::discovery_scan,
            commands::tv_pair,
            commands::tv_send_key,
            commands::tv_screenshot,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("CaptureOne failed to start: {e}");
        std::process::exit(1);
    }
}

fn resolve_paths(app: &AppHandle) -> Result<AppPaths, Box<dyn std::error::Error>> {
    let p = app.path();
    let config_dir = p.app_config_dir()?;
    let data_dir = p.app_data_dir()?;
    let log_dir = p.app_log_dir().unwrap_or_else(|_| data_dir.join("logs"));
    let videos = std::env::var_os("CAPTUREONE_OUTPUT_DIR")
        .map(PathBuf::from)
        .or_else(|| p.video_dir().ok().map(|v| v.join("CaptureOne")))
        .or_else(|| p.home_dir().ok().map(|h| h.join("Videos").join("CaptureOne")))
        .unwrap_or_else(|| data_dir.join("Recordings"));
    Ok(AppPaths { db_path: data_dir.join("library.sqlite3"), config_dir, data_dir, log_dir, default_output_dir: videos })
}

fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let paths = resolve_paths(app)?;
    std::fs::create_dir_all(&paths.config_dir)?;
    std::fs::create_dir_all(&paths.data_dir)?;
    let default_out = paths.default_output_dir.to_string_lossy().to_string();
    let settings = settings::load(&paths.config_dir, &default_out);
    if let Err(e) = std::fs::create_dir_all(&settings.output_directory) {
        log::warn!("cannot create output directory {}: {e}", settings.output_directory);
    }

    let backend = platform::create_backend();
    if let Err(e) = &backend {
        log::error!("capture backend unavailable: {e}");
    }
    let engine = backend.clone().and_then(|b| MediaEngine::new(b, EngineConfig::default()));
    if let Ok(e) = &engine {
        let info = e.ffmpeg_info();
        log::info!("ffmpeg: {:?} ({:?}), ffprobe: {:?}", info.ffmpeg_path, info.ffmpeg_version, info.ffprobe_path);
    }
    let library = library::Library::open(&paths.db_path).or_else(|e| {
        log::error!("library database unavailable ({e}); using a temporary in-memory library");
        library::Library::open_in_memory()
    })?;

    let core: Shared = Arc::new(AppCore {
        paths,
        app_version: app.package_info().version.to_string(),
        backend,
        engine,
        settings: RwLock::new(settings.clone()),
        library: Mutex::new(library),
        active: Mutex::new(None),
        region: Mutex::new(None),
        shortcut_actions: Mutex::new(HashMap::new()),
        last_target: Mutex::new(None),
        capabilities: Mutex::new(None),
    });
    app.manage(core.clone());

    commands::allow_asset_dir(app, &PathBuf::from(&settings.output_directory));
    for problem in shortcuts::register(app, &core, &settings.shortcuts) {
        log::warn!("{problem}");
    }
    if let Err(e) = tray::create(app) {
        log::warn!("system tray unavailable: {e}");
    }

    // Recover interrupted sessions and warm up the (slow) encoder verification.
    let (app2, core2) = (app.clone(), core.clone());
    let _ = std::thread::Builder::new().name("captureone-startup".into()).spawn(move || {
        match commands::recover(&app2, &core2) {
            Ok(found) if !found.is_empty() => {
                log::info!("recovered {} interrupted recording(s)", found.len());
                recording::notify(&app2, "Recovered recordings", &format!("{} interrupted recording(s) were finalized and added to your library.", found.len()));
            }
            Ok(_) => {}
            Err(e) => log::warn!("recovery skipped: {e}"),
        }
        if let Err(e) = recording::capabilities(&core2) {
            log::warn!("capabilities unavailable: {e}");
        }
    });
    Ok(())
}

fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    let app = window.app_handle();
    let Some(core) = app.try_state::<Shared>() else { return };
    match (window.label(), event) {
        (windows::MAIN, WindowEvent::CloseRequested { api, .. }) if core.active().is_some() => {
            // Never lose a running recording by closing the window: keep running in the tray.
            api.prevent_close();
            let _ = window.hide();
            recording::notify(app, "CaptureOne is still recording", "Stop from the tray icon, the indicator or your shortcut.");
        }
        (windows::REGION, WindowEvent::Destroyed) => region::abandon(core.inner()),
        _ => {}
    }
}
