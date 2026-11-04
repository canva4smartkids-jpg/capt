import { Badge, Button, Card, Dialog, EmptyState, Icon, IconButton, Toggle, useToast, type IconName } from "@captureone/design-system";
import type { CaptureError, DeviceKind, DiscoveredDevice, HubStatus, TvRemoteKey } from "@captureone/capture-contracts";
import { useCallback, useEffect, useState, type KeyboardEvent } from "react";
import { relativeTime } from "../lib/format";
import { asFailure, invoke } from "../lib/ipc";
import { describeError, useApp } from "../state/app-state";

const KIND_LABEL: Record<DeviceKind, string> = {
  desktop: "Desktop",
  "android-phone": "Android phone",
  "android-tablet": "Android tablet",
  "android-tv": "Android TV",
  "google-tv": "Google TV",
  "samsung-tizen-tv": "Samsung TV (Tizen)",
  "lg-webos-tv": "LG TV (webOS)",
  "other-tv": "TV",
  unknown: "Unknown device",
};

const KIND_ICON: Partial<Record<DeviceKind, IconName>> = { desktop: "monitor", "android-phone": "devices", "android-tablet": "devices" };

/** What each kind of device can actually do with CaptureOne (shown verbatim; no over-promising). */
const KIND_ABILITIES: Partial<Record<DeviceKind, string>> = {
  "lg-webos-tv": "Remote keys after you accept the pairing prompt on the TV, plus an experimental one-shot screenshot through an undocumented LG call (may break with firmware updates; protected content stays black).",
  "samsung-tizen-tv": "Remote keys only. Samsung TVs expose no screenshot or recording API to third-party apps.",
  "android-tv": "Install CaptureOne for Android TV to record with per-session consent. Protected content is blanked by the OS.",
  "google-tv": "Install CaptureOne for Android TV to record with per-session consent. Protected content is blanked by the OS.",
  "other-tv": "Can be discovered only; no capture or control support.",
};

function isUnsupported(e: CaptureError | null): boolean {
  return e?.code === "UNSUPPORTED";
}

export function DevicesPage() {
  return (
    <div className="page page--devices">
      <header className="page-header">
        <div>
          <h1>Devices</h1>
          <p className="muted">Pair TV and phone companions with this computer, discover TVs on your network and control them.</p>
        </div>
      </header>
      <div className="devices-grid">
        <HubCard />
        <DiscoveryCard />
        <AbilitiesCard />
      </div>
    </div>
  );
}

function NotIntegrated({ error, what }: { error: CaptureError; what: string }) {
  return (
    <div className="inline-alert inline-alert--info" role="status">
      <Icon name="info" size={16} />
      <div>
        <strong>{what} is not available in this build.</strong>
        <p className="muted small">{error.message}</p>
      </div>
    </div>
  );
}

function HubCard() {
  const app = useApp();
  const toast = useToast();
  const [status, setStatus] = useState<HubStatus | null>(null);
  const [error, setError] = useState<CaptureError | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setStatus(await invoke("hub_status"));
      setError(null);
    } catch (e) {
      setError(asFailure(e).toJSON());
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (fn: () => Promise<HubStatus>) => {
    setBusy(true);
    try {
      setStatus(await fn());
      setError(null);
    } catch (e) {
      const err = asFailure(e).toJSON();
      if (isUnsupported(err)) setError(err);
      else app.notifyError("Companion hub", e);
    } finally {
      setBusy(false);
    }
  };

  const unsupported = isUnsupported(error);
  const enabled = status?.enabled ?? false;
  const code = status?.pairingCode ?? null;

  return (
    <Card title="Companion hub" icon="link" subtitle="Lets paired TVs and phones browse your recordings and start/stop recording on this computer. Off by default; LAN only.">
      <div className="form-stack">
        {error && (unsupported ? <NotIntegrated error={error} what="The companion hub" /> : <p className="text-danger">{describeError(error)}</p>)}
        <Toggle
          label="Allow companions on this network"
          description={enabled && status?.url ? `Listening at ${status.url}` : "When on, devices must enter the pairing code shown here."}
          checked={enabled}
          disabled={unsupported || busy || status === null}
          disabledReason={unsupported ? "Requires the companion hub service." : undefined}
          onChange={(v) => void run(() => invoke("hub_set_enabled", { enabled: v }))}
        />
        <div className={`pairing ${enabled && code ? "" : "is-disabled"}`}>
          <div>
            <div className="section-label">Pairing code</div>
            <div className="pairing__code" aria-label={code ? `Pairing code ${code.split("").join(" ")}` : "No pairing code"}>
              {(code ?? "––––––").padEnd(6, "–").split("").map((c, i) => (
                <span key={i}>{c}</span>
              ))}
            </div>
          </div>
          <div className="pairing__actions">
            <Button size="sm" icon="refresh" disabled={!enabled || unsupported || busy} onClick={() => void run(() => invoke("hub_regenerate_code"))}>
              New code
            </Button>
            {status?.url && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  navigator.clipboard.writeText(status.url ?? "").then(
                    () => toast.push({ tone: "success", title: "Address copied" }),
                    () => undefined,
                  )
                }
              >
                Copy address
              </Button>
            )}
          </div>
        </div>
        <div>
          <div className="section-label">Paired devices</div>
          {status && status.pairedDevices.length > 0 ? (
            <ul className="device-list device-list--compact">
              {status.pairedDevices.map((d) => (
                <li key={`${d.name}-${d.pairedAt}`}>
                  <Icon name={KIND_ICON[d.deviceKind as DeviceKind] ?? "tv"} size={18} />
                  <div>
                    <strong>{d.name}</strong>
                    <span className="muted small">
                      {KIND_LABEL[d.deviceKind as DeviceKind] ?? d.deviceKind} · paired {relativeTime(d.pairedAt)}
                    </span>
                  </div>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted small">No devices paired yet.</p>
          )}
        </div>
      </div>
    </Card>
  );
}

function DiscoveryCard() {
  const app = useApp();
  const toast = useToast();
  const [devices, setDevices] = useState<DiscoveredDevice[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<CaptureError | null>(null);
  const [remote, setRemote] = useState<DiscoveredDevice | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const scan = async () => {
    setScanning(true);
    try {
      setDevices(await invoke("discovery_scan", { timeoutMs: 4000 }));
      setError(null);
    } catch (e) {
      setError(asFailure(e).toJSON());
    } finally {
      setScanning(false);
    }
  };

  const pair = async (d: DiscoveredDevice) => {
    setBusyId(d.id);
    toast.push({ tone: "info", title: `Pairing with ${d.name}…`, message: "Accept the CaptureOne prompt on the TV within 60 seconds.", key: `pair-${d.id}`, timeout: 60_000 });
    try {
      const r = await invoke("tv_pair", { deviceId: d.id });
      toast.push({ tone: r.paired ? "success" : "warning", title: r.paired ? `Paired with ${d.name}` : `Not paired with ${d.name}`, message: r.message, key: `pair-${d.id}` });
    } catch (e) {
      app.notifyError(`Pairing with ${d.name} failed`, e);
    } finally {
      setBusyId(null);
    }
  };

  const screenshot = async (d: DiscoveredDevice) => {
    setBusyId(d.id);
    try {
      // Success is announced by the backend's `screenshot` event (toast + library refresh).
      await invoke("tv_screenshot", { deviceId: d.id });
    } catch (e) {
      app.notifyError(`Screenshot from ${d.name} failed`, e);
    } finally {
      setBusyId(null);
    }
  };

  return (
    <Card
      title="TVs on this network"
      icon="wifi"
      subtitle="Finds TVs via SSDP and mDNS. Nothing is sent to a device until you pair."
      actions={
        <Button size="sm" icon="search" loading={scanning} onClick={() => void scan()}>
          {devices ? "Scan again" : "Scan"}
        </Button>
      }
    >
      {error ? (
        isUnsupported(error) ? (
          <NotIntegrated error={error} what="Network discovery" />
        ) : (
          <p className="text-danger">{describeError(error)}</p>
        )
      ) : devices === null ? (
        <EmptyState icon="tv" title="Scan your network" description="Make sure the TV is on and connected to the same network as this computer." action={<Button variant="primary" icon="search" loading={scanning} onClick={() => void scan()}>Scan now</Button>} />
      ) : devices.length === 0 ? (
        <EmptyState icon="tv" title="No devices found" description="Check that the TV is on, on the same subnet, and that your firewall allows local discovery." />
      ) : (
        <ul className="device-list">
          {devices.map((d) => {
            const lg = d.controlProtocols.includes("lg-ssap");
            const controllable = lg || d.controlProtocols.includes("samsung-remote-ws") || d.controlProtocols.includes("android-tv-remote");
            return (
              <li key={d.id}>
                <Icon name={KIND_ICON[d.kind] ?? "tv"} size={22} />
                <div className="device-list__main">
                  <strong>{d.name}</strong>
                  <span className="muted small">
                    {KIND_LABEL[d.kind]}
                    {d.manufacturer ? ` · ${d.manufacturer}` : ""}
                    {d.model ? ` ${d.model}` : ""} · {d.address}
                  </span>
                  <div className="device-list__badges">
                    {d.controlProtocols.map((p) => (
                      <Badge key={p} tone="neutral">
                        {p}
                      </Badge>
                    ))}
                    {d.discoveredVia.map((v) => (
                      <Badge key={v} tone="info">
                        {v}
                      </Badge>
                    ))}
                  </div>
                  {KIND_ABILITIES[d.kind] && <p className="muted small">{KIND_ABILITIES[d.kind]}</p>}
                </div>
                <div className="device-list__actions">
                  <Button size="sm" disabled={!controllable || busyId === d.id} onClick={() => void pair(d)}>
                    Pair
                  </Button>
                  <Button size="sm" icon="dpad" disabled={!controllable} onClick={() => setRemote(d)}>
                    Remote
                  </Button>
                  {lg && (
                    <Button
                      size="sm"
                      icon="camera"
                      loading={busyId === d.id}
                      title="Experimental: uses an undocumented LG webOS call. Protected content is black."
                      onClick={() => void screenshot(d)}
                    >
                      Screenshot (experimental)
                    </Button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
      <RemoteDialog device={remote} onClose={() => setRemote(null)} />
    </Card>
  );
}

const ARROW_KEYS: Record<string, TvRemoteKey> = { ArrowUp: "UP", ArrowDown: "DOWN", ArrowLeft: "LEFT", ArrowRight: "RIGHT", Enter: "ENTER", Backspace: "BACK", Escape: "BACK" };

export function RemoteDialog({ device, onClose }: { device: DiscoveredDevice | null; onClose: () => void }) {
  const app = useApp();
  const [last, setLast] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  if (!device) return null;

  const send = async (key: TvRemoteKey) => {
    setLast(`${key}…`);
    try {
      await invoke("tv_send_key", { deviceId: device.id, key });
      setLast(key);
      setError(null);
    } catch (e) {
      const err = asFailure(e).toJSON();
      setError(describeError(err));
      if (!isUnsupported(err)) app.notifyError("Remote key failed", e);
    }
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const k = ARROW_KEYS[e.key];
    if (!k || e.key === "Escape") return;
    if ((e.target as HTMLElement).tagName === "BUTTON" && e.key === "Enter") return;
    e.preventDefault();
    void send(k);
  };

  const Key = ({ k, icon, label, className }: { k: TvRemoteKey; icon: IconName; label: string; className?: string }) => (
    <IconButton icon={icon} label={label} variant="secondary" size="lg" className={className} onClick={() => void send(k)} />
  );

  return (
    <Dialog open onClose={onClose} size="sm" title={`Remote · ${device.name}`} description="Arrow keys, Enter and Backspace also work while this dialog is focused.">
      <div className="remote" onKeyDown={onKey}>
        <div className="remote__row">
          <Key k="POWER" icon="power" label="Power" />
          <Key k="HOME" icon="home" label="Home" />
          <Key k="MENU" icon="menu" label="Menu" />
        </div>
        <div className="remote__dpad" role="group" aria-label="Directional pad">
          <Key k="UP" icon="chevronUp" label="Up" className="remote__up" />
          <Key k="LEFT" icon="chevronLeft" label="Left" className="remote__left" />
          <button type="button" className="remote__ok" aria-label="OK" onClick={() => void send("ENTER")}>
            OK
          </button>
          <Key k="RIGHT" icon="chevronRight" label="Right" className="remote__right" />
          <Key k="DOWN" icon="chevronDown" label="Down" className="remote__down" />
        </div>
        <div className="remote__row">
          <Key k="BACK" icon="back" label="Back" />
          <Key k="VOLUME_DOWN" icon="volumeDown" label="Volume down" />
          <Key k="MUTE" icon="mute" label="Mute" />
          <Key k="VOLUME_UP" icon="volumeUp" label="Volume up" />
        </div>
        <div className="remote__row">
          <Key k="PLAY" icon="play" label="Play" />
          <Key k="PAUSE" icon="pause" label="Pause" />
          <Key k="STOP" icon="stop" label="Stop" />
        </div>
        <p className="remote__status muted small" aria-live="polite">
          {error ? <span className="text-danger">{error}</span> : last ? (last.endsWith("…") ? `Sending ${last} (first key may wait for the TV's pairing prompt)` : `Sent ${last}`) : "Ready"}
        </p>
      </div>
    </Dialog>
  );
}

function AbilitiesCard() {
  return (
    <Card title="What TVs can and cannot do" icon="info" className="abilities-card">
      <ul className="abilities">
        <li>
          <Badge tone="danger">Not possible</Badge>
          <span>Samsung (Tizen) and LG (webOS) TVs do not let third-party apps record their screen. CaptureOne will not pretend otherwise.</span>
        </li>
        <li>
          <Badge tone="warning">Partial</Badge>
          <span>LG webOS: one-shot screenshots over the local SSAP API after you approve the TV's prompt. Streaming apps and HDMI inputs with DRM appear black.</span>
        </li>
        <li>
          <Badge tone="success">Native</Badge>
          <span>Android TV / Google TV: the CaptureOne Android TV app can record with your consent each session (MediaProjection). Protected video is blanked by the OS.</span>
        </li>
        <li>
          <Badge tone="info">Companion</Badge>
          <span>On any paired TV or phone you can browse and play recordings from this computer and start/stop recording here.</span>
        </li>
        <li>
          <Badge tone="neutral">Hardware</Badge>
          <span>To record a TV's picture, connect its HDMI output through a capture card to this computer. HDCP-protected sources cannot be recorded.</span>
        </li>
      </ul>
    </Card>
  );
}

