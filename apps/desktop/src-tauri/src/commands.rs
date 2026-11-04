//! Tauri commands: one per entry in `DesktopCommands` (packages/capture-contracts/src/desktop-ipc.ts).
//! Tauri maps the camelCase JS argument keys to these snake_case parameters. Every
//! error is rejected as a serialized contract `CaptureError`.

use crate::events::*;
use crate::library::{annotate_missing, free_disk_bytes};
use crate::state::Shared;
use crate::{actions, recording, region, settings, shortcuts, windows};
use capture_core::*;
use media_engine::AudioDeviceInfo;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

type Cmd<T> = Result<T, CaptureError>;

/// Runs blocking engine/IO work off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CaptureResult<T> + Send + 'static) -> CaptureResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| CaptureError::internal(format!("background task failed: {e}")))?
}

// ------------------------------------------------------------------ capabilities & sources

#[tauri::command]
pub async fn get_capabilities(state: State<'_, Shared>) -> Cmd<CapabilityReport> {
    let core = state.inner().clone();
    blocking(move || recording::capabilities(&core)).await
}

#[tauri::command]
pub async fn get_permission_status(state: State<'_, Shared>, kind: PermissionKind) -> Cmd<PermissionStatus> {
    let backend = state.backend()?;
    blocking(move || Ok(backend.permission_status(kind))).await
}

#[tauri::command]
pub async fn request_permission(state: State<'_, Shared>, kind: PermissionKind) -> Cmd<PermissionStatus> {
    let backend = state.backend()?;
    let core = state.inner().clone();
    let st = blocking(move || backend.request_permission(kind)).await?;
    core.capabilities.lock().take(); // permissions can change capability levels
    Ok(st)
}

#[tauri::command]
pub async fn open_permission_settings(kind: PermissionKind) -> Cmd<()> {
    blocking(move || crate::platform::open_privacy_settings(kind)).await
}

#[tauri::command]
pub async fn list_sources(state: State<'_, Shared>, kinds: Option<Vec<SourceKind>>) -> Cmd<Vec<CaptureSource>> {
    let engine = state.engine()?;
    let kinds = kinds.filter(|k| !k.is_empty()).unwrap_or_else(|| vec![SourceKind::Display, SourceKind::Window]);
    let mut list = blocking(move || engine.list_sources(&kinds)).await?;
    // Never offer our own overlays as capture sources.
    list.retain(|s| !(s.kind == SourceKind::Window && (s.name == windows::INDICATOR_TITLE || s.name == windows::REGION_TITLE)));
    Ok(list)
}

#[tauri::command]
pub async fn get_source_thumbnail(state: State<'_, Shared>, kind: SourceKind, source_id: String, max_width: u32) -> Cmd<String> {
    let engine = state.engine()?;
    blocking(move || engine.source_thumbnail(kind, &source_id, max_width.clamp(32, 7680))).await
}

#[tauri::command]
pub async fn list_audio_devices(state: State<'_, Shared>) -> Cmd<Vec<AudioDeviceInfo>> {
    let engine = state.engine()?;
    blocking(move || engine.list_audio_devices()).await
}

// ------------------------------------------------------------------ capture

#[tauri::command]
pub async fn take_screenshot(app: AppHandle, state: State<'_, Shared>, request: ScreenshotRequest) -> Cmd<MediaMetadata> {
    let core = state.inner().clone();
    blocking(move || actions::take_screenshot(&app, &core, request, true)).await
}

#[tauri::command]
pub async fn start_recording(app: AppHandle, state: State<'_, Shared>, config: RecordingConfig) -> Cmd<String> {
    let core = state.inner().clone();
    blocking(move || recording::start(&app, &core, config)).await
}

#[tauri::command]
pub async fn pause_recording(state: State<'_, Shared>, session_id: String) -> Cmd<()> {
    let ctl = state.session_by_id(&session_id)?;
    let s = ctl.engine_session().ok_or_else(|| CaptureError::invalid_state("the recording has not started yet"))?;
    blocking(move || s.pause()).await
}

#[tauri::command]
pub async fn resume_recording(state: State<'_, Shared>, session_id: String) -> Cmd<()> {
    let ctl = state.session_by_id(&session_id)?;
    let s = ctl.engine_session().ok_or_else(|| CaptureError::invalid_state("the recording has not started yet"))?;
    blocking(move || s.resume()).await
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, Shared>, session_id: String) -> Cmd<MediaMetadata> {
    let core = state.inner().clone();
    let ctl = core.session_by_id(&session_id)?;
    blocking(move || actions::stop(&core, &ctl)).await
}

#[tauri::command]
pub async fn cancel_recording(state: State<'_, Shared>, session_id: String) -> Cmd<()> {
    let ctl = state.session_by_id(&session_id)?;
    blocking(move || actions::cancel(&ctl)).await
}

#[tauri::command]
pub fn get_recording_status(state: State<'_, Shared>, session_id: String) -> Cmd<RecordingStatus> {
    Ok(state.session_by_id(&session_id)?.status())
}

#[tauri::command]
pub fn get_active_session(state: State<'_, Shared>) -> Cmd<Option<RecordingStatus>> {
    Ok(state.active().map(|c| c.status()))
}

// ------------------------------------------------------------------ region

#[tauri::command]
pub async fn select_region(app: AppHandle, state: State<'_, Shared>, display_id: String) -> Cmd<Option<Rect>> {
    region::select_region(app, state.inner().clone(), display_id).await
}

#[tauri::command]
pub fn region_selection_done(state: State<'_, Shared>, rect: Option<Rect>) -> Cmd<()> {
    region::finish(state.inner(), rect)
}

// ------------------------------------------------------------------ library

#[tauri::command]
pub async fn library_list(state: State<'_, Shared>) -> Cmd<Vec<MediaMetadata>> {
    let core = state.inner().clone();
    blocking(move || Ok(annotate_missing(core.library.lock().list()?))).await
}

#[tauri::command]
pub async fn library_rename(state: State<'_, Shared>, id: String, title: String, rename_file: Option<bool>) -> Cmd<MediaMetadata> {
    let core = state.inner().clone();
    let m = blocking(move || {
        let m = core.library.lock().rename(&id, &title, rename_file.unwrap_or(false))?;
        core.library_changed();
        Ok(m)
    })
    .await?;
    Ok(m)
}

#[tauri::command]
pub async fn library_delete(state: State<'_, Shared>, id: String, delete_file: bool) -> Cmd<()> {
    let core = state.inner().clone();
    blocking(move || {
        core.library.lock().delete(&id, delete_file)?;
        core.library_changed();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn library_export(state: State<'_, Shared>, id: String, destination: String) -> Cmd<String> {
    let core = state.inner().clone();
    let dest = PathBuf::from(destination);
    blocking(move || core.library.lock().export(&id, &dest).map(|p| p.to_string_lossy().to_string())).await
}

#[tauri::command]
pub async fn library_reveal(app: AppHandle, state: State<'_, Shared>, id: String) -> Cmd<()> {
    use tauri_plugin_opener::OpenerExt;
    let m = state.library.lock().get(&id)?.ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "library item not found"))?;
    let path = PathBuf::from(&m.path);
    let target = if path.exists() { path } else { path.parent().map(PathBuf::from).ok_or_else(|| CaptureError::io("no folder"))? };
    app.opener().reveal_item_in_dir(&target).map_err(|e| CaptureError::io(format!("could not open the file manager: {e}")))
}

#[tauri::command]
pub async fn library_validate(state: State<'_, Shared>, id: String) -> Cmd<MediaMetadata> {
    let core = state.inner().clone();
    let engine = core.engine()?;
    blocking(move || {
        let m = core.library.lock().get(&id)?.ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "library item not found"))?;
        let inspected = engine.inspect(std::path::Path::new(&m.path))?;
        core.library.lock().apply_inspection(&id, inspected)
    })
    .await
}

#[tauri::command]
pub async fn library_storage(state: State<'_, Shared>) -> Cmd<StorageUsage> {
    let core = state.inner().clone();
    blocking(move || {
        let totals = core.library.lock().storage_totals()?;
        let dir = core.output_dir();
        Ok(StorageUsage {
            total_bytes: totals.total_bytes,
            video_count: totals.video_count,
            screenshot_count: totals.screenshot_count,
            free_disk_bytes: free_disk_bytes(&dir),
            output_directory: dir.to_string_lossy().to_string(),
        })
    })
    .await
}

/// Finalizes interrupted sessions found in the output directory and adds them to the library.
pub fn recover(app: &AppHandle, core: &Shared) -> CaptureResult<Vec<MediaMetadata>> {
    let engine = core.engine()?;
    let dir = core.output_dir();
    let mut out = Vec::new();
    for r in engine.recover_sessions(&dir) {
        match r {
            Ok(m) => match core.library.lock().upsert(&m) {
                Ok(stored) => out.push(stored),
                Err(e) => log::error!("could not add recovered file {}: {e}", m.path),
            },
            Err(e) => {
                log::warn!("recovery failed: {e}");
                emit_capture(app, CaptureEvent::Warning { session_id: None, message: format!("An interrupted recording could not be recovered: {}", e.message) });
            }
        }
    }
    if !out.is_empty() {
        core.library_changed();
    }
    Ok(out)
}

#[tauri::command]
pub async fn recover_recordings(app: AppHandle, state: State<'_, Shared>) -> Cmd<Vec<MediaMetadata>> {
    let core = state.inner().clone();
    blocking(move || recover(&app, &core)).await
}

// ------------------------------------------------------------------ settings & diagnostics

#[tauri::command]
pub fn get_settings(state: State<'_, Shared>) -> Cmd<Value> {
    Ok(state.settings().to_json())
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, state: State<'_, Shared>, settings: Value) -> Cmd<Value> {
    let parsed = settings::from_value_strict(&settings)?;
    let core = state.inner().clone();
    let saved = blocking(move || {
        std::fs::create_dir_all(&parsed.output_directory)?;
        settings::save(&core.paths.config_dir, &parsed)?;
        Ok(parsed)
    })
    .await?;
    let previous = std::mem::replace(&mut *state.settings.write(), saved.clone());
    if previous.output_directory != saved.output_directory {
        allow_asset_dir(&app, &PathBuf::from(&saved.output_directory));
    }
    if previous.shortcuts != saved.shortcuts {
        for p in shortcuts::register(&app, state.inner(), &saved.shortcuts) {
            emit_capture(&app, CaptureEvent::Warning { session_id: None, message: p });
        }
    }
    Ok(saved.to_json())
}

/// Lets `<video>`/`<img>` load files from the output directory through the asset protocol.
pub fn allow_asset_dir(app: &AppHandle, dir: &std::path::Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
        log::warn!("asset scope for {}: {e}", dir.display());
    }
}

#[tauri::command]
pub async fn get_diagnostics(state: State<'_, Shared>) -> Cmd<Diagnostics> {
    let core = state.inner().clone();
    blocking(move || {
        let ff = core.engine.as_ref().ok().map(|e| e.ffmpeg_info());
        let os_version = core.backend.as_ref().ok().map(|b| {
            let c = b.capabilities();
            format!("{} {}", c.os_name, c.os_version)
        });
        Ok(Diagnostics {
            app_version: core.app_version.clone(),
            platform: std::env::consts::OS.to_string(),
            os_version: os_version.unwrap_or_else(|| std::env::consts::OS.to_string()),
            ffmpeg_path: ff.as_ref().and_then(|f| f.ffmpeg_path.clone()),
            ffmpeg_version: ff.as_ref().and_then(|f| f.ffmpeg_version.clone()),
            ffprobe_path: ff.as_ref().and_then(|f| f.ffprobe_path.clone()),
            data_directory: core.paths.data_dir.to_string_lossy().to_string(),
            log_directory: core.paths.log_dir.to_string_lossy().to_string(),
            database_path: core.paths.db_path.to_string_lossy().to_string(),
        })
    })
    .await
}

// ------------------------------------------------------------------ companion hub / TVs

#[tauri::command]
pub async fn hub_status(state: State<'_, Shared>) -> Cmd<HubStatus> {
    let core = state.inner().clone();
    blocking(move || Ok(crate::hub::status(&core))).await
}

/// Starts/stops the LAN hub and persists the choice (`companionHubEnabled`).
#[tauri::command]
pub async fn hub_set_enabled(app: AppHandle, state: State<'_, Shared>, enabled: bool) -> Cmd<HubStatus> {
    let core = state.inner().clone();
    blocking(move || {
        if enabled {
            crate::hub::start(&core, Some(app.clone()), crate::hub::lan_config(&core))?;
        } else {
            crate::hub::stop(&core); // joins the server thread (can take a few seconds)
        }
        let mut s = core.settings();
        if s.companion_hub_enabled != enabled {
            s.companion_hub_enabled = enabled;
            settings::save(&core.paths.config_dir, &s)?;
            *core.settings.write() = s;
        }
        Ok(crate::hub::status(&core))
    })
    .await
}

#[tauri::command]
pub async fn hub_regenerate_code(state: State<'_, Shared>) -> Cmd<HubStatus> {
    let core = state.inner().clone();
    blocking(move || crate::hub::regenerate_code(&core)).await
}

#[tauri::command]
pub async fn discovery_scan(state: State<'_, Shared>, timeout_ms: u64) -> Cmd<Vec<device_discovery::DiscoveredDevice>> {
    let core = state.inner().clone();
    blocking(move || crate::tv::scan(&core, timeout_ms)).await
}

#[tauri::command]
pub async fn tv_pair(state: State<'_, Shared>, device_id: String) -> Cmd<TvPairResult> {
    let core = state.inner().clone();
    blocking(move || crate::tv::pair(&core, &device_id)).await
}

#[tauri::command]
pub async fn tv_send_key(state: State<'_, Shared>, device_id: String, key: device_discovery::TvRemoteKey) -> Cmd<()> {
    let core = state.inner().clone();
    blocking(move || crate::tv::send_key(&core, &device_id, key)).await
}

#[tauri::command]
pub async fn tv_screenshot(app: AppHandle, state: State<'_, Shared>, device_id: String) -> Cmd<MediaMetadata> {
    let core = state.inner().clone();
    blocking(move || crate::tv::screenshot(Some(&app), &core, &device_id)).await
}

/// Additive (proposed) command: URL the preview `<video>` should load. On Linux, WebKitGTK
/// cannot play media from the `asset://` scheme, so a loopback endpoint is used; elsewhere
/// this returns null and the UI uses the asset protocol.
#[tauri::command]
pub async fn media_stream_url(state: State<'_, Shared>, id: String) -> Cmd<Option<String>> {
    if !cfg!(target_os = "linux") {
        return Ok(None);
    }
    let core = state.inner().clone();
    blocking(move || {
        core.library.lock().get(&id)?.ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "library item not found"))?;
        let info = crate::media_server::ensure(&core)?;
        Ok(Some(crate::media_server::url_for(&info, &id)))
    })
    .await
}

/// Commands beyond the contract (proposed upstream; see the desktop report).
/// Commands registered beyond the contract (none: `media_stream_url` is now in the contract).
pub const EXTRA_COMMANDS: &[&str] = &[];

/// Command names, in contract order (used by build.rs ACL generation and tests).
pub const COMMANDS: &[&str] = &[
    "media_stream_url",
    "get_capabilities",
    "get_permission_status",
    "request_permission",
    "open_permission_settings",
    "list_sources",
    "get_source_thumbnail",
    "list_audio_devices",
    "take_screenshot",
    "start_recording",
    "pause_recording",
    "resume_recording",
    "stop_recording",
    "cancel_recording",
    "get_recording_status",
    "get_active_session",
    "select_region",
    "region_selection_done",
    "library_list",
    "library_rename",
    "library_delete",
    "library_export",
    "library_reveal",
    "library_validate",
    "library_storage",
    "recover_recordings",
    "get_settings",
    "save_settings",
    "get_diagnostics",
    "hub_status",
    "hub_set_enabled",
    "hub_regenerate_code",
    "discovery_scan",
    "tv_pair",
    "tv_send_key",
    "tv_screenshot",
];

#[cfg(test)]
mod tests {
    use super::COMMANDS;

    /// Every command in the TS contract is implemented (names must match exactly).
    #[test]
    fn implements_every_contract_command() {
        let ts = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../packages/capture-contracts/src/desktop-ipc.ts")).unwrap();
        let body = ts.split("export interface DesktopCommands {").nth(1).unwrap().split("\n}").next().unwrap();
        let mut contract: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("/**") && !l.starts_with('*') && l.contains(": ["))
            .filter_map(|l| l.split(':').next())
            .collect();
        contract.sort();
        let mut ours: Vec<&str> = COMMANDS.to_vec();
        ours.sort();
        assert_eq!(ours, contract);
        let build_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs")).unwrap();
        for c in COMMANDS.iter().chain(super::EXTRA_COMMANDS) {
            assert!(build_rs.contains(&format!("\"{c}\"")), "build.rs ACL list misses {c}");
        }
    }
}
