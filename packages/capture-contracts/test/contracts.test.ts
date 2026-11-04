import { describe, expect, it } from "vitest";
import {
  CapabilityReport,
  CaptureEvent,
  CaptureFailure,
  CaptureTarget,
  RECORDING_TRANSITIONS,
  RecordingConfig,
  RecordingState,
  PairRequest,
  canTransition,
  featureLevel,
  isUsable,
  toCaptureError,
  userMessage,
  CaptureErrorCode,
} from "../src/index.js";

const baseConfig = {
  target: { kind: "display", displayId: "0" },
  video: { codec: "h264", resolution: { mode: "source" }, fps: 30, quality: "medium" },
  audio: { microphone: { enabled: true }, system: { enabled: false } },
  output: { directory: "/tmp" },
};

describe("RecordingConfig", () => {
  it("applies defaults", () => {
    const cfg = RecordingConfig.parse(baseConfig);
    expect(cfg.countdownSec).toBe(3);
    expect(cfg.audio.sampleRate).toBe(48000);
    expect(cfg.audio.bitrateKbps).toBe(160);
    expect(cfg.audio.trackMode).toBe("mixed");
    expect(cfg.video.keyframeIntervalSec).toBe(2);
    expect(cfg.limits.minFreeDiskMb).toBe(500);
    expect(cfg.output.container).toBe("mp4");
  });
  it("rejects unsupported fps", () => {
    expect(RecordingConfig.safeParse({ ...baseConfig, video: { ...baseConfig.video, fps: 25 } }).success).toBe(false);
  });
  it("requires bitrate for custom quality", () => {
    const r = RecordingConfig.safeParse({ ...baseConfig, video: { ...baseConfig.video, quality: "custom" } });
    expect(r.success).toBe(false);
    const ok = RecordingConfig.safeParse({ ...baseConfig, video: { ...baseConfig.video, quality: "custom", bitrateKbps: 8000 } });
    expect(ok.success).toBe(true);
  });
  it("validates region targets", () => {
    expect(CaptureTarget.safeParse({ kind: "region", displayId: "1", rect: { x: 0, y: 0, width: 0, height: 10 } }).success).toBe(false);
    expect(CaptureTarget.safeParse({ kind: "region", displayId: "1", rect: { x: 10, y: 20, width: 300, height: 200 } }).success).toBe(true);
  });
});

describe("state machine", () => {
  it("covers every state", () => {
    for (const s of RecordingState.options) expect(RECORDING_TRANSITIONS[s]).toBeDefined();
  });
  it("only references valid states", () => {
    for (const targets of Object.values(RECORDING_TRANSITIONS)) for (const t of targets) expect(RecordingState.options).toContain(t);
  });
  it("enforces legal transitions", () => {
    expect(canTransition("recording", "paused")).toBe(true);
    expect(canTransition("paused", "recording")).toBe(true);
    expect(canTransition("idle", "paused")).toBe(false);
    expect(canTransition("completed", "recording")).toBe(false);
  });
  it("every state can reach idle", () => {
    for (const s of RecordingState.options) {
      const seen = new Set<string>([s]);
      const q = [s];
      while (q.length) {
        const cur = q.shift()!;
        for (const n of RECORDING_TRANSITIONS[cur]) if (!seen.has(n)) (seen.add(n), q.push(n));
      }
      expect(seen.has("idle")).toBe(true);
    }
  });
});

describe("capabilities", () => {
  const report = CapabilityReport.parse({
    schemaVersion: 1,
    platform: "tizen",
    osName: "Tizen",
    osVersion: "6.5",
    appVersion: "0.1.0",
    generatedAt: new Date().toISOString(),
    features: { displayCapture: { level: "external-hardware", reason: "No public screen capture API for third-party Tizen apps" }, remoteControl: { level: "native" } },
    encoders: [],
    frameRates: [],
    screenshotFormats: [],
  });
  it("treats missing as unsupported", () => {
    expect(featureLevel(report, "videoRecording")).toBe("unsupported");
    expect(isUsable(report, "displayCapture")).toBe(false);
    expect(isUsable(report, "remoteControl")).toBe(true);
  });
});

describe("errors", () => {
  it("normalises unknown values", () => {
    expect(toCaptureError(new Error("boom")).code).toBe("INTERNAL");
    expect(toCaptureError(JSON.stringify({ code: "STORAGE_FULL", message: "x", recoverable: false })).code).toBe("STORAGE_FULL");
    expect(toCaptureError({ code: "PERMISSION_DENIED", message: "no", recoverable: false }).code).toBe("PERMISSION_DENIED");
    expect(toCaptureError(new CaptureFailure({ code: "DRM_PROTECTED", message: "p", recoverable: false })).code).toBe("DRM_PROTECTED");
    expect(toCaptureError(42).code).toBe("INTERNAL");
  });
  it("has a user message for every code", () => {
    for (const c of CaptureErrorCode.options) expect(userMessage(c).length).toBeGreaterThan(5);
  });
});

describe("events and companion", () => {
  it("parses events", () => {
    expect(CaptureEvent.safeParse({ type: "countdown", sessionId: "s", remainingSec: 2 }).success).toBe(true);
    expect(CaptureEvent.safeParse({ type: "nope" }).success).toBe(false);
  });
  it("validates pair code", () => {
    expect(PairRequest.safeParse({ code: "123456", deviceName: "TV", deviceKind: "lg-webos-tv" }).success).toBe(true);
    expect(PairRequest.safeParse({ code: "12a456", deviceName: "TV", deviceKind: "lg-webos-tv" }).success).toBe(false);
  });
});
