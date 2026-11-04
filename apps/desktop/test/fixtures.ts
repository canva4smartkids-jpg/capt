import type { CapabilityReport, CaptureSource, MediaMetadata } from "@captureone/capture-contracts";
import { defaultSettings, type AppSettings } from "@captureone/settings";

export const capabilities: CapabilityReport = {
  schemaVersion: 1,
  platform: "linux",
  osName: "Ubuntu",
  osVersion: "24.04",
  appVersion: "0.1.0",
  generatedAt: "2026-11-04T12:00:00Z",
  features: {
    screenshot: { level: "native" },
    displayCapture: { level: "native" },
    windowCapture: { level: "native" },
    regionCapture: { level: "native" },
    videoRecording: { level: "native" },
    pauseResume: { level: "native" },
    cursorCapture: { level: "native" },
    microphoneAudio: { level: "native" },
    systemAudio: { level: "partial", reason: "PulseAudio monitor sources only" },
    separateAudioTracks: { level: "native" },
  },
  encoders: [
    { codec: "h264", name: "libx264", hardware: false, verified: true },
    { codec: "h265", name: "libx265", hardware: false, verified: false },
  ],
  frameRates: [24, 30, 60],
  screenshotFormats: ["png", "jpeg"],
  notes: [],
};

export const sources: CaptureSource[] = [
  { id: "disp-0", kind: "display", name: "Display 1", width: 1920, height: 1080, x: 0, y: 0, isPrimary: true, scaleFactor: 1 },
  { id: "disp-1", kind: "display", name: "Display 2", width: 2560, height: 1440, x: 1920, y: 0, scaleFactor: 1 },
  { id: "win-42", kind: "window", name: "Editor", width: 1280, height: 800, appName: "code" },
];

export const settings: AppSettings = defaultSettings("/home/u/Videos/CaptureOne");

export const video: MediaMetadata = {
  id: "rec-1",
  kind: "video",
  path: "/home/u/Videos/CaptureOne/CaptureOne 2026-11-04 12-00-00.mp4",
  fileName: "CaptureOne 2026-11-04 12-00-00.mp4",
  title: "",
  createdAt: "2026-11-04T12:00:00Z",
  sizeBytes: 12_345_678,
  width: 1920,
  height: 1080,
  durationMs: 61_000,
  fps: 30,
  videoCodec: "h264",
  videoBitrateKbps: 4800,
  audioTracks: [],
  container: "mp4",
  validation: { valid: true, checkedAt: "2026-11-04T12:01:02Z", tool: "ffprobe 6.1.1", issues: [] },
};

export const shot: MediaMetadata = {
  id: "shot-1",
  kind: "screenshot",
  path: "/home/u/Videos/CaptureOne/Shot.png",
  fileName: "Shot.png",
  title: "Login bug",
  createdAt: "2026-11-04T13:00:00Z",
  sizeBytes: 250_000,
  width: 1920,
  height: 1080,
  audioTracks: [],
  imageFormat: "png",
  container: "png",
};
