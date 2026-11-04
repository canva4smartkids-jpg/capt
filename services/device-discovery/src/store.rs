//! Persistent TV credentials: LG client-keys, Samsung tokens and pinned TLS certificate
//! fingerprints, keyed by TV IP address. Stored as JSON with owner-only permissions.

use capture_core::{CaptureError, CaptureResult};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvCredentials {
    /// LG `client-key` or Samsung `token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// SHA-256 (hex) of the TV's self-signed TLS certificate, pinned on first use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_sha256: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    lg: BTreeMap<String, TvCredentials>,
    #[serde(default)]
    samsung: BTreeMap<String, TvCredentials>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Lg,
    Samsung,
}

pub struct CredentialStore {
    path: Option<PathBuf>,
    data: Mutex<Data>,
}

impl CredentialStore {
    /// Opens (or creates) `dir/tv-credentials.json`.
    pub fn open(dir: &Path) -> CaptureResult<Arc<Self>> {
        std::fs::create_dir_all(dir).map_err(CaptureError::from)?;
        let path = dir.join("tv-credentials.json");
        let data = match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
                log::warn!("ignoring invalid {path:?}: {e}");
                Data::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Arc::new(Self { path: Some(path), data: Mutex::new(data) }))
    }

    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self { path: None, data: Mutex::new(Data::default()) })
    }

    pub fn get(&self, vendor: Vendor, ip: &str) -> TvCredentials {
        let d = self.data.lock();
        let map = match vendor {
            Vendor::Lg => &d.lg,
            Vendor::Samsung => &d.samsung,
        };
        map.get(ip).cloned().unwrap_or_default()
    }

    pub fn update(&self, vendor: Vendor, ip: &str, f: impl FnOnce(&mut TvCredentials)) -> CaptureResult<()> {
        let mut d = self.data.lock();
        let map = match vendor {
            Vendor::Lg => &mut d.lg,
            Vendor::Samsung => &mut d.samsung,
        };
        let entry = map.entry(ip.to_string()).or_default();
        let before = entry.clone();
        f(entry);
        if *entry != before {
            self.persist(&d)?;
        }
        Ok(())
    }

    /// Forgets a TV (it will prompt for pairing again and its certificate is re-pinned).
    pub fn forget(&self, ip: &str) -> CaptureResult<()> {
        let mut d = self.data.lock();
        d.lg.remove(ip);
        d.samsung.remove(ip);
        self.persist(&d)
    }

    fn persist(&self, d: &Data) -> CaptureResult<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_vec_pretty(d).map_err(|e| CaptureError::internal(e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            let mut f = o.open(&tmp).map_err(CaptureError::from)?;
            f.write_all(&json).map_err(CaptureError::from)?;
            f.sync_all().map_err(CaptureError::from)?;
        }
        std::fs::rename(&tmp, path).map_err(CaptureError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists() {
        let dir = tempfile::tempdir().unwrap();
        let s = CredentialStore::open(dir.path()).unwrap();
        s.update(Vendor::Lg, "1.2.3.4", |c| c.secret = Some("k".into())).unwrap();
        let s2 = CredentialStore::open(dir.path()).unwrap();
        assert_eq!(s2.get(Vendor::Lg, "1.2.3.4").secret.as_deref(), Some("k"));
        assert_eq!(s2.get(Vendor::Samsung, "1.2.3.4"), TvCredentials::default());
        s2.forget("1.2.3.4").unwrap();
        assert_eq!(CredentialStore::open(dir.path()).unwrap().get(Vendor::Lg, "1.2.3.4"), TvCredentials::default());
    }
}
