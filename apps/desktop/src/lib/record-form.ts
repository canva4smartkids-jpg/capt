/**
 * Pure state logic behind the Record page: capability gating, option lists, live
 * size estimates (shared preset math from @captureone/settings) and conversion of
 * the form into a contract `RecordingConfig`.
 */
import {
  RecordingConfig,
  featureLevel,
  isUsable,
  type AudioSettings,
  type CapabilityReport,
  type CaptureSource,
  type CaptureTarget,
  type FeatureKey,
  type FrameRate,
  type ImageFormat,
  type QualityPreset,
  type RecordingConfigInput,
  type Rect,
  type ResolutionSetting,
  type ScreenshotRequestInput,
  type SupportLevel,
  type VideoCodec,
  type VideoSettings,
} from "@captureone/capture-contracts";
import type { AudioDevice } from "@captureone/capture-contracts";
import { estimateFileSizeBytes, resolveOutputSize, videoBitrateKbps, type AppSettings, type Size } from "@captureone/settings";

export type SourceTab = "display" | "window" | "region";
export type Countdown = 0 | 3 | 5 | 10;

export interface RegionSelection {
  displayId: string;
  rect: Rect;
}

export interface RecordForm {
  sourceTab: SourceTab;
  displayId: string | null;
  windowId: string | null;
  region: RegionSelection | null;
  video: VideoSettings;
  audio: AudioSettings;
  /** Minutes; null = unlimited. */
  maxDurationMin: number | null;
  countdownSec: Countdown;
  screenshotFormat: ImageFormat;
  jpegQuality: number;
}

export function initialRecordForm(settings: AppSettings): RecordForm {
  return {
    sourceTab: "display",
    displayId: null,
    windowId: null,
    region: null,
    video: { ...settings.video },
    audio: {
      ...settings.audio,
      microphone: { ...settings.audio.microphone },
      system: { ...settings.audio.system },
    },
    maxDurationMin: settings.maxDurationSec ? Math.max(1, Math.round(settings.maxDurationSec / 60)) : null,
    countdownSec: settings.countdownSec,
    screenshotFormat: settings.screenshotFormat,
    jpegQuality: settings.jpegQuality,
  };
}

// ---------------------------------------------------------------- gating

export interface Gate {
  allowed: boolean;
  level: SupportLevel;
  /** Shown next to the control when not plainly native (why disabled, or the caveat for partial). */
  reason?: string;
}

export function gate(cap: CapabilityReport | null, key: FeatureKey, fallbackReason?: string): Gate {
  if (!cap) return { allowed: false, level: "unsupported", reason: "Capabilities are still loading." };
  const level = featureLevel(cap, key);
  const reason = cap.features[key]?.reason ?? (level === "native" ? undefined : fallbackReason);
  return { allowed: isUsable(cap, key), level, ...(reason ? { reason } : {}) };
}

export const systemAudioGate = (cap: CapabilityReport | null) => gate(cap, "systemAudio", "System audio capture is not supported on this platform.");
export const microphoneGate = (cap: CapabilityReport | null, devices: readonly AudioDevice[] | null): Gate => {
  const g = gate(cap, "microphoneAudio", "Microphone capture is not supported on this platform.");
  if (g.allowed && devices && devices.filter((d) => d.kind === "input").length === 0) {
    return { allowed: false, level: g.level, reason: "No microphone was found." };
  }
  return g;
};
export const separateTracksGate = (cap: CapabilityReport | null) => gate(cap, "separateAudioTracks", "Separate audio tracks are not supported.");
export const windowCaptureGate = (cap: CapabilityReport | null) => gate(cap, "windowCapture", "Window capture is not supported on this platform.");
export const regionCaptureGate = (cap: CapabilityReport | null) => gate(cap, "regionCapture", "Region capture is not supported on this platform.");
export const pauseGate = (cap: CapabilityReport | null) => gate(cap, "pauseResume", "Pause/resume is not supported.");
export const cursorGate = (cap: CapabilityReport | null) => gate(cap, "cursorCapture", "Cursor capture is not supported.");

export interface CodecOption {
  value: VideoCodec;
  label: string;
  disabled: boolean;
  reason?: string;
  encoder?: string;
  hardware?: boolean;
}

/** H.264/H.265 are offered only when the engine verified an encoder with a real test encode. */
export function codecOptions(cap: CapabilityReport | null): CodecOption[] {
  const codecs: VideoCodec[] = ["h264", "h265"];
  return codecs.map((codec) => {
    const name = codec === "h264" ? "H.264 (AVC)" : "H.265 (HEVC)";
    const verified = (cap?.encoders ?? []).filter((e) => e.codec === codec && e.verified);
    const best = verified.find((e) => e.hardware) ?? verified[0];
    if (!best) {
      const listed = (cap?.encoders ?? []).some((e) => e.codec === codec);
      return { value: codec, label: `${name} — unavailable`, disabled: true, reason: listed ? "Encoder present but failed verification" : "No verified encoder" };
    }
    return { value: codec, label: `${name} · ${best.name}${best.hardware ? " (hardware)" : ""}`, disabled: false, encoder: best.name, hardware: best.hardware };
  });
}

/** Returns a codec the engine can actually use: the requested one if verified, else the first verified, else null. */
export function effectiveCodec(requested: VideoCodec, cap: CapabilityReport | null): VideoCodec | null {
  const opts = codecOptions(cap);
  if (opts.find((o) => o.value === requested && !o.disabled)) return requested;
  return opts.find((o) => !o.disabled)?.value ?? null;
}

export function frameRateOptions(cap: CapabilityReport | null): { value: `${FrameRate}`; label: string; disabled: boolean }[] {
  const allowed = new Set<number>(cap?.frameRates ?? [24, 30, 60]);
  return ([24, 30, 60] as const).map((f) => ({ value: `${f}` as const, label: `${f} fps`, disabled: !allowed.has(f) }));
}

export type ResolutionKey = "source" | "2160" | "1440" | "1080" | "720" | "480";

export function resolutionKey(r: ResolutionSetting): ResolutionKey {
  if (r.mode === "height") return `${r.height}` as ResolutionKey;
  return "source";
}

export function resolutionFromKey(k: ResolutionKey): ResolutionSetting {
  return k === "source" ? { mode: "source" } : { mode: "height", height: Number(k) as 2160 | 1440 | 1080 | 720 | 480 };
}

/** Resolution choices; heights above the source are disabled (the engine never upscales). */
export function resolutionOptions(source: Size | null, cap: CapabilityReport | null): { value: ResolutionKey; label: string; disabled: boolean }[] {
  const max = cap?.maxRecordingResolution;
  const out: { value: ResolutionKey; label: string; disabled: boolean }[] = [
    { value: "source", label: source ? `Source (${source.width}×${source.height})` : "Source resolution", disabled: false },
  ];
  for (const h of [2160, 1440, 1080, 720, 480] as const) {
    const tooBig = source ? h > source.height : false;
    const overCap = max ? h > max.height : false;
    out.push({ value: `${h}` as ResolutionKey, label: `${h === 2160 ? "4K (2160p)" : `${h}p`}${tooBig ? " — larger than source" : ""}`, disabled: tooBig || overCap });
  }
  return out;
}

export const QUALITY_OPTIONS: { value: QualityPreset; label: string }[] = [
  { value: "low", label: "Low — smallest files" },
  { value: "medium", label: "Medium — balanced" },
  { value: "high", label: "High" },
  { value: "ultra", label: "Ultra — near lossless" },
  { value: "custom", label: "Custom bitrate" },
];

// ---------------------------------------------------------------- sources & targets

export function targetFromForm(form: RecordForm): CaptureTarget | null {
  switch (form.sourceTab) {
    case "display":
      return form.displayId ? { kind: "display", displayId: form.displayId } : null;
    case "window":
      return form.windowId ? { kind: "window", windowId: form.windowId } : null;
    case "region":
      return form.region ? { kind: "region", displayId: form.region.displayId, rect: form.region.rect } : null;
    default: {
      const never: never = form.sourceTab;
      return never;
    }
  }
}

/** Physical pixel size of what will be captured, or null when no source is selected. */
export function captureSize(form: RecordForm, sources: readonly CaptureSource[]): Size | null {
  switch (form.sourceTab) {
    case "display": {
      const s = sources.find((x) => x.kind === "display" && x.id === form.displayId);
      return s ? { width: s.width, height: s.height } : null;
    }
    case "window": {
      const s = sources.find((x) => x.kind === "window" && x.id === form.windowId);
      return s ? { width: s.width, height: s.height } : null;
    }
    case "region":
      return form.region ? { width: form.region.rect.width, height: form.region.rect.height } : null;
    default:
      return null;
  }
}

export function describeTarget(form: RecordForm, sources: readonly CaptureSource[]): string {
  const t = targetFromForm(form);
  if (!t) return "No source selected";
  if (t.kind === "display") return sources.find((s) => s.id === t.displayId)?.name ?? "Display";
  if (t.kind === "window") return sources.find((s) => s.id === t.windowId)?.name ?? "Window";
  if (t.kind === "region") return `Region ${t.rect.width}×${t.rect.height} on ${sources.find((s) => s.id === t.displayId)?.name ?? "display"}`;
  return "Device";
}

/** Picks a sensible default display (primary first). */
export function defaultDisplayId(sources: readonly CaptureSource[]): string | null {
  const displays = sources.filter((s) => s.kind === "display");
  return (displays.find((d) => d.isPrimary) ?? displays[0])?.id ?? null;
}

// ---------------------------------------------------------------- estimates

export interface Estimate {
  output: Size;
  videoKbps: number;
  audioKbps: number;
  totalKbps: number;
  bytesPerMinute: number;
  /** Projected size at the duration limit, when one is set. */
  bytesAtLimit: number | null;
}

export function audioKbpsTotal(audio: AudioSettings, allowSystem = true, allowMic = true): number {
  const enabled = (audio.microphone.enabled && allowMic ? 1 : 0) + (audio.system.enabled && allowSystem ? 1 : 0);
  if (enabled === 0) return 0;
  return audio.trackMode === "separate" ? audio.bitrateKbps * enabled : audio.bitrateKbps;
}

export function estimate(form: RecordForm, source: Size | null, opts: { allowSystem?: boolean; allowMic?: boolean } = {}): Estimate | null {
  if (!source || source.width <= 0 || source.height <= 0) return null;
  if (form.video.quality === "custom" && !form.video.bitrateKbps) return null;
  const output = resolveOutputSize(source, form.video.resolution);
  const videoKbps = videoBitrateKbps(output, form.video.fps, form.video.codec, form.video.quality, form.video.bitrateKbps);
  const audioKbps = audioKbpsTotal(form.audio, opts.allowSystem ?? true, opts.allowMic ?? true);
  const bytesPerMinute = estimateFileSizeBytes(videoKbps, audioKbps, 60);
  const bytesAtLimit = form.maxDurationMin ? estimateFileSizeBytes(videoKbps, audioKbps, form.maxDurationMin * 60) : null;
  return { output, videoKbps, audioKbps, totalKbps: videoKbps + audioKbps, bytesPerMinute, bytesAtLimit };
}

export function formatKbps(kbps: number): string {
  return kbps >= 1000 ? `${(kbps / 1000).toFixed(kbps >= 10_000 ? 0 : 1)} Mbps` : `${Math.round(kbps)} kbps`;
}

// ---------------------------------------------------------------- config building

export type BuildResult = { ok: true; config: RecordingConfigInput } | { ok: false; errors: string[] };

/**
 * Converts the form into a RecordingConfig, applying capability gating (unusable
 * audio sources are switched off, the codec falls back to a verified one) and
 * validating with the contract schema.
 */
export function buildRecordingConfig(form: RecordForm, settings: AppSettings, cap: CapabilityReport | null): BuildResult {
  const errors: string[] = [];
  const target = targetFromForm(form);
  if (!target) errors.push(form.sourceTab === "region" ? "Select a region first." : "Select a source to record.");
  const codec = effectiveCodec(form.video.codec, cap);
  if (!codec) errors.push("No verified video encoder is available on this system.");
  if (!settings.outputDirectory) errors.push("Choose an output folder in Settings.");
  if (errors.length > 0 || !target || !codec) return { ok: false, errors };

  const sys = systemAudioGate(cap);
  const mic = gate(cap, "microphoneAudio");
  const sep = separateTracksGate(cap);
  const video: VideoSettings = { ...form.video, codec };
  if (video.quality !== "custom") delete video.bitrateKbps;
  const config: RecordingConfigInput = {
    target,
    video,
    audio: {
      ...form.audio,
      microphone: { ...form.audio.microphone, enabled: form.audio.microphone.enabled && mic.allowed },
      system: { ...form.audio.system, enabled: form.audio.system.enabled && sys.allowed },
      trackMode: form.audio.trackMode === "separate" && sep.allowed ? "separate" : "mixed",
    },
    output: { directory: settings.outputDirectory, fileNameTemplate: settings.fileNameTemplate, container: "mp4" },
    limits: {
      ...(form.maxDurationMin ? { maxDurationSec: Math.round(form.maxDurationMin * 60) } : {}),
      minFreeDiskMb: settings.minFreeDiskMb,
    },
    countdownSec: form.countdownSec,
  };
  const parsed = RecordingConfig.safeParse(config);
  if (!parsed.success) {
    return { ok: false, errors: parsed.error.issues.map((i) => `${i.path.join(".") || "config"}: ${i.message}`) };
  }
  return { ok: true, config };
}

export function buildScreenshotRequest(form: RecordForm, settings: AppSettings): ScreenshotRequestInput | null {
  const target = targetFromForm(form);
  if (!target) return null;
  return {
    target,
    format: form.screenshotFormat,
    jpegQuality: form.jpegQuality,
    outputDirectory: settings.outputDirectory || undefined,
    fileNameTemplate: settings.fileNameTemplate,
    showCursor: form.video.showCursor,
  };
}

/** Writes the record form's choices back into settings defaults ("remember these settings"). */
export function formToSettings(form: RecordForm, settings: AppSettings): AppSettings {
  return {
    ...settings,
    video: { ...form.video },
    audio: { ...form.audio },
    countdownSec: form.countdownSec,
    screenshotFormat: form.screenshotFormat,
    jpegQuality: form.jpegQuality,
    maxDurationSec: form.maxDurationMin ? form.maxDurationMin * 60 : undefined,
  };
}
