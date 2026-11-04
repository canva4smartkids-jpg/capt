import { describe, expect, it } from "vitest";
import { estimateFileSizeBytes } from "@captureone/settings";
import type { CapabilityReport } from "@captureone/capture-contracts";
import {
  buildRecordingConfig,
  buildScreenshotRequest,
  captureSize,
  codecOptions,
  defaultDisplayId,
  effectiveCodec,
  estimate,
  formToSettings,
  initialRecordForm,
  microphoneGate,
  resolutionOptions,
  systemAudioGate,
  targetFromForm,
  windowCaptureGate,
} from "../src/lib/record-form";
import { capabilities, settings, sources } from "./fixtures";

const form = () => ({ ...initialRecordForm(settings), displayId: "disp-0" });

describe("capability gating", () => {
  it("offers only codecs with a verified encoder", () => {
    const opts = codecOptions(capabilities);
    expect(opts.find((o) => o.value === "h264")).toMatchObject({ disabled: false, encoder: "libx264" });
    expect(opts.find((o) => o.value === "h265")).toMatchObject({ disabled: true, reason: "Encoder present but failed verification" });
    expect(codecOptions(null).every((o) => o.disabled)).toBe(true);
  });

  it("prefers verified hardware encoders in the label", () => {
    const cap: CapabilityReport = { ...capabilities, encoders: [{ codec: "h264", name: "libx264", hardware: false, verified: true }, { codec: "h264", name: "h264_nvenc", hardware: true, verified: true }] };
    expect(codecOptions(cap)[0]).toMatchObject({ encoder: "h264_nvenc", hardware: true });
  });

  it("falls back to a usable codec, or null when none is verified", () => {
    expect(effectiveCodec("h265", capabilities)).toBe("h264");
    expect(effectiveCodec("h264", { ...capabilities, encoders: [] })).toBeNull();
  });

  it("gates system audio: unsupported disabled with reason; partial allowed with caveat", () => {
    expect(systemAudioGate(capabilities)).toEqual({ allowed: true, level: "partial", reason: "PulseAudio monitor sources only" });
    const { systemAudio: _s, ...rest } = capabilities.features;
    const g = systemAudioGate({ ...capabilities, features: rest });
    expect(g.allowed).toBe(false);
    expect(g.reason).toMatch(/not supported/);
    expect(systemAudioGate(null).allowed).toBe(false);
  });

  it("disables the microphone when no input device exists", () => {
    expect(microphoneGate(capabilities, []).allowed).toBe(false);
    expect(microphoneGate(capabilities, [{ id: "m", name: "Mic", kind: "input", isDefault: true }]).allowed).toBe(true);
    expect(windowCaptureGate({ ...capabilities, features: { ...capabilities.features, windowCapture: { level: "unsupported", reason: "Wayland" } } })).toMatchObject({ allowed: false, reason: "Wayland" });
  });

  it("never offers resolutions above the source", () => {
    const opts = resolutionOptions({ width: 1920, height: 1080 }, capabilities);
    expect(opts.find((o) => o.value === "1440")?.disabled).toBe(true);
    expect(opts.find((o) => o.value === "1080")?.disabled).toBe(false);
    expect(opts[0]?.label).toBe("Source (1920×1080)");
  });
});

describe("estimates", () => {
  it("uses the shared preset math (1080p30 H.264 medium + 160k audio)", () => {
    const f = form();
    f.audio = { ...f.audio, microphone: { ...f.audio.microphone, enabled: true } };
    const est = estimate(f, { width: 1920, height: 1080 });
    expect(est?.videoKbps).toBe(Math.round((1920 * 1080 * 30 * 0.08) / 1000));
    expect(est?.audioKbps).toBe(160);
    expect(est?.bytesPerMinute).toBe(estimateFileSizeBytes(est!.videoKbps, 160, 60));
    expect(est?.bytesAtLimit).toBeNull();
  });

  it("doubles audio for separate tracks, drops gated sources, scales resolution and projects the limit", () => {
    const f = form();
    f.audio = { ...f.audio, trackMode: "separate", microphone: { ...f.audio.microphone, enabled: true }, system: { ...f.audio.system, enabled: true } };
    f.video = { ...f.video, resolution: { mode: "height", height: 720 } };
    f.maxDurationMin = 10;
    const est = estimate(f, { width: 2560, height: 1440 });
    expect(est?.output).toEqual({ width: 1280, height: 720 });
    expect(est?.audioKbps).toBe(320);
    expect(est?.bytesAtLimit).toBe(estimateFileSizeBytes(est!.videoKbps, 320, 600));
    expect(estimate(f, { width: 2560, height: 1440 }, { allowSystem: false })?.audioKbps).toBe(160);
  });

  it("returns null without a source or without a custom bitrate", () => {
    expect(estimate(form(), null)).toBeNull();
    const f = form();
    f.video = { ...f.video, quality: "custom", bitrateKbps: undefined };
    expect(estimate(f, { width: 100, height: 100 })).toBeNull();
    f.video.bitrateKbps = 12_000;
    expect(estimate(f, { width: 100, height: 100 })?.videoKbps).toBe(12_000);
  });
});

describe("targets and config", () => {
  it("derives the capture target and size per tab", () => {
    const f = form();
    expect(targetFromForm(f)).toEqual({ kind: "display", displayId: "disp-0" });
    expect(captureSize(f, sources)).toEqual({ width: 1920, height: 1080 });
    f.sourceTab = "window";
    expect(targetFromForm(f)).toBeNull();
    f.windowId = "win-42";
    expect(captureSize(f, sources)).toEqual({ width: 1280, height: 800 });
    f.sourceTab = "region";
    f.region = { displayId: "disp-1", rect: { x: 10, y: 20, width: 640, height: 360 } };
    expect(targetFromForm(f)).toEqual({ kind: "region", displayId: "disp-1", rect: { x: 10, y: 20, width: 640, height: 360 } });
    expect(captureSize(f, sources)).toEqual({ width: 640, height: 360 });
    expect(defaultDisplayId(sources)).toBe("disp-0");
  });

  it("builds a schema-valid RecordingConfig with gating applied", () => {
    const f = form();
    f.video = { ...f.video, codec: "h265" };
    f.audio = { ...f.audio, system: { ...f.audio.system, enabled: true } };
    f.maxDurationMin = 5;
    const cap = { ...capabilities, features: { ...capabilities.features, systemAudio: { level: "unsupported" as const, reason: "no" } } };
    const r = buildRecordingConfig(f, settings, cap);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.config.video.codec).toBe("h264");
    expect(r.config.audio.system.enabled).toBe(false);
    expect(r.config.limits?.maxDurationSec).toBe(300);
    expect(r.config.output).toMatchObject({ directory: "/home/u/Videos/CaptureOne", container: "mp4" });
    expect(r.config.video.bitrateKbps).toBeUndefined();
  });

  it("reports actionable errors", () => {
    const noSource = { ...form(), displayId: null };
    expect(buildRecordingConfig(noSource, settings, capabilities)).toEqual({ ok: false, errors: ["Select a source to record."] });
    const noEncoder = buildRecordingConfig(form(), settings, { ...capabilities, encoders: [] });
    expect(noEncoder.ok ? [] : noEncoder.errors).toContain("No verified video encoder is available on this system.");
    const noDir = buildRecordingConfig(form(), { ...settings, outputDirectory: "" }, capabilities);
    expect(noDir.ok ? [] : noDir.errors).toContain("Choose an output folder in Settings.");
    const tiny = { ...form(), sourceTab: "region" as const, region: { displayId: "disp-0", rect: { x: 0, y: 0, width: 640, height: 360 } } };
    expect(buildRecordingConfig(tiny, settings, capabilities).ok).toBe(true);
    const custom = form();
    custom.video = { ...custom.video, quality: "custom", bitrateKbps: undefined };
    const r = buildRecordingConfig(custom, settings, capabilities);
    expect(r.ok ? [] : r.errors.join()).toMatch(/bitrateKbps/);
  });

  it("builds screenshot requests and writes back defaults", () => {
    const f = { ...form(), screenshotFormat: "jpeg" as const, jpegQuality: 70 };
    expect(buildScreenshotRequest(f, settings)).toMatchObject({ target: { kind: "display", displayId: "disp-0" }, format: "jpeg", jpegQuality: 70 });
    expect(buildScreenshotRequest({ ...f, displayId: null }, settings)).toBeNull();
    const next = formToSettings({ ...f, countdownSec: 10, maxDurationMin: 2 }, settings);
    expect(next).toMatchObject({ countdownSec: 10, screenshotFormat: "jpeg", jpegQuality: 70, maxDurationSec: 120 });
  });
});
