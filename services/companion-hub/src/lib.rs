//! CaptureOne companion hub: the LAN HTTP/WebSocket server the desktop app embeds so that
//! TV (Samsung Tizen, LG webOS) and mobile companion apps can browse/play recordings and
//! remote-control recording on the desktop.
//!
//! Protocol: `packages/capture-contracts/src/companion.ts` (mirrored in [`protocol`]).
//! The hub never claims capture capabilities for the companion: TVs only *control* the desktop
//! and play back files it recorded (e.g. from an HDMI capture card).
//!
//! Companion-facing differences from the desktop's own view of a recording:
//! `MediaMetadata.path` is rewritten to the hub-relative `/api/v1/recordings/:id/file` URL and
//! `thumbnailPath` to `/api/v1/recordings/:id/thumbnail` (absent when there is no thumbnail), so
//! companions never see local file-system paths.

pub mod protocol;

#[cfg(feature = "server")]
mod mdns;
#[cfg(feature = "server")]
pub mod net;
#[cfg(feature = "server")]
pub mod pairing;
#[cfg(feature = "server")]
mod server;

pub use protocol::*;

#[cfg(feature = "server")]
pub use server_api::*;

#[cfg(feature = "server")]
mod server_api {
    use crate::pairing::{Pairing, PairingPolicy};
    use crate::protocol::{HubEvent, HubInfo, PairedDevice, RemoteCommand, RemoteCommandResult};
    use crate::server::{router, AppState};
    use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, MediaMetadata, RecordingStatus};
    use std::net::{IpAddr, SocketAddr};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;
    use tokio::sync::{broadcast, watch};

    /// What the hub serves. Implemented by the desktop app on top of its library and media engine.
    /// Methods may block (they are called from a blocking thread pool, never from async workers).
    pub trait HubBackend: Send + Sync + 'static {
        fn hub_info(&self) -> HubInfo;
        fn list_recordings(&self) -> Vec<MediaMetadata>;
        fn recording_path(&self, id: &str) -> Option<PathBuf>;
        fn thumbnail_path(&self, id: &str) -> Option<PathBuf>;
        fn status(&self) -> Option<RecordingStatus>;
        fn command(&self, cmd: RemoteCommand) -> RemoteCommandResult;
    }

    pub struct HubConfig {
        /// `0.0.0.0` to accept LAN companions, `127.0.0.1` for local-only.
        pub bind: IpAddr,
        /// 0 picks a free port (tests); companions default to [`crate::COMPANION_DEFAULT_PORT`].
        pub port: u16,
        /// Where hashed pairing tokens are persisted (`companion-paired-devices.json`).
        pub state_dir: PathBuf,
    }

    /// Optional knobs beyond [`HubConfig`]; `HubHandle::start` uses the defaults.
    #[derive(Debug, Clone)]
    pub struct HubOptions {
        /// Advertise `_captureone._tcp` over mDNS (best effort).
        pub mdns: bool,
        pub pairing: PairingPolicy,
        /// Capacity of the per-connection event queue before a slow client is resynced.
        pub event_buffer: usize,
    }

    impl Default for HubOptions {
        fn default() -> Self {
            Self { mdns: true, pairing: PairingPolicy::default(), event_buffer: 64 }
        }
    }

    pub struct HubHandle {
        local_addr: SocketAddr,
        pairing: Arc<Pairing>,
        events: broadcast::Sender<HubEvent>,
        shutdown: Option<watch::Sender<bool>>,
        thread: Option<JoinHandle<()>>,
        mdns: Option<crate::mdns::Advertisement>,
    }

    impl HubHandle {
        /// Binds the listener (errors such as "port in use" are returned here) and serves on a
        /// dedicated thread running its own tokio runtime.
        pub fn start(backend: Arc<dyn HubBackend>, cfg: HubConfig) -> CaptureResult<HubHandle> {
            Self::start_with_options(backend, cfg, HubOptions::default())
        }

        pub fn start_with_options(backend: Arc<dyn HubBackend>, cfg: HubConfig, opts: HubOptions) -> CaptureResult<HubHandle> {
            let pairing = Arc::new(Pairing::load(&cfg.state_dir, opts.pairing.clone())?);
            let listener = std::net::TcpListener::bind(SocketAddr::new(cfg.bind, cfg.port)).map_err(|e| {
                let code = if e.kind() == std::io::ErrorKind::AddrInUse { CaptureErrorCode::DeviceBusy } else { CaptureErrorCode::Network };
                CaptureError::new(code, format!("companion hub cannot listen on {}:{}: {e}", cfg.bind, cfg.port))
            })?;
            listener.set_nonblocking(true).map_err(|e| CaptureError::new(CaptureErrorCode::Network, e.to_string()))?;
            let local_addr = listener.local_addr().map_err(|e| CaptureError::new(CaptureErrorCode::Network, e.to_string()))?;

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(16)
                .thread_name("companion-hub")
                .enable_all()
                .build()
                .map_err(|e| CaptureError::internal(format!("cannot create hub runtime: {e}")))?;

            let (events, _) = broadcast::channel(opts.event_buffer.max(4));
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            let state = AppState { backend: backend.clone(), pairing: pairing.clone(), events: events.clone(), shutdown: shutdown_rx.clone(), ws_clients: Default::default() };
            let app = router(state).into_make_service_with_connect_info::<SocketAddr>();

            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<CaptureResult<()>>();
            let thread = std::thread::Builder::new()
                .name("companion-hub".into())
                .spawn(move || {
                    runtime.block_on(async move {
                        let listener = match tokio::net::TcpListener::from_std(listener) {
                            Ok(l) => l,
                            Err(e) => {
                                let _ = ready_tx.send(Err(CaptureError::new(CaptureErrorCode::Network, e.to_string())));
                                return;
                            }
                        };
                        let _ = ready_tx.send(Ok(()));
                        let mut sig = shutdown_rx.clone();
                        let mut sig2 = shutdown_rx;
                        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                            let _ = sig.wait_for(|v| *v).await;
                        });
                        tokio::select! {
                            r = server => if let Err(e) = r { log::error!("companion hub server error: {e}") },
                            // Do not let a stuck connection hold shutdown forever.
                            _ = async { let _ = sig2.wait_for(|v| *v).await; tokio::time::sleep(Duration::from_secs(3)).await } => {}
                        }
                    });
                    runtime.shutdown_timeout(Duration::from_secs(1));
                })
                .map_err(|e| CaptureError::internal(format!("cannot spawn hub thread: {e}")))?;
            match ready_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let _ = thread.join();
                    return Err(e);
                }
                Err(_) => return Err(CaptureError::internal("companion hub did not start")),
            }

            let mdns = if opts.mdns { crate::mdns::advertise(&backend.hub_info(), cfg.bind, local_addr.port()) } else { None };
            log::info!("companion hub listening on {local_addr}");
            Ok(HubHandle { local_addr, pairing, events, shutdown: Some(shutdown_tx), thread: Some(thread), mdns })
        }

        /// Base URL companions should use, e.g. `http://192.168.1.20:47800`. When bound to the
        /// unspecified address this is the primary LAN IPv4 address.
        pub fn url(&self) -> String {
            let ip = if self.local_addr.ip().is_unspecified() { crate::net::primary_lan_ip().unwrap_or(self.local_addr.ip()) } else { self.local_addr.ip() };
            crate::net::http_url(ip, self.local_addr.port())
        }

        /// All base URLs the hub is reachable at (one per non-loopback interface address when
        /// bound to the unspecified address).
        pub fn urls(&self) -> Vec<String> {
            if self.local_addr.ip().is_unspecified() {
                let mut v: Vec<String> = crate::net::lan_ips().into_iter().map(|ip| crate::net::http_url(ip, self.local_addr.port())).collect();
                if v.is_empty() {
                    v.push(self.url());
                }
                v
            } else {
                vec![self.url()]
            }
        }

        pub fn local_addr(&self) -> SocketAddr {
            self.local_addr
        }

        pub fn pairing_code(&self) -> String {
            self.pairing.code()
        }

        /// Rotates the pairing code; the previous code stops working immediately. Existing
        /// tokens remain valid.
        pub fn regenerate_code(&self) -> String {
            match self.pairing.regenerate() {
                Ok(c) => c,
                Err(e) => {
                    // The OS RNG failing is not recoverable; keep the old code rather than panic.
                    log::error!("companion hub: cannot regenerate code: {e}");
                    self.pairing.code()
                }
            }
        }

        pub fn paired_devices(&self) -> Vec<PairedDevice> {
            self.pairing.devices()
        }

        /// Forgets a paired companion; its token is rejected from now on.
        pub fn unpair(&self, device_id: &str) -> CaptureResult<bool> {
            self.pairing.remove(device_id)
        }

        /// Sends an event to every connected `/api/v1/events` WebSocket.
        pub fn broadcast(&self, event: HubEvent) {
            let _ = self.events.send(event);
        }

        /// Stops accepting connections, closes WebSockets and joins the server thread.
        pub fn stop(mut self) {
            self.shutdown_now();
        }

        fn shutdown_now(&mut self) {
            if let Some(ad) = self.mdns.take() {
                ad.stop();
            }
            if let Some(tx) = self.shutdown.take() {
                let _ = tx.send(true);
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    impl Drop for HubHandle {
        fn drop(&mut self) {
            self.shutdown_now();
        }
    }
}
