//! Global shortcuts from settings (re-registered whenever settings change).

use crate::events::{emit_shortcut, ShortcutAction};
use crate::settings::ShortcutBindings;
use crate::state::Shared;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let Some(core) = app.try_state::<Shared>() else { return };
            let action = core.shortcut_actions.lock().get(&shortcut.id()).copied();
            if let Some(action) = action {
                emit_shortcut(app, action);
                crate::actions::perform(app, action);
            }
        })
        .build()
}

pub fn bindings(b: &ShortcutBindings) -> [(ShortcutAction, &str); 4] {
    [
        (ShortcutAction::StartStop, b.start_stop.as_str()),
        (ShortcutAction::PauseResume, b.pause_resume.as_str()),
        (ShortcutAction::Screenshot, b.screenshot.as_str()),
        (ShortcutAction::RegionScreenshot, b.region_screenshot.as_str()),
    ]
}

/// Replaces all registrations. Returns human-readable problems (invalid or taken shortcuts).
pub fn register(app: &AppHandle, core: &Shared, b: &ShortcutBindings) -> Vec<String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut map = core.shortcut_actions.lock();
    map.clear();
    let mut problems = Vec::new();
    for (action, accel) in bindings(b) {
        if accel.trim().is_empty() {
            continue;
        }
        match accel.parse::<Shortcut>() {
            Ok(sc) => match gs.register(sc) {
                Ok(()) => {
                    map.insert(sc.id(), action);
                }
                Err(e) => problems.push(format!("Shortcut {accel} could not be registered ({e}); another app may be using it.")),
            },
            Err(e) => problems.push(format!("Shortcut {accel} is not valid: {e}")),
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_accelerators_parse() {
        for (_, accel) in bindings(&ShortcutBindings::default()) {
            accel.parse::<Shortcut>().unwrap_or_else(|e| panic!("{accel}: {e}"));
        }
        for accel in ["Alt+F9", "CommandOrControl+Alt+Up", "Control+Shift+Space", "PrintScreen", "Super+Shift+1"] {
            accel.parse::<Shortcut>().unwrap_or_else(|e| panic!("{accel}: {e}"));
        }
    }
}
