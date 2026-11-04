//! LAN discovery and TV control (device-discovery crate). Every call here may block for a
//! long time (pairing waits up to 60 s for the user to accept on the TV), so commands run
//! these on blocking threads.

use crate::events::{emit_capture, CaptureEvent, TvPairResult};
use crate::state::Shared;
use capture_core::*;
use device_discovery::{ControlProtocol, CredentialStore, DiscoveredDevice, TvClient, TvRemoteKey};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

pub fn credential_store(core: &Shared) -> CaptureResult<Arc<CredentialStore>> {
    let mut slot = core.tv_store.lock();
    if let Some(s) = slot.as_ref() {
        return Ok(s.clone());
    }
    let store = CredentialStore::open(&core.paths.data_dir.join("tv"))?;
    *slot = Some(store.clone());
    Ok(store)
}

/// Scans the LAN and remembers the results (keyed by device id) for pairing/control.
pub fn scan(core: &Shared, timeout_ms: u64) -> CaptureResult<Vec<DiscoveredDevice>> {
    let devices = device_discovery::scan(Duration::from_millis(timeout_ms.clamp(500, 15_000)))?;
    let mut map = core.discovered.lock();
    map.clear();
    for d in &devices {
        map.insert(d.id.clone(), d.clone());
    }
    Ok(devices)
}

fn device(core: &Shared, id: &str) -> CaptureResult<DiscoveredDevice> {
    core.discovered.lock().get(id).cloned().ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, "This device is not in the last scan. Scan the network again."))
}

/// Existing control session, or a new one (pairs on first use: the TV shows a prompt).
fn client(core: &Shared, d: &DiscoveredDevice) -> CaptureResult<Arc<Mutex<TvClient>>> {
    if let Some(c) = core.tv_clients.lock().get(&d.id) {
        return Ok(c.clone());
    }
    let connected = Arc::new(Mutex::new(TvClient::connect(d, credential_store(core)?)?));
    let mut map = core.tv_clients.lock();
    Ok(map.entry(d.id.clone()).or_insert(connected).clone())
}

/// Forgets a broken connection so the next call reconnects.
fn drop_on_network_error<T>(core: &Shared, id: &str, r: CaptureResult<T>) -> CaptureResult<T> {
    if let Err(e) = &r {
        if matches!(e.code, CaptureErrorCode::Network | CaptureErrorCode::Interrupted | CaptureErrorCode::IoError) {
            core.tv_clients.lock().remove(id);
        }
    }
    r
}

pub fn pair(core: &Shared, device_id: &str) -> CaptureResult<TvPairResult> {
    let d = device(core, device_id)?;
    core.tv_clients.lock().remove(&d.id);
    match client(core, &d) {
        Ok(_) => {
            let what = if d.control_protocols.contains(&ControlProtocol::LgSsap) {
                "Remote keys and the experimental LG screenshot are available."
            } else {
                "Remote keys are available (Samsung TVs offer no screenshot API)."
            };
            Ok(TvPairResult { paired: true, message: format!("Paired with {}. {what}", d.name) })
        }
        // Not controllable at all: a real error for the UI.
        Err(e) if matches!(e.code, CaptureErrorCode::Unsupported | CaptureErrorCode::InvalidConfig) => Err(e),
        // Declined on the TV, timed out, unreachable: report as "not paired" with the reason.
        Err(e) => Ok(TvPairResult { paired: false, message: e.message }),
    }
}

pub fn send_key(core: &Shared, device_id: &str, key: TvRemoteKey) -> CaptureResult<()> {
    let d = device(core, device_id)?;
    let c = client(core, &d)?;
    let r = c.lock().send_key(key);
    drop_on_network_error(core, &d.id, r)
}

/// LG webOS only (experimental, undocumented SSAP call). Saves into the output folder and
/// adds a screenshot item marked as coming from the TV.
pub fn screenshot(app: Option<&AppHandle>, core: &Shared, device_id: &str) -> CaptureResult<MediaMetadata> {
    let d = device(core, device_id)?;
    if !d.control_protocols.contains(&ControlProtocol::LgSsap) {
        return Err(CaptureError::unsupported(format!("{} has no screenshot API; only LG webOS TVs offer an (undocumented) one.", d.name)));
    }
    let c = client(core, &d)?;
    let dir = core.output_dir();
    let r = c.lock().screenshot(&dir, &d.name);
    let mut media = drop_on_network_error(core, &d.id, r)?;
    media.title = format!("{} — TV screenshot (experimental)", d.name);
    media.source_name = Some(format!("{} (LG webOS TV)", d.name));
    let stored = core.library.lock().upsert(&media)?;
    if let Some(app) = app {
        emit_capture(app, CaptureEvent::Screenshot { media: stored.clone() });
    }
    core.library_changed();
    Ok(stored)
}
