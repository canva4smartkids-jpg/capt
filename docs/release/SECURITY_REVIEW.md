# Security review (pre-release)

Branch `claude/gifted-fermi-0oqiw1`, 2026-11-04. Findings are fixed in this branch unless marked accepted.

## Scope

| Area | Components |
|---|---|
| Companion hub | `services/companion-hub` (axum HTTP/WebSocket on the LAN) |
| Desktop shell | `apps/desktop/src-tauri` (CSP, capabilities, asset scope, loopback media server, commands) |
| TV control | `services/device-discovery` (SSDP/UPnP, LG/Samsung control) |
| Media engine | `native/media-engine` (ffmpeg invocation, session recovery) |
| Android | `apps/android`, `apps/android-tv`, `native/android-capture` |
| TV web app | `apps/tv-web` |

## Method

Manual source review of each surface against its attack model (hostile LAN peer, hostile web page or webview content, hostile file in the output folder, hostile TV or UPnP responder), followed by targeted regression tests for every fix. No dynamic fuzzing or third-party pen test was done. Android code was reviewed statically; AGP builds only run in CI.

## Findings

| ID | Severity | Component | Description | Status | Test |
|---|---|---|---|---|---|
| S-01 | Medium | companion-hub | Unbounded number of `/events` WebSockets from one paired (or token-stealing) peer allowed memory/FD exhaustion. Now capped at 32, extra upgrades get 503. | Fixed | `integration.rs::security_ids_ranges_body_and_ws_cap` |
| S-02 | Medium | companion-hub pairing | Per-address failure table was unbounded (an IPv6 /64 gives unlimited source addresses). Capped at 4096 tracked clients; stale entries are pruned and, if still full, pairing attempts get a lockout response. | Fixed | covered by existing `pairing` unit tests (behaviour unchanged below the cap) |
| S-03 | Medium | desktop settings | `outputDirectory` was only checked for being non-empty. `/`, `/etc` or a relative path was accepted and then added to the asset-protocol scope (webview could read any file under it) and used for ffmpeg output. Now must be absolute, free of `.`/`..`, not a filesystem root or Unix system directory; hand-edited settings files fall back to the default. | Fixed | `settings::tests::output_directory_is_constrained` |
| S-04 | Medium | desktop library | `library_export` accepted any destination and overwrote it (e.g. `~/.bashrc`, autostart files, another recording). Destination must now be absolute, without `..`, keep the recording's extension, not be a directory/symlink, not be another library item. | Fixed | `library::tests::export_copies_and_refuses_self_overwrite` |
| S-05 | Medium | media-engine recovery | `session.json` is untrusted (user-writable folder): `sessionId` flowed into a temp file name (`../` traversal) and `audio[].file` was joined onto the session dir (absolute or `../` paths made ffmpeg read arbitrary files into the recovered recording). Ids are restricted to `[A-Za-z0-9_-]{1,64}` (fallback: sanitized dir name) and audio entries to bare file names. | Fixed | `recover::tests::untrusted_manifest_fields_are_constrained` |
| S-06 | Low | desktop media server (Linux) | Request line/headers were read without a size bound and with unlimited connection threads. Reads are now capped at 16 KiB and 32 concurrent connections. Already loopback-only, random 128-bit per-launch token, library ids only. | Fixed | `media_server::tests::serves_library_items_with_ranges_and_nothing_else` |
| S-07 | Low | device-discovery | `http_get` wrote the URL path verbatim into the request line; a TV-supplied `imageUri` containing CR/LF could inject headers into the request to that same TV. Paths with whitespace/control characters are refused. SSRF itself was already prevented (fetch only from the responding IP, 512 KiB cap, scheme check). | Fixed | `transport::tests::http_get_refuses_foreign_hosts_and_request_splitting` |
| S-08 | Low | android-capture / android-tv | Hub address field accepted public IPs and arbitrary host names, so the bearer token could be sent in clear text to the internet if mistyped. `HubAddress.parse` now only accepts private/loopback/link-local/CGNAT IPv4 and single-label or `.local`/`.lan`/`.home`/`.home.arpa`/`.internal` names. | Fixed | `CompanionClientTest` (host test, gradle `hostOnly`) |
| S-09 | Low | android-tv | `networkSecurityConfig` permits cleartext app-wide (Android cannot match IP ranges in domain configs). Mitigated by S-08 and by the hub being the only endpoint the app contacts. | Accepted (see residual) | n/a |
| S-10 | Info | desktop tests | `commands::tests::implements_every_contract_command` failed during the review while the concurrent device-capture work was registering `media_stream_url`; unrelated to the findings above. Re-run the desktop suite after that work lands. | Open (handed over) | existing test |

### Reviewed, no issue found

* **Hub auth**: tokens are 256-bit from the OS RNG, stored only as SHA-256 (verified in test: the token never appears in the store file), compared in constant time against every entry; store file is 0600. Pairing codes are uniform 6-digit, 5 failures per address trigger doubling lockout (30 s to 15 min), and 30 global failures rotate the code. All routes except `/info` and `/pair` require a token; WebSocket upgrade is authenticated by the same middleware.
* **Token in URLs**: `?token=` is required for `<video>`/`<img>`/WebSocket. The hub has no request logging layer and logs no URIs, so the token is not written to logs. Residual: it appears in browser history/devtools of the TV.
* **File routes**: recording ids resolve only through the library (`library.get(id)` with a bound SQL parameter); the path comes from the DB row, never from the request. Traversal-looking ids (`..%2F`, absolute, NUL) return 404. Range handling is `tower-http` `ServeFile`: invalid, overflowing, multi-range and unit-mismatch headers return 200/206/416 and never more than the file (tested).
* **Request limits**: 256 KiB body limit (413 tested), 64 KiB WebSocket message limit, keep-alive pings, lagging clients resynced.
* **CORS**: wildcard origin is intentional (TV apps run from `file://`/packaged origins); safe because auth is a bearer token, never cookies. Private-network preflight is allowed.
* **Bind default**: hub is off by default (`companionHubEnabled: false`); when enabled it binds `0.0.0.0` so LAN companions can reach it. Loopback media server binds `127.0.0.1` only.
* **Tauri**: CSP is `default-src 'self'`, no `unsafe-inline`/`unsafe-eval`, `connect-src` limited to IPC, `object-src 'none'`, `form-action 'none'`. Capabilities are per window: indicator and region windows get only the commands they need; `main` has no opener/shell/fs plugin permissions (only `dialog:allow-open/save`); `library_reveal` resolves a library id (no user path) and uses the opener plugin's reveal-in-folder, no process spawn. Asset scope is limited to `$VIDEO/CaptureOne/**` plus the (now validated) output directory. `library_rename` sanitizes the title (`/`, `\`, `..` cannot survive) and keeps the file in its folder (tested with `../../etc/cron.d/evil`). `tv_*` commands only accept device ids from the last scan, never raw IPs.
* **Discovery**: LOCATION is fetched only from the responding IP; XML parsed with `roxmltree` (node cap 50k, entity expansion limits, 512 KiB body cap). TLS verification is replaced by a per-connection pinned verifier used only for the TV's IP (TOFU on first pair, pin mismatch fails); there is no global accept-invalid setting.
* **ffmpeg**: always `Command::new(ffmpeg).args(vec)`, never a shell; file names go through `sanitize_stem`/`persist_no_clobber` (hard-link, never overwrites); input paths are absolute paths rather than URLs.
* **Android**: only `MainActivity`/`TvActivity` (launcher) and the quick-settings tile (protected by `BIND_QUICK_SETTINGS_TILE`) are exported; `CaptureService` is not exported; no FileProvider or ContentProvider is declared (sharing uses MediaStore URIs with `FLAG_GRANT_READ_URI_PERMISSION`); all PendingIntents use `FLAG_IMMUTABLE`; all SQLite access uses parameterised `query`/`delete`/`insertWithOnConflict` (the only `execSQL` calls are static DDL); `allowBackup=false`. The phone app has no network security config, so cleartext is blocked by default on API 28+.
* **tv-web**: all hub strings are rendered with `createElement`/`textContent`/`setAttribute`; there is no `innerHTML`, `eval` or `document.write`. Hub-provided paths are appended to the stored `baseUrl`, so a hostile `path` cannot redirect the token to another host. The token is stored in `localStorage` (TV browsers offer nothing better).

## Residual risks

1. **Companion protocol v1 is plain HTTP on the LAN.** Anyone on the same network segment can sniff the pairing code exchange and later bearer tokens, and can impersonate the hub. TLS (with pinning of a self-signed hub certificate, as already done for TVs) is planned; until then the hub is opt-in and should be used on trusted networks only.
2. Pairing code stays valid after a successful pairing until it rotates (30 global failures or manual regenerate). Consider rotating after every success.
3. The hub has no global cap on concurrent plain HTTP connections or read timeouts for slow clients (only the WebSocket cap). Impact is limited to the LAN and to availability.
4. `?token=` URLs are visible in TV browser history/dev tools and any LAN proxy.
5. Session intermediates (`.captureone-sessions/`) and the recording library are written with the user's default umask; they inherit the output folder's permissions.
6. TV pairing uses TOFU for TLS certificates; a hostile device present at first pairing could be pinned.
7. Android TV allows cleartext app-wide (S-09); the hub address validator (S-08) is the compensating control.
8. Device-capture ids (v4l2 / dshow / AVFoundation) are being added concurrently and were not reviewed; they must be passed to ffmpeg as vector arguments and validated against enumerated devices.
9. No dependency vulnerability scan (`cargo audit`, `pnpm audit`, OSV) was possible offline; run it in CI before release.
