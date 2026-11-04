/**
 * Honest capability report for the TV itself. Samsung Tizen and LG webOS give third-party apps
 * no screen-capture API, so the TV app is a companion: it controls a CaptureOne desktop and plays
 * back what the desktop recorded (for the TV picture: through an HDMI capture card).
 */
import type { CapabilityReport, FeatureKey, FeatureSupport, Platform } from "@captureone/capture-contracts";
import type { PlatformId } from "./keys.js";

export interface TvDeviceInfo {
  manufacturer: string;
  model?: string;
  osName: string;
  osVersion: string;
  firmware?: string;
  resolution?: { width: number; height: number };
  /** Which APIs supplied the information (shown on the Home screen). */
  sources: string[];
}

export const APP_VERSION: string = typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "0.0.0-dev";

export function platformOf(id: PlatformId): Platform {
  return id === "tizen" ? "tizen" : id === "webos" ? "webos" : "other-tv";
}

const HDMI_REASON =
  "Record this TV's picture by connecting its HDMI source (through an HDMI splitter) to a USB capture card on a CaptureOne desktop. HDCP-protected content (most streaming apps, Blu-ray) cannot be captured.";

export function tvFeatures(id: PlatformId): Partial<Record<FeatureKey, FeatureSupport>> {
  const os = id === "tizen" ? "Samsung Tizen" : id === "webos" ? "LG webOS" : "This browser";
  return {
    displayCapture: { level: "external-hardware", reason: `${os} gives apps no screen-capture API. ${HDMI_REASON}` },
    videoRecording: {
      level: "companion-only",
      reason: "Starts, pauses and stops recording on the paired CaptureOne desktop (e.g. of an HDMI capture card). The TV app never records the TV screen itself.",
    },
    pauseResume: { level: "companion-only", reason: "Pauses/resumes the recording running on the desktop." },
    screenshot: { level: "companion-only", reason: "Asks the paired desktop to take a screenshot of its own screen." },
    hdmiCaptureDevice: { level: "external-hardware", reason: HDMI_REASON },
    tvNativeScreenshot:
      id === "webos"
        ? {
            level: "partial",
            reason:
              "Only from a paired CaptureOne desktop over the LAN (LG ssap://tv/executeOneShot): needs accepting a prompt on the TV, depends on model/firmware, protected content is black. Not available inside this app and not verified on real hardware.",
          }
        : id === "tizen"
          ? { level: "unsupported", reason: "Samsung's TV remote-control API has no screenshot function and Tizen apps cannot capture the screen." }
          : { level: "unsupported", reason: "Not a TV." },
    remoteControl: { level: "native", reason: "Controls the paired CaptureOne desktop over the LAN (HTTP + WebSocket)." },
    recordingHistory: { level: "companion-only", reason: "Browses and plays the desktop's recording library." },
    fileTransfer: { level: "partial", reason: "Streams recordings from the desktop for playback (HTTP range requests); saving files on the TV is not supported." },
    deviceDiscovery: { level: "unsupported", reason: "TV web apps cannot send multicast discovery packets; enter the address shown in the desktop app." },
    microphoneAudio: { level: "unsupported", reason: "TV apps have no audio capture API." },
    systemAudio: { level: "unsupported", reason: "TV apps have no audio capture API." },
    share: { level: "unsupported", reason: "No share target on TVs." },
  };
}

export function buildTvCapabilityReport(id: PlatformId, info: TvDeviceInfo, generatedAt: string = new Date().toISOString()): CapabilityReport {
  const report: CapabilityReport = {
    schemaVersion: 1,
    platform: platformOf(id),
    osName: info.osName,
    osVersion: info.osVersion || "unknown",
    appVersion: APP_VERSION,
    generatedAt,
    features: tvFeatures(id),
    encoders: [],
    frameRates: [],
    screenshotFormats: [],
    notes: [
      "Companion app: it never records or screenshots the TV screen itself.",
      "To record what the TV shows, capture its HDMI signal with a capture card attached to a CaptureOne desktop. HDCP-protected content stays black.",
    ],
  };
  if (info.model) report.deviceModel = info.model;
  return report;
}

/** Features shown on the Home screen, in display order. */
export const DISPLAYED_FEATURES: readonly FeatureKey[] = [
  "displayCapture",
  "videoRecording",
  "screenshot",
  "hdmiCaptureDevice",
  "tvNativeScreenshot",
  "remoteControl",
  "recordingHistory",
  "fileTransfer",
  "deviceDiscovery",
  "microphoneAudio",
  "systemAudio",
];

export const FEATURE_LABELS: Record<FeatureKey, string> = {
  screenshot: "Screenshot",
  displayCapture: "Capture this TV's screen",
  windowCapture: "Window capture",
  regionCapture: "Region capture",
  multiDisplay: "Multiple displays",
  videoRecording: "Video recording",
  pauseResume: "Pause / resume",
  cursorCapture: "Cursor capture",
  microphoneAudio: "Microphone audio",
  systemAudio: "TV audio capture",
  separateAudioTracks: "Separate audio tracks",
  h264: "H.264",
  h265: "H.265",
  hardwareEncoding: "Hardware encoding",
  globalShortcuts: "Global shortcuts",
  recordingHistory: "Recording library",
  share: "Share",
  hdmiCaptureDevice: "HDMI capture card",
  remoteControl: "Remote control of the desktop",
  deviceDiscovery: "Automatic hub discovery",
  tvNativeScreenshot: "TV picture screenshot (LG)",
  fileTransfer: "Play recordings from the desktop",
};

export const LEVEL_LABELS: Record<string, string> = {
  native: "Native",
  partial: "Partial",
  "companion-only": "Companion only",
  "external-hardware": "External hardware",
  unsupported: "Unsupported",
};

export const LEVEL_EXPLANATIONS: Record<string, string> = {
  native: "Works with a public API available to ordinary apps.",
  partial: "Works with restrictions (model, firmware, DRM).",
  "companion-only": "Done by the paired CaptureOne desktop; this TV only controls it.",
  "external-hardware": "Only possible with an HDMI capture card on a desktop.",
  unsupported: "Not possible on this TV.",
};
