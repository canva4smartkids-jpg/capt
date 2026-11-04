import { z } from "zod";
import { PermissionKind } from "./permissions.js";

/** Every runtime CaptureOne ships on. `linux` is a desktop target used as the verifiable reference build. */
export const Platform = z.enum([
  "windows",
  "macos",
  "linux",
  "android",
  "android-tv",
  "google-tv",
  "tizen",
  "webos",
  "other-tv",
]);
export type Platform = z.infer<typeof Platform>;

/**
 * Honest support classification. A platform must never report `native` for a
 * feature unless the OS exposes a public, permitted API for it.
 */
export const SupportLevel = z.enum([
  /** Implemented with a public OS API available to ordinary third-party apps. */
  "native",
  /** Works with restrictions (device/OS-version dependent, app opt-out, DRM blanking...). */
  "partial",
  /** The app on this platform can only act as a companion (remote, browser, transfer). */
  "companion-only",
  /** Only possible by recording the device output through external HDMI capture hardware. */
  "external-hardware",
  /** Not possible on this platform. */
  "unsupported",
]);
export type SupportLevel = z.infer<typeof SupportLevel>;

export const FeatureKey = z.enum([
  "screenshot",
  "displayCapture",
  "windowCapture",
  "regionCapture",
  "multiDisplay",
  "videoRecording",
  "pauseResume",
  "cursorCapture",
  "microphoneAudio",
  "systemAudio",
  "separateAudioTracks",
  "h264",
  "h265",
  "hardwareEncoding",
  "globalShortcuts",
  "recordingHistory",
  "share",
  "hdmiCaptureDevice",
  "remoteControl",
  "deviceDiscovery",
  "tvNativeScreenshot",
  "fileTransfer",
]);
export type FeatureKey = z.infer<typeof FeatureKey>;

export const FeatureSupport = z.object({
  level: SupportLevel,
  /** Human readable explanation, required for anything that is not `native`. */
  reason: z.string().optional(),
  /** Permissions that must be granted before the feature can be used. */
  requires: z.array(PermissionKind).optional(),
  /** Minimum OS version, e.g. "Android 10", "macOS 12.3", "Windows 10 1903". */
  minOsVersion: z.string().optional(),
});
export type FeatureSupport = z.infer<typeof FeatureSupport>;

export const VideoCodec = z.enum(["h264", "h265"]);
export type VideoCodec = z.infer<typeof VideoCodec>;

export const EncoderInfo = z.object({
  codec: VideoCodec,
  /** Implementation name, e.g. "h264_mf", "h264_videotoolbox", "libx264", "c2.android.avc.encoder". */
  name: z.string().min(1),
  hardware: z.boolean(),
  /** Whether the encoder was verified with a real test encode (not just listed). */
  verified: z.boolean(),
});
export type EncoderInfo = z.infer<typeof EncoderInfo>;

export const FrameRate = z.union([z.literal(24), z.literal(30), z.literal(60)]);
export type FrameRate = z.infer<typeof FrameRate>;

export const ImageFormat = z.enum(["png", "jpeg"]);
export type ImageFormat = z.infer<typeof ImageFormat>;

export const CapabilityReport = z.object({
  schemaVersion: z.literal(1),
  platform: Platform,
  osName: z.string(),
  osVersion: z.string(),
  deviceModel: z.string().optional(),
  appVersion: z.string(),
  /** ISO-8601 timestamp. */
  generatedAt: z.string(),
  /** Partial: absent keys are treated as `unsupported`. */
  features: z.partialRecord(FeatureKey, FeatureSupport),
  encoders: z.array(EncoderInfo),
  frameRates: z.array(FrameRate),
  screenshotFormats: z.array(ImageFormat),
  maxRecordingResolution: z.object({ width: z.number().int().positive(), height: z.number().int().positive() }).optional(),
  notes: z.array(z.string()).default([]),
});
export type CapabilityReport = z.infer<typeof CapabilityReport>;

export function featureLevel(report: CapabilityReport, key: FeatureKey): SupportLevel {
  return report.features[key]?.level ?? "unsupported";
}

/** True when the feature can be used directly on this device (native or partial). */
export function isUsable(report: CapabilityReport, key: FeatureKey): boolean {
  const level = featureLevel(report, key);
  return level === "native" || level === "partial";
}
