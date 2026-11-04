import {
  Badge,
  Button,
  Card,
  Field,
  Icon,
  KeyboardShortcut,
  SegmentedControl,
  Select,
  Slider,
  Spinner,
  TextInput,
  Toggle,
  acceleratorFromKeyEvent,
  useToast,
  type Tone,
} from "@captureone/design-system";
import type { Diagnostics, PermissionKind, PermissionStatus, QualityPreset } from "@captureone/capture-contracts";
import { AppSettings, defaultSettings, type ShortcutBindings } from "@captureone/settings";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { asFailure, invoke } from "../lib/ipc";
import { QUALITY_OPTIONS, codecOptions, resolutionFromKey, resolutionKey, systemAudioGate, type ResolutionKey } from "../lib/record-form";
import { SHORTCUT_NAMES, TEMPLATE_TOKENS, fileNamePreview, hasErrors, settingsEqual, validateSettings } from "../lib/settings-form";
import { describeError, useApp } from "../state/app-state";

export function SettingsPage() {
  const app = useApp();
  const toast = useToast();
  const [draft, setDraft] = useState<AppSettings>(app.settings);
  const [saving, setSaving] = useState(false);
  const templateRef = useRef<HTMLInputElement>(null);

  useEffect(() => setDraft(app.settings), [app.settings]);

  const errors = useMemo(() => validateSettings(draft), [draft]);
  const dirty = !settingsEqual(draft, app.settings);
  const set = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => setDraft((d) => ({ ...d, [key]: value }));

  const chooseFolder = async () => {
    try {
      const picked = await open({ directory: true, multiple: false, defaultPath: draft.outputDirectory || undefined, title: "Choose output folder" });
      if (typeof picked === "string") set("outputDirectory", picked);
    } catch (e) {
      app.notifyError("Could not open the folder picker", e);
    }
  };

  const insertToken = (token: string) => {
    const el = templateRef.current;
    const t = draft.fileNameTemplate;
    const start = el?.selectionStart ?? t.length;
    const end = el?.selectionEnd ?? t.length;
    set("fileNameTemplate", t.slice(0, start) + token + t.slice(end));
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(start + token.length, start + token.length);
    });
  };

  const save = async () => {
    if (hasErrors(errors)) return;
    setSaving(true);
    try {
      await app.saveSettings(AppSettings.parse(draft));
      toast.push({ tone: "success", title: "Settings saved" });
    } catch (e) {
      app.notifyError("Could not save settings", e);
    } finally {
      setSaving(false);
    }
  };

  const codecs = codecOptions(app.capabilities);
  const sys = systemAudioGate(app.capabilities);

  return (
    <div className="page page--settings">
      <header className="page-header">
        <div>
          <h1>Settings</h1>
          <p className="muted">Defaults for new recordings, shortcuts, appearance and system access.</p>
        </div>
      </header>

      <div className="settings-grid">
        <Card title="Output" icon="folder">
          <div className="form-stack">
            <Field label="Output folder" error={errors.outputDirectory} description="Recordings, screenshots and recovery files are written here.">
              {({ id, describedBy, invalid }) => (
                <div className="input-with-button">
                  <TextInput id={id} aria-describedby={describedBy} aria-invalid={invalid} value={draft.outputDirectory} onChange={(e) => set("outputDirectory", e.target.value)} spellCheck={false} />
                  <Button icon="folder" onClick={() => void chooseFolder()}>
                    Choose…
                  </Button>
                </div>
              )}
            </Field>
            <Field label="File name template" error={errors.fileNameTemplate} description={<>Preview: <code>{fileNamePreview(draft.fileNameTemplate)}</code></>}>
              {({ id, describedBy, invalid }) => (
                <TextInput ref={templateRef} id={id} aria-describedby={describedBy} aria-invalid={invalid} value={draft.fileNameTemplate} onChange={(e) => set("fileNameTemplate", e.target.value)} spellCheck={false} />
              )}
            </Field>
            <div className="token-chips" aria-label="Insert template token">
              {TEMPLATE_TOKENS.map((t) => (
                <button key={t.token} type="button" className="chip" title={t.description} onClick={() => insertToken(t.token)}>
                  <code>{t.token}</code> {t.description}
                </button>
              ))}
            </div>
          </div>
        </Card>

        <Card title="Default video" icon="video">
          <div className="form-stack">
            <Field label="Codec" inline>
              {({ id }) => (
                <Select id={id} value={draft.video.codec} options={codecs.map((c) => ({ value: c.value, label: c.label, disabled: c.disabled }))} onValueChange={(codec) => set("video", { ...draft.video, codec })} />
              )}
            </Field>
            <Field label="Resolution" inline>
              {({ id }) => (
                <Select<ResolutionKey>
                  id={id}
                  value={resolutionKey(draft.video.resolution)}
                  options={[
                    { value: "source", label: "Source" },
                    { value: "2160", label: "4K (2160p)" },
                    { value: "1440", label: "1440p" },
                    { value: "1080", label: "1080p" },
                    { value: "720", label: "720p" },
                    { value: "480", label: "480p" },
                  ]}
                  onValueChange={(k) => set("video", { ...draft.video, resolution: resolutionFromKey(k) })}
                />
              )}
            </Field>
            <div className="field-row">
              <span className="co-field__label">Frame rate</span>
              <SegmentedControl
                label="Default frame rate"
                value={`${draft.video.fps}` as "24" | "30" | "60"}
                options={(["24", "30", "60"] as const).map((v) => ({ value: v, label: v }))}
                onChange={(v) => set("video", { ...draft.video, fps: Number(v) as 24 | 30 | 60 })}
              />
            </div>
            <Field label="Quality" inline>
              {({ id }) => (
                <Select<QualityPreset>
                  id={id}
                  value={draft.video.quality}
                  options={QUALITY_OPTIONS}
                  onValueChange={(quality) => set("video", { ...draft.video, quality, bitrateKbps: quality === "custom" ? (draft.video.bitrateKbps ?? 8000) : draft.video.bitrateKbps })}
                />
              )}
            </Field>
            {draft.video.quality === "custom" && (
              <Field label="Bitrate" inline error={errors["video.bitrateKbps"]}>
                {({ id, invalid }) => (
                  <TextInput
                    id={id}
                    type="number"
                    suffix="kbps"
                    aria-invalid={invalid}
                    value={draft.video.bitrateKbps ?? ""}
                    onChange={(e) => set("video", { ...draft.video, bitrateKbps: e.target.value === "" ? undefined : Math.round(Number(e.target.value)) })}
                  />
                )}
              </Field>
            )}
            <Field label="Hardware acceleration" inline description="Auto uses a verified hardware encoder when available.">
              {({ id, describedBy }) => (
                <Select
                  id={id}
                  aria-describedby={describedBy}
                  value={draft.video.hardwareAcceleration}
                  options={[
                    { value: "auto", label: "Auto" },
                    { value: "prefer", label: "Prefer hardware" },
                    { value: "off", label: "Software only" },
                  ]}
                  onValueChange={(hardwareAcceleration) => set("video", { ...draft.video, hardwareAcceleration })}
                />
              )}
            </Field>
            <Toggle label="Show cursor" checked={draft.video.showCursor} onChange={(showCursor) => set("video", { ...draft.video, showCursor })} />
          </div>
        </Card>

        <Card title="Default audio" icon="speaker">
          <div className="form-stack">
            <Toggle label="Record microphone" checked={draft.audio.microphone.enabled} onChange={(enabled) => set("audio", { ...draft.audio, microphone: { ...draft.audio.microphone, enabled } })} />
            <Toggle
              label="Record system audio"
              checked={draft.audio.system.enabled && sys.allowed}
              disabled={!sys.allowed}
              disabledReason={sys.reason}
              description={sys.allowed ? sys.reason : undefined}
              onChange={(enabled) => set("audio", { ...draft.audio, system: { ...draft.audio.system, enabled } })}
            />
            <div className="field-row">
              <span className="co-field__label">Tracks</span>
              <SegmentedControl label="Default track layout" value={draft.audio.trackMode} options={[{ value: "mixed", label: "Mixed" }, { value: "separate", label: "Separate" }]} onChange={(trackMode) => set("audio", { ...draft.audio, trackMode })} />
            </div>
            <div className="field-row">
              <span className="co-field__label">Sample rate</span>
              <SegmentedControl
                label="Sample rate"
                value={`${draft.audio.sampleRate}` as "44100" | "48000"}
                options={[{ value: "44100", label: "44.1 kHz" }, { value: "48000", label: "48 kHz" }]}
                onChange={(v) => set("audio", { ...draft.audio, sampleRate: Number(v) as 44100 | 48000 })}
              />
            </div>
            <Field label="AAC bitrate" inline>
              {({ id }) => (
                <Select
                  id={id}
                  value={`${draft.audio.bitrateKbps}`}
                  options={(["96", "128", "160", "192", "256"] as const).map((v) => ({ value: v, label: `${v} kbps` }))}
                  onValueChange={(v) => set("audio", { ...draft.audio, bitrateKbps: Number(v) as 96 | 128 | 160 | 192 | 256 })}
                />
              )}
            </Field>
          </div>
        </Card>

        <Card title="Recording & screenshots" icon="timer">
          <div className="form-stack">
            <div className="field-row">
              <span className="co-field__label">Countdown</span>
              <SegmentedControl
                label="Default countdown"
                value={`${draft.countdownSec}` as "0" | "3" | "5" | "10"}
                options={(["0", "3", "5", "10"] as const).map((v) => ({ value: v, label: v === "0" ? "Off" : `${v}s` }))}
                onChange={(v) => set("countdownSec", Number(v) as 0 | 3 | 5 | 10)}
              />
            </div>
            <Field label="Default duration limit" inline error={errors.maxDurationSec} description="Empty = no limit">
              {({ id, describedBy, invalid }) => (
                <TextInput
                  id={id}
                  type="number"
                  min={1}
                  suffix="min"
                  aria-describedby={describedBy}
                  aria-invalid={invalid}
                  value={draft.maxDurationSec ? Math.round(draft.maxDurationSec / 60) : ""}
                  onChange={(e) => set("maxDurationSec", e.target.value === "" ? undefined : Math.round(Number(e.target.value) * 60))}
                />
              )}
            </Field>
            <Field label="Stop when free disk is below" inline error={errors.minFreeDiskMb}>
              {({ id, invalid }) => (
                <TextInput id={id} type="number" min={0} suffix="MB" aria-invalid={invalid} value={draft.minFreeDiskMb} onChange={(e) => set("minFreeDiskMb", Math.max(0, Math.round(Number(e.target.value) || 0)))} />
              )}
            </Field>
            <div className="field-row">
              <span className="co-field__label">Screenshot format</span>
              <SegmentedControl label="Screenshot format" value={draft.screenshotFormat} options={[{ value: "png", label: "PNG" }, { value: "jpeg", label: "JPEG" }]} onChange={(v) => set("screenshotFormat", v)} />
            </div>
            {draft.screenshotFormat === "jpeg" && <Slider label="JPEG quality" min={1} max={100} value={draft.jpegQuality} onChange={(v) => set("jpegQuality", v)} />}
            <Toggle label="Show recording indicator" description="Small always-on-top bar with timer, pause and stop while recording." checked={draft.showRecordingIndicator} onChange={(v) => set("showRecordingIndicator", v)} />
            <Toggle label="Minimize window when recording starts" checked={draft.minimizeOnRecord} onChange={(v) => set("minimizeOnRecord", v)} />
          </div>
        </Card>

        <Card title="Keyboard shortcuts" icon="keyboard" subtitle="Global shortcuts work even when CaptureOne is in the background.">
          <ShortcutEditor value={draft.shortcuts} errors={errors} onChange={(shortcuts) => set("shortcuts", shortcuts)} />
          <div className="inapp-keys">
            <h3 className="section-label">In the app</h3>
            <ul>
              <li>
                <KeyboardShortcut accelerator="CommandOrControl+1" /> – <KeyboardShortcut accelerator="CommandOrControl+4" /> Switch pages
              </li>
              <li>
                <kbd className="co-kbd">/</kbd> Search the library
              </li>
              <li>
                <kbd className="co-kbd">F2</kbd> Rename · <kbd className="co-kbd">Del</kbd> Delete (focused library item)
              </li>
              <li>
                <kbd className="co-kbd">Esc</kbd> Cancel countdown, close dialogs, cancel region selection
              </li>
            </ul>
          </div>
        </Card>

        <Card title="Appearance" icon="sun">
          <div className="field-row">
            <span className="co-field__label">Theme</span>
            <SegmentedControl
              label="Theme"
              value={draft.theme}
              options={[
                { value: "system", label: "System" },
                { value: "light", label: "Light", icon: "sun" },
                { value: "dark", label: "Dark", icon: "moon" },
              ]}
              onChange={(theme) => set("theme", theme)}
            />
          </div>
        </Card>

        <PermissionsCard />
        <DiagnosticsCard />
      </div>

      {(dirty || saving) && (
        <div className="save-bar" role="region" aria-label="Unsaved changes">
          <span>
            <Icon name="info" size={16} /> {hasErrors(errors) ? "Fix the highlighted fields to save." : "You have unsaved changes."}
          </span>
          <div className="save-bar__actions">
            <Button variant="ghost" onClick={() => setDraft(defaultSettings(app.settings.outputDirectory))}>
              Reset to defaults
            </Button>
            <Button onClick={() => setDraft(app.settings)}>Discard</Button>
            <Button variant="primary" icon="check" loading={saving} disabled={hasErrors(errors)} onClick={() => void save()}>
              Save changes
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

const SHORTCUT_ORDER: (keyof ShortcutBindings)[] = ["startStop", "pauseResume", "screenshot", "regionScreenshot"];

function ShortcutEditor({ value, errors, onChange }: { value: ShortcutBindings; errors: Partial<Record<string, string>>; onChange: (v: ShortcutBindings) => void }) {
  const [capturing, setCapturing] = useState<keyof ShortcutBindings | null>(null);
  const defaults = defaultSettings().shortcuts;
  const onKey = (key: keyof ShortcutBindings) => (e: KeyboardEvent<HTMLButtonElement>) => {
    if (capturing !== key) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setCapturing(null);
      return;
    }
    if (e.key === "Backspace" || e.key === "Delete") {
      onChange({ ...value, [key]: "" });
      setCapturing(null);
      return;
    }
    const accel = acceleratorFromKeyEvent(e.nativeEvent);
    if (accel) {
      onChange({ ...value, [key]: accel });
      setCapturing(null);
    }
  };
  return (
    <ul className="shortcut-list">
      {SHORTCUT_ORDER.map((k) => {
        const err = errors[`shortcuts.${k}`];
        return (
          <li key={k} className={err ? "is-invalid" : undefined}>
            <div className="shortcut-list__label">
              <span>{SHORTCUT_NAMES[k]}</span>
              {err && (
                <span className="text-danger small" role="alert">
                  {err}
                </span>
              )}
            </div>
            <button
              type="button"
              className={`shortcut-capture ${capturing === k ? "is-capturing" : ""}`}
              aria-label={`${SHORTCUT_NAMES[k]} shortcut: ${value[k] || "not set"}. Press to change.`}
              onClick={() => setCapturing(capturing === k ? null : k)}
              onKeyDown={onKey(k)}
              onBlur={() => capturing === k && setCapturing(null)}
            >
              {capturing === k ? <span className="muted">Press keys… (Esc cancels, Del clears)</span> : <KeyboardShortcut accelerator={value[k]} />}
            </button>
            <Button size="sm" variant="ghost" disabled={value[k] === defaults[k]} onClick={() => onChange({ ...value, [k]: defaults[k] })}>
              Default
            </Button>
          </li>
        );
      })}
    </ul>
  );
}

const PERMISSIONS: { kind: PermissionKind; label: string; why: string }[] = [
  { kind: "screenCapture", label: "Screen recording", why: "Needed to capture displays and windows." },
  { kind: "microphone", label: "Microphone", why: "Needed to record your voice." },
  { kind: "systemAudio", label: "System audio", why: "Needed to record sound playing on this computer." },
];

const STATE_TONE: Record<PermissionStatus["state"], Tone> = {
  granted: "success",
  denied: "danger",
  prompt: "warning",
  restricted: "danger",
  "not-applicable": "neutral",
  "per-session": "info",
};

function PermissionsCard() {
  const app = useApp();
  const [statuses, setStatuses] = useState<Partial<Record<PermissionKind, PermissionStatus | { error: string }>>>({});
  const [busy, setBusy] = useState<PermissionKind | null>(null);

  const load = useCallback(async () => {
    const entries = await Promise.all(
      PERMISSIONS.map(async (p) => {
        try {
          return [p.kind, await invoke("get_permission_status", { kind: p.kind })] as const;
        } catch (e) {
          return [p.kind, { error: describeError(asFailure(e).toJSON()) }] as const;
        }
      }),
    );
    setStatuses(Object.fromEntries(entries));
  }, []);

  useEffect(() => {
    void load();
    // Re-check when the user comes back from System Settings.
    const onFocus = () => void load();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [load]);

  const request = async (kind: PermissionKind) => {
    setBusy(kind);
    try {
      const st = await invoke("request_permission", { kind });
      setStatuses((s) => ({ ...s, [kind]: st }));
    } catch (e) {
      app.notifyError("Permission request failed", e);
    } finally {
      setBusy(null);
    }
  };

  const openSettings = async (kind: PermissionKind) => {
    try {
      await invoke("open_permission_settings", { kind });
    } catch (e) {
      app.notifyError("Could not open system settings", e);
    }
  };

  const isMac = app.capabilities?.platform === "macos";
  return (
    <Card title="Permissions" icon="shield" subtitle={isMac ? "macOS asks once per app; changes may require restarting CaptureOne." : "Current access to capture devices."} actions={<Button size="sm" variant="ghost" icon="refresh" onClick={() => void load()}>Re-check</Button>}>
      <ul className="perm-list">
        {PERMISSIONS.map((p) => {
          const st = statuses[p.kind];
          return (
            <li key={p.kind}>
              <div className="perm-list__text">
                <strong>{p.label}</strong>
                <span className="muted small">{st && "error" in st ? st.error : (st?.message ?? p.why)}</span>
              </div>
              <div className="perm-list__actions">
                {!st ? (
                  <Spinner size={14} />
                ) : "error" in st ? (
                  <Badge tone="danger">Unknown</Badge>
                ) : (
                  <>
                    <Badge tone={STATE_TONE[st.state]} dot>
                      {st.state === "not-applicable" ? "Not required" : st.state}
                    </Badge>
                    {st.state === "prompt" && (
                      <Button size="sm" loading={busy === p.kind} onClick={() => void request(p.kind)}>
                        Request
                      </Button>
                    )}
                    {(st.requiresSystemSettings || st.state === "denied") && (
                      <Button size="sm" variant="ghost" icon="settings" onClick={() => void openSettings(p.kind)}>
                        Open System Settings
                      </Button>
                    )}
                  </>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </Card>
  );
}

function DiagnosticsCard() {
  const toast = useToast();
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    invoke("get_diagnostics")
      .then(setDiag)
      .catch((e) => setError(describeError(asFailure(e).toJSON())));
  }, []);
  const copy = async () => {
    if (!diag) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(diag, null, 2));
      toast.push({ tone: "success", title: "Diagnostics copied" });
    } catch {
      toast.push({ tone: "warning", title: "Clipboard unavailable" });
    }
  };
  const rows: [string, string | null | undefined][] = diag
    ? [
        ["App version", diag.appVersion],
        ["Platform", `${diag.platform} · ${diag.osVersion}`],
        ["FFmpeg", diag.ffmpegPath ? `${diag.ffmpegVersion ?? "unknown version"} — ${diag.ffmpegPath}` : null],
        ["FFprobe", diag.ffprobePath],
        ["Data folder", diag.dataDirectory],
        ["Logs", diag.logDirectory],
        ["Library database", diag.databasePath],
      ]
    : [];
  return (
    <Card title="Diagnostics" icon="info" actions={<Button size="sm" variant="ghost" disabled={!diag} onClick={() => void copy()}>Copy</Button>}>
      {error ? (
        <p className="text-danger">{error}</p>
      ) : !diag ? (
        <Spinner size={16} label="Loading diagnostics" />
      ) : (
        <dl className="diag-list">
          {rows.map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd className={v ? "mono break" : "text-danger"}>{v ?? "Not found — recording is unavailable until FFmpeg is installed."}</dd>
            </div>
          ))}
        </dl>
      )}
    </Card>
  );
}
