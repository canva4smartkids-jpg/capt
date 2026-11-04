//! Pairing: 6-digit codes, per-client lockout, random bearer tokens stored as SHA-256 hashes.

use crate::protocol::{PairRequest, PairedDevice};
use capture_core::{CaptureError, CaptureResult};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Maximum number of remembered companions. Pairing more fails until one is removed.
pub const MAX_PAIRED_DEVICES: usize = 64;
/// Upper bound on per-address failure records (an IPv6 /64 has unlimited addresses).
const MAX_TRACKED_CLIENTS: usize = 4096;
const STORE_FILE: &str = "companion-paired-devices.json";

/// Pairing brute-force protection.
#[derive(Debug, Clone)]
pub struct PairingPolicy {
    /// Consecutive wrong codes from one client address before it is locked out.
    pub max_failures: u32,
    /// First lockout duration; doubles with every further lockout of the same client, capped at `max_lockout`.
    pub lockout: Duration,
    pub max_lockout: Duration,
    /// Total wrong codes (all clients) after which the current code is rotated automatically.
    pub rotate_after_failures: u32,
}

impl Default for PairingPolicy {
    fn default() -> Self {
        Self { max_failures: 5, lockout: Duration::from_secs(30), max_lockout: Duration::from_secs(15 * 60), rotate_after_failures: 30 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PairError {
    Invalid(String),
    WrongCode { remaining_attempts: u32 },
    LockedOut { retry_after: Duration },
    TooManyDevices,
    Storage(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredDevice {
    #[serde(flatten)]
    device: PairedDevice,
    /// Hex SHA-256 of the bearer token. The token itself is never stored.
    token_sha256: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    devices: Vec<StoredDevice>,
}

#[derive(Debug, Default)]
struct ClientFailures {
    consecutive: u32,
    lockouts: u32,
    locked_until: Option<Instant>,
}

struct Inner {
    code: String,
    devices: Vec<StoredDevice>,
    failures: HashMap<IpAddr, ClientFailures>,
    failures_since_rotation: u32,
}

pub struct Pairing {
    inner: Mutex<Inner>,
    store_path: PathBuf,
    policy: PairingPolicy,
}

pub fn random_bytes<const N: usize>() -> CaptureResult<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| CaptureError::internal(format!("OS random generator failed: {e}")))?;
    Ok(buf)
}

/// Uniform 6-digit code (rejection sampling avoids modulo bias).
pub fn random_code() -> CaptureResult<String> {
    loop {
        let n = u32::from_le_bytes(random_bytes::<4>()?);
        // 4_294_000_000 is the largest multiple of 1_000_000 below u32::MAX.
        if n < 4_294_000_000 {
            return Ok(format!("{:06}", n % 1_000_000));
        }
    }
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Pairing {
    pub fn load(state_dir: &Path, policy: PairingPolicy) -> CaptureResult<Self> {
        std::fs::create_dir_all(state_dir).map_err(|e| CaptureError::from(e).with_detail("path", state_dir.display().to_string()))?;
        let store_path = state_dir.join(STORE_FILE);
        let devices = match std::fs::read_to_string(&store_path) {
            Ok(text) => match serde_json::from_str::<StoreFile>(&text) {
                Ok(f) => f.devices,
                Err(e) => {
                    // A corrupt store must not brick the hub: keep a copy and start empty.
                    let backup = store_path.with_extension("json.corrupt");
                    let _ = std::fs::rename(&store_path, &backup);
                    log::warn!("paired-device store {store_path:?} is invalid ({e}); moved to {backup:?}");
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(CaptureError::from(e).with_detail("path", store_path.display().to_string())),
        };
        Ok(Self {
            inner: Mutex::new(Inner { code: random_code()?, devices, failures: HashMap::new(), failures_since_rotation: 0 }),
            store_path,
            policy,
        })
    }

    pub fn code(&self) -> String {
        self.inner.lock().code.clone()
    }

    pub fn regenerate(&self) -> CaptureResult<String> {
        let code = random_code()?;
        let mut g = self.inner.lock();
        g.code = code.clone();
        g.failures_since_rotation = 0;
        Ok(code)
    }

    pub fn devices(&self) -> Vec<PairedDevice> {
        self.inner.lock().devices.iter().map(|d| d.device.clone()).collect()
    }

    /// Removes a paired companion; its token stops working immediately.
    pub fn remove(&self, device_id: &str) -> CaptureResult<bool> {
        let mut g = self.inner.lock();
        let before = g.devices.len();
        g.devices.retain(|d| d.device.id != device_id);
        let removed = g.devices.len() != before;
        if removed {
            self.persist(&g.devices)?;
        }
        Ok(removed)
    }

    /// Validates the token and returns the paired device it belongs to.
    pub fn authenticate(&self, token: &str) -> Option<PairedDevice> {
        if token.len() < 32 || token.len() > 256 {
            return None;
        }
        let digest = sha256_hex(token);
        let g = self.inner.lock();
        let mut found = None;
        // Compare against every entry so timing does not depend on the position.
        for d in &g.devices {
            if ct_eq(d.token_sha256.as_bytes(), digest.as_bytes()) {
                found = Some(d.device.clone());
            }
        }
        found
    }

    /// Checks the code for `client`, applying lockout. On success returns the new token.
    pub fn pair(&self, client: IpAddr, req: &PairRequest, now_iso: String) -> Result<(String, PairedDevice), PairError> {
        req.validate().map_err(PairError::Invalid)?;
        let now = Instant::now();
        let mut g = self.inner.lock();
        let policy = self.policy.clone();
        if g.failures.len() >= MAX_TRACKED_CLIENTS {
            // Bound memory: drop clients that are neither locked out nor mid-streak.
            g.failures.retain(|_, f| f.locked_until.is_some_and(|u| u > now));
            if g.failures.len() >= MAX_TRACKED_CLIENTS {
                return Err(PairError::LockedOut { retry_after: policy.lockout });
            }
        }
        {
            let f = g.failures.entry(client).or_default();
            if let Some(until) = f.locked_until {
                if until > now {
                    return Err(PairError::LockedOut { retry_after: until - now });
                }
                f.locked_until = None;
            }
        }
        if !ct_eq(req.code.as_bytes(), g.code.as_bytes()) {
            g.failures_since_rotation += 1;
            if g.failures_since_rotation >= policy.rotate_after_failures {
                if let Ok(code) = random_code() {
                    log::warn!("companion hub: {} wrong pairing codes, rotating the code", g.failures_since_rotation);
                    g.code = code;
                    g.failures_since_rotation = 0;
                }
            }
            let f = g.failures.entry(client).or_default();
            f.consecutive += 1;
            if f.consecutive >= policy.max_failures {
                let factor = 1u32 << f.lockouts.min(16);
                let dur = policy.lockout.saturating_mul(factor).min(policy.max_lockout);
                f.lockouts += 1;
                f.consecutive = 0;
                f.locked_until = Some(now + dur);
                return Err(PairError::LockedOut { retry_after: dur });
            }
            return Err(PairError::WrongCode { remaining_attempts: policy.max_failures - f.consecutive });
        }
        g.failures.remove(&client);
        if g.devices.len() >= MAX_PAIRED_DEVICES {
            return Err(PairError::TooManyDevices);
        }
        let token = hex::encode(random_bytes::<32>().map_err(|e| PairError::Storage(e.message))?);
        let id = hex::encode(random_bytes::<8>().map_err(|e| PairError::Storage(e.message))?);
        let capabilities = req.capabilities.clone();
        let device = PairedDevice { id, name: req.device_name.trim().to_string(), device_kind: req.device_kind, paired_at: now_iso, capabilities };
        let mut devices = g.devices.clone();
        devices.push(StoredDevice { device: device.clone(), token_sha256: sha256_hex(&token) });
        self.persist(&devices).map_err(|e| PairError::Storage(e.message))?;
        g.devices = devices;
        Ok((token, device))
    }

    fn persist(&self, devices: &[StoredDevice]) -> CaptureResult<()> {
        let file = StoreFile { version: 1, devices: devices.to_vec() };
        let json = serde_json::to_vec_pretty(&file).map_err(|e| CaptureError::internal(e.to_string()))?;
        let tmp = self.store_path.with_extension("json.tmp");
        write_private(&tmp, &json)?;
        std::fs::rename(&tmp, &self.store_path).map_err(CaptureError::from)?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn kind_of(&self, id: &str) -> Option<crate::protocol::DeviceKind> {
        self.inner.lock().devices.iter().find(|d| d.device.id == id).map(|d| d.device.device_kind)
    }
}

fn write_private(path: &Path, data: &[u8]) -> CaptureResult<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(CaptureError::from)?;
    f.write_all(data).map_err(CaptureError::from)?;
    f.sync_all().map_err(CaptureError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DeviceKind;
    use std::net::Ipv4Addr;

    fn req(code: &str) -> PairRequest {
        PairRequest { code: code.into(), device_name: "Living room TV".into(), device_kind: DeviceKind::SamsungTizenTv, capabilities: None }
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            let c = random_code().unwrap();
            assert_eq!(c.len(), 6);
            assert!(c.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn pair_persist_reload_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let p = Pairing::load(dir.path(), PairingPolicy::default()).unwrap();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let (token, dev) = p.pair(ip, &req(&p.code()), "2026-01-01T00:00:00Z".into()).unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(p.authenticate(&token).unwrap().id, dev.id);
        assert!(p.authenticate(&"0".repeat(64)).is_none());
        let stored = std::fs::read_to_string(dir.path().join(STORE_FILE)).unwrap();
        assert!(!stored.contains(&token), "token must not be stored in clear text");
        assert!(stored.contains(&sha256_hex(&token)));
        let p2 = Pairing::load(dir.path(), PairingPolicy::default()).unwrap();
        assert_eq!(p2.kind_of(&dev.id), Some(DeviceKind::SamsungTizenTv));
        assert!(p2.authenticate(&token).is_some());
        assert!(p2.remove(&dev.id).unwrap());
        assert!(p2.authenticate(&token).is_none());
    }

    #[test]
    fn lockout_escalates() {
        let dir = tempfile::tempdir().unwrap();
        let policy = PairingPolicy { max_failures: 3, lockout: Duration::from_millis(200), max_lockout: Duration::from_secs(1), rotate_after_failures: 1000 };
        let p = Pairing::load(dir.path(), policy).unwrap();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        let other = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6));
        let wrong = if p.code() == "000000" { "111111" } else { "000000" };
        assert_eq!(p.pair(ip, &req(wrong), String::new()).unwrap_err(), PairError::WrongCode { remaining_attempts: 2 });
        assert_eq!(p.pair(ip, &req(wrong), String::new()).unwrap_err(), PairError::WrongCode { remaining_attempts: 1 });
        assert!(matches!(p.pair(ip, &req(wrong), String::new()), Err(PairError::LockedOut { .. })));
        // Even the right code is refused while locked out.
        assert!(matches!(p.pair(ip, &req(&p.code()), String::new()), Err(PairError::LockedOut { .. })));
        // Other clients are unaffected.
        assert!(p.pair(other, &req(&p.code()), String::new()).is_ok());
        std::thread::sleep(Duration::from_millis(250));
        for _ in 0..2 {
            assert!(matches!(p.pair(ip, &req(wrong), String::new()), Err(PairError::WrongCode { .. })));
        }
        match p.pair(ip, &req(wrong), String::new()) {
            Err(PairError::LockedOut { retry_after }) => assert!(retry_after >= Duration::from_millis(390), "second lockout doubles"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rotates_after_many_failures() {
        let dir = tempfile::tempdir().unwrap();
        let policy = PairingPolicy { max_failures: 1000, rotate_after_failures: 4, ..Default::default() };
        let p = Pairing::load(dir.path(), policy).unwrap();
        let original = p.code();
        let wrong = if original == "000000" { "111111" } else { "000000" };
        for i in 0..4u8 {
            let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 1, i));
            let _ = p.pair(ip, &req(wrong), String::new());
        }
        // Rotation may pick the same code with probability 1e-6; tolerate by checking counter reset.
        assert!(p.code() != original || p.inner.lock().failures_since_rotation == 0);
    }

    #[test]
    fn corrupt_store_is_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STORE_FILE), b"{not json").unwrap();
        let p = Pairing::load(dir.path(), PairingPolicy::default()).unwrap();
        assert!(p.devices().is_empty());
        assert!(dir.path().join("companion-paired-devices.json.corrupt").exists());
    }
}
