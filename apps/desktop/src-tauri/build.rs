//! Generates per-command ACL permissions (`allow-<command>`) so each window's capability
//! file grants only the commands that window needs (see capabilities/ and permissions/).

const COMMANDS: &[&str] = &[
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

fn main() {
    let attrs = tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(e) = tauri_build::try_build(attrs) {
        panic!("tauri-build failed: {e:#}");
    }
}
