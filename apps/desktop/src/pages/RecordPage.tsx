import { Badge, Button, Card, Field, Icon, IconButton, KeyboardShortcut, SegmentedControl, Select, Slider, TextInput, Toggle, useToast } from "@captureone/design-system";
import type { CaptureSource, FrameRate, QualityPreset } from "@captureone/capture-contracts";
import { formatBytes, formatDuration } from "@captureone/settings";
import { useCallback, useEffect, useMemo, useState } from "react";
import { asFailure, invoke } from "../lib/ipc";
import {
  QUALITY_OPTIONS,
  buildRecordingConfig,
  buildScreenshotRequest,
  captureSize,
  codecOptions,
  cursorGate,
  defaultDisplayId,
  describeTarget,
  effectiveCodec,
  estimate,
  formToSettings,
  formatKbps,
  frameRateOptions,
  initialRecordForm,
  microphoneGate,
  pauseGate,
  regionCaptureGate,
  resolutionFromKey,
  resolutionKey,
  resolutionOptions,
  separateTracksGate,
  systemAudioGate,
  windowCaptureGate,
  type Countdown,
  type RecordForm,
  type ResolutionKey,
} from "../lib/record-form";
import { isBusy } from "../lib/recording-machine";
import { CountdownOverlay } from "../components/CountdownOverlay";
import { RecordingPanel } from "../components/RecordingPanel";
import { SourcePicker } from "../components/SourcePicker";
import { describeError, useApp } from "../state/app-state";

const DURATION_PRESETS = ["0", "5", "10", "15", "30", "60", "120"] as const;

export function RecordPage() {
  const app = useApp();
  const { settings, settingsLoaded, capabilities, audioDevices, recording } = app;
  const toast = useToast();
  const [form, setForm] = useState<RecordForm>(() => initialRecordForm(settings));
  const [sources, setSources] = useState<CaptureSource[]>([]);
  const [sourcesLoading, setSourcesLoading] = useState(true);
  const [sourcesError, setSourcesError] = useState<string | null>(null);
  const [thumbVersion, setThumbVersion] = useState(0);
  const [selectingRegion, setSelectingRegion] = useState(false);
  const [shooting, setShooting] = useState(false);
  const [freeDisk, setFreeDisk] = useState<number | null>(null);
  const [errors, setErrors] = useState<string[]>([]);

  // Re-seed the form once persisted settings arrive.
  useEffect(() => {
    if (settingsLoaded) setForm((f) => ({ ...initialRecordForm(settings), sourceTab: f.sourceTab, displayId: f.displayId, windowId: f.windowId, region: f.region }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsLoaded]);

  const patch = useCallback((p: Partial<RecordForm>) => {
    setErrors([]);
    setForm((f) => ({ ...f, ...p }));
  }, []);

  const loadSources = useCallback(async () => {
    setSourcesLoading(true);
    try {
      const list = await invoke("list_sources", { kinds: ["display", "window"] });
      setSources(list);
      setSourcesError(null);
      setForm((f) => {
        const displayOk = list.some((s) => s.kind === "display" && s.id === f.displayId);
        const windowOk = list.some((s) => s.kind === "window" && s.id === f.windowId);
        return { ...f, displayId: displayOk ? f.displayId : defaultDisplayId(list), windowId: windowOk ? f.windowId : null };
      });
    } catch (e) {
      setSourcesError(describeError(asFailure(e).toJSON()));
    } finally {
      setSourcesLoading(false);
      setThumbVersion((v) => v + 1);
    }
  }, []);

  useEffect(() => {
    void loadSources();
  }, [loadSources]);

  useEffect(() => {
    invoke("library_storage")
      .then((s) => setFreeDisk(s.freeDiskBytes))
      .catch(() => setFreeDisk(null));
  }, [app.libraryVersion]);

  const displays = useMemo(() => sources.filter((s) => s.kind === "display"), [sources]);
  const windows = useMemo(() => sources.filter((s) => s.kind === "window"), [sources]);
  const size = captureSize(form, sources);

  const sysGate = systemAudioGate(capabilities);
  const micGate = microphoneGate(capabilities, audioDevices);
  const sepGate = separateTracksGate(capabilities);
  const codecs = codecOptions(capabilities);
  const codec = effectiveCodec(form.video.codec, capabilities);
  const est = estimate({ ...form, video: { ...form.video, codec: codec ?? form.video.codec } }, size, { allowSystem: sysGate.allowed, allowMic: micGate.allowed });
  const busy = isBusy(recording);
  const inputs = (audioDevices ?? []).filter((d) => d.kind === "input");
  const loopbacks = (audioDevices ?? []).filter((d) => d.kind === "loopback");

  const start = async () => {
    const built = buildRecordingConfig(form, settings, capabilities);
    if (!built.ok) {
      setErrors(built.errors);
      return;
    }
    setErrors([]);
    await app.startRecording(built.config);
  };

  const screenshot = async () => {
    const req = buildScreenshotRequest(form, settings);
    if (!req) {
      setErrors([form.sourceTab === "region" ? "Select a region first." : "Select a source first."]);
      return;
    }
    setShooting(true);
    try {
      await invoke("take_screenshot", { request: req });
      // The backend also emits a `screenshot` event (toast + library refresh).
    } catch (e) {
      app.notifyError("Screenshot failed", e);
    } finally {
      setShooting(false);
    }
  };

  const selectRegion = async (displayId: string) => {
    setSelectingRegion(true);
    try {
      const rect = await invoke("select_region", { displayId });
      if (rect) patch({ region: { displayId, rect }, sourceTab: "region" });
    } catch (e) {
      app.notifyError("Region selection failed", e);
    } finally {
      setSelectingRegion(false);
    }
  };

  const saveDefaults = async () => {
    try {
      await app.saveSettings(formToSettings(form, settings));
      toast.push({ tone: "success", title: "Saved as default settings" });
    } catch (e) {
      app.notifyError("Could not save defaults", e);
    }
  };

  const durationValue = form.maxDurationMin === null ? "0" : DURATION_PRESETS.includes(String(form.maxDurationMin) as (typeof DURATION_PRESETS)[number]) ? String(form.maxDurationMin) : "custom";
  const minutesFit = est && freeDisk !== null && est.bytesPerMinute > 0 ? Math.floor(Math.max(0, freeDisk - settings.minFreeDiskMb * 1024 * 1024) / est.bytesPerMinute) : null;

  return (
    <div className="page page--record">
      <header className="page-header">
        <div>
          <h1>Record</h1>
          <p className="muted">Choose what to capture, tune quality, and start. Everything is encoded locally.</p>
        </div>
        <div className="page-header__actions">
          <Button icon="refresh" variant="ghost" onClick={() => void loadSources()} loading={sourcesLoading} disabled={busy}>
            Refresh sources
          </Button>
        </div>
      </header>

      <div className="record-grid">
        <div className="record-main">
          <Card title="Source" icon="monitor" subtitle={describeTarget(form, sources)}>
            <SourcePicker
              form={form}
              onChange={patch}
              displays={displays}
              windows={windows}
              loading={sourcesLoading}
              error={sourcesError}
              windowGate={windowCaptureGate(capabilities)}
              regionGate={regionCaptureGate(capabilities)}
              onSelectRegion={(id) => void selectRegion(id)}
              selectingRegion={selectingRegion}
              thumbVersion={thumbVersion}
              disabled={busy}
            />
          </Card>

          <div className="two-col">
            <Card title="Video" icon="video">
              <fieldset className="form-stack" disabled={busy}>
                <Field label="Codec" description={codec === null ? "No verified encoder: recording is unavailable." : codecs.find((c) => c.value === codec)?.hardware ? "Hardware encoder" : undefined}>
                  {({ id, describedBy }) => (
                    <Select
                      id={id}
                      aria-describedby={describedBy}
                      value={codec ?? form.video.codec}
                      options={codecs.map((c) => ({ value: c.value, label: c.label, disabled: c.disabled }))}
                      onValueChange={(v) => patch({ video: { ...form.video, codec: v } })}
                    />
                  )}
                </Field>
                <Field label="Resolution">
                  {({ id }) => (
                    <Select<ResolutionKey>
                      id={id}
                      value={resolutionKey(form.video.resolution)}
                      options={resolutionOptions(size, capabilities)}
                      onValueChange={(k) => patch({ video: { ...form.video, resolution: resolutionFromKey(k) } })}
                    />
                  )}
                </Field>
                <div className="field-row">
                  <span className="co-field__label" id="fps-label">
                    Frame rate
                  </span>
                  <SegmentedControl
                    label="Frame rate"
                    value={`${form.video.fps}` as `${FrameRate}`}
                    options={frameRateOptions(capabilities).map((o) => ({ value: o.value, label: `${o.value}`, disabled: o.disabled }))}
                    onChange={(v) => patch({ video: { ...form.video, fps: Number(v) as FrameRate } })}
                  />
                </div>
                <Field label="Quality">
                  {({ id }) => (
                    <Select<QualityPreset>
                      id={id}
                      value={form.video.quality}
                      options={QUALITY_OPTIONS}
                      onValueChange={(quality) => patch({ video: { ...form.video, quality, bitrateKbps: quality === "custom" ? (form.video.bitrateKbps ?? est?.videoKbps ?? 8000) : form.video.bitrateKbps } })}
                    />
                  )}
                </Field>
                {form.video.quality === "custom" && (
                  <Field label="Video bitrate" description="250 – 200,000 kbps">
                    {({ id, describedBy }) => (
                      <TextInput
                        id={id}
                        aria-describedby={describedBy}
                        type="number"
                        inputMode="numeric"
                        min={250}
                        max={200000}
                        step={250}
                        suffix="kbps"
                        value={form.video.bitrateKbps ?? ""}
                        onChange={(e) => patch({ video: { ...form.video, bitrateKbps: e.target.value === "" ? undefined : Math.round(Number(e.target.value)) } })}
                      />
                    )}
                  </Field>
                )}
                <Toggle
                  label="Show cursor"
                  checked={form.video.showCursor}
                  onChange={(showCursor) => patch({ video: { ...form.video, showCursor } })}
                  disabled={!cursorGate(capabilities).allowed}
                  disabledReason={cursorGate(capabilities).reason}
                />
              </fieldset>
            </Card>

            <Card title="Audio" icon="speaker">
              <fieldset className="form-stack" disabled={busy}>
                <Toggle
                  label="Microphone"
                  description={micGate.reason && micGate.allowed ? micGate.reason : undefined}
                  checked={form.audio.microphone.enabled && micGate.allowed}
                  disabled={!micGate.allowed}
                  disabledReason={micGate.reason}
                  onChange={(enabled) => patch({ audio: { ...form.audio, microphone: { ...form.audio.microphone, enabled } } })}
                />
                {form.audio.microphone.enabled && micGate.allowed && (
                  <div className="sub-controls">
                    <Field label="Input device">
                      {({ id }) => (
                        <Select
                          id={id}
                          value={form.audio.microphone.deviceId ?? ""}
                          options={[{ value: "", label: "System default" }, ...inputs.map((d) => ({ value: d.id, label: `${d.name}${d.isDefault ? " (default)" : ""}` }))]}
                          onValueChange={(v) => patch({ audio: { ...form.audio, microphone: { ...form.audio.microphone, deviceId: v || undefined } } })}
                        />
                      )}
                    </Field>
                    <Slider
                      label="Microphone gain"
                      min={0}
                      max={2}
                      step={0.05}
                      value={form.audio.microphone.gain}
                      formatValue={(v) => `${Math.round(v * 100)}%`}
                      onChange={(gain) => patch({ audio: { ...form.audio, microphone: { ...form.audio.microphone, gain } } })}
                    />
                  </div>
                )}
                <Toggle
                  label="System audio"
                  description={sysGate.allowed && sysGate.level === "partial" ? sysGate.reason : undefined}
                  checked={form.audio.system.enabled && sysGate.allowed}
                  disabled={!sysGate.allowed}
                  disabledReason={sysGate.reason}
                  onChange={(enabled) => patch({ audio: { ...form.audio, system: { ...form.audio.system, enabled } } })}
                />
                {form.audio.system.enabled && sysGate.allowed && (
                  <div className="sub-controls">
                    {loopbacks.length > 0 && (
                      <Field label="Output to capture">
                        {({ id }) => (
                          <Select
                            id={id}
                            value={form.audio.system.deviceId ?? ""}
                            options={[{ value: "", label: "Default output" }, ...loopbacks.map((d) => ({ value: d.id, label: d.name }))]}
                            onValueChange={(v) => patch({ audio: { ...form.audio, system: { ...form.audio.system, deviceId: v || undefined } } })}
                          />
                        )}
                      </Field>
                    )}
                    <Slider
                      label="System audio gain"
                      min={0}
                      max={2}
                      step={0.05}
                      value={form.audio.system.gain}
                      formatValue={(v) => `${Math.round(v * 100)}%`}
                      onChange={(gain) => patch({ audio: { ...form.audio, system: { ...form.audio.system, gain } } })}
                    />
                  </div>
                )}
                <div className="field-row">
                  <span className="co-field__label">Tracks</span>
                  <SegmentedControl
                    label="Audio track layout"
                    value={form.audio.trackMode === "separate" && sepGate.allowed ? "separate" : "mixed"}
                    options={[
                      { value: "mixed", label: "Mixed" },
                      { value: "separate", label: "Separate", disabled: !sepGate.allowed, title: sepGate.allowed ? "One AAC track per source" : sepGate.reason },
                    ]}
                    onChange={(trackMode) => patch({ audio: { ...form.audio, trackMode } })}
                  />
                </div>
                <Field label="Audio bitrate" inline>
                  {({ id }) => (
                    <Select
                      id={id}
                      value={`${form.audio.bitrateKbps}`}
                      options={(["96", "128", "160", "192", "256"] as const).map((v) => ({ value: v, label: `${v} kbps AAC` }))}
                      onValueChange={(v) => patch({ audio: { ...form.audio, bitrateKbps: Number(v) as 96 | 128 | 160 | 192 | 256 } })}
                    />
                  )}
                </Field>
              </fieldset>
            </Card>
          </div>

          <Card title="Options" icon="timer">
            <fieldset className="options-grid" disabled={busy}>
              <div className="field-row">
                <span className="co-field__label">Countdown</span>
                <SegmentedControl
                  label="Countdown"
                  value={`${form.countdownSec}`}
                  options={(["0", "3", "5", "10"] as const).map((v) => ({ value: v, label: v === "0" ? "Off" : `${v}s` }))}
                  onChange={(v) => patch({ countdownSec: Number(v) as Countdown })}
                />
              </div>
              <Field label="Stop automatically after">
                {({ id }) => (
                  <Select
                    id={id}
                    value={durationValue}
                    options={[
                      ...DURATION_PRESETS.map((v) => ({ value: v, label: v === "0" ? "No limit" : `${v} minutes` })),
                      ...(durationValue === "custom" ? [{ value: "custom", label: `${form.maxDurationMin} minutes` }] : []),
                    ]}
                    onValueChange={(v) => patch({ maxDurationMin: v === "0" || v === "custom" ? (v === "custom" ? form.maxDurationMin : null) : Number(v) })}
                  />
                )}
              </Field>
              <div className="field-row">
                <span className="co-field__label">Screenshot format</span>
                <SegmentedControl
                  label="Screenshot format"
                  value={form.screenshotFormat}
                  options={[
                    { value: "png", label: "PNG" },
                    { value: "jpeg", label: "JPEG", disabled: capabilities ? !capabilities.screenshotFormats.includes("jpeg") : false },
                  ]}
                  onChange={(screenshotFormat) => patch({ screenshotFormat })}
                />
              </div>
              {form.screenshotFormat === "jpeg" && (
                <Slider label="JPEG quality" min={1} max={100} value={form.jpegQuality} onChange={(jpegQuality) => patch({ jpegQuality })} />
              )}
            </fieldset>
          </Card>
        </div>

        <aside className="record-side" aria-label="Start recording">
          {recording.phase === "idle" || recording.phase === "requesting" ? (
            <Card className="start-card">
              <div className="estimate" aria-live="polite">
                <div className="estimate__big">
                  <span className="estimate__value">{est ? formatBytes(est.bytesPerMinute) : "—"}</span>
                  <span className="estimate__unit">per minute</span>
                </div>
                <dl className="stat-list">
                  <div>
                    <dt>Output</dt>
                    <dd>{est ? `${est.output.width}×${est.output.height}` : "—"}</dd>
                  </div>
                  <div>
                    <dt>Video</dt>
                    <dd>{est ? formatKbps(est.videoKbps) : "—"}</dd>
                  </div>
                  <div>
                    <dt>Audio</dt>
                    <dd>{est ? (est.audioKbps > 0 ? formatKbps(est.audioKbps) : "None") : "—"}</dd>
                  </div>
                  <div>
                    <dt>Total</dt>
                    <dd>{est ? formatKbps(est.totalKbps) : "—"}</dd>
                  </div>
                  {est?.bytesAtLimit != null && (
                    <div>
                      <dt>At limit</dt>
                      <dd>{formatBytes(est.bytesAtLimit)}</dd>
                    </div>
                  )}
                  {minutesFit !== null && (
                    <div>
                      <dt>Fits on disk</dt>
                      <dd>≈ {formatDuration(minutesFit * 60_000)}</dd>
                    </div>
                  )}
                </dl>
                <p className="estimate__note">Estimate from the shared encoding presets; real size depends on screen content.</p>
              </div>
              {errors.length > 0 && (
                <ul className="inline-alert inline-alert--danger" role="alert">
                  {errors.map((e) => (
                    <li key={e}>{e}</li>
                  ))}
                </ul>
              )}
              <Button variant="record" size="xl" block icon="record" loading={recording.phase === "requesting"} disabled={codec === null || !settingsLoaded} onClick={() => void start()}>
                Start recording
              </Button>
              <div className="start-card__shortcut">
                <KeyboardShortcut accelerator={settings.shortcuts.startStop} />
                <span className="muted">from anywhere</span>
              </div>
              <Button size="lg" block icon="camera" loading={shooting} onClick={() => void screenshot()}>
                Screenshot ({form.screenshotFormat.toUpperCase()})
              </Button>
              <Button variant="ghost" size="sm" block onClick={() => void saveDefaults()}>
                Save these settings as default
              </Button>
            </Card>
          ) : (
            <Card className="start-card">
              <RecordingPanel
                state={recording}
                pauseSupported={pauseGate(capabilities).allowed}
                onAction={(a) => void app.recordingAction(a)}
                onDismiss={() => app.dispatchRecording({ type: "dismiss" })}
              />
            </Card>
          )}
          <CapabilityNotes />
        </aside>
      </div>

      {recording.phase === "countdown" && recording.countdownRemaining !== null && (
        <CountdownOverlay remaining={recording.countdownRemaining} onCancel={() => void app.recordingAction("cancel")} />
      )}
    </div>
  );
}

function CapabilityNotes() {
  const { capabilities, capabilitiesError, reloadCapabilities } = useApp();
  const [open, setOpen] = useState(false);
  if (capabilitiesError) {
    return (
      <div className="inline-alert inline-alert--danger" role="alert">
        <Icon name="alert" size={16} />
        <div>
          <strong>Capture engine unavailable.</strong> {describeError(capabilitiesError)}
          <div>
            <Button size="sm" variant="ghost" icon="refresh" onClick={() => void reloadCapabilities()}>
              Retry
            </Button>
          </div>
        </div>
      </div>
    );
  }
  if (!capabilities) return null;
  const limited = Object.entries(capabilities.features).filter(([, f]) => f && f.level !== "native");
  return (
    <Card
      title="This device"
      icon="shield"
      actions={<IconButton icon={open ? "chevronUp" : "chevronDown"} label={open ? "Hide capability details" : "Show capability details"} size="sm" onClick={() => setOpen(!open)} aria-expanded={open} />}
    >
      <div className="cap-summary">
        {capabilities.encoders
          .filter((e) => e.verified)
          .map((e) => (
            <Badge key={e.name} tone="success" icon="check">
              {e.name}
            </Badge>
          ))}
        {capabilities.encoders.filter((e) => e.verified).length === 0 && <Badge tone="danger">No verified encoder</Badge>}
        <Badge tone="neutral">{limited.length} limited features</Badge>
      </div>
      {open && (
        <ul className="cap-list">
          {limited.map(([key, f]) => (
            <li key={key}>
              <Badge tone={f?.level === "partial" ? "warning" : "neutral"}>{f?.level}</Badge>
              <div>
                <strong>{key}</strong>
                {f?.reason && <p className="muted">{f.reason}</p>}
              </div>
            </li>
          ))}
          {capabilities.notes.map((n) => (
            <li key={n}>
              <Icon name="info" size={14} /> <span className="muted">{n}</span>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
