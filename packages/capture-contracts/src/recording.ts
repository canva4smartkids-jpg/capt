import { z } from "zod";
import { CaptureTarget } from "./sources.js";
import { FrameRate, VideoCodec } from "./platform.js";
import { CaptureError } from "./errors.js";

export const ResolutionSetting = z.discriminatedUnion("mode", [
  /** Record at the source's native pixel size. */
  z.object({ mode: z.literal("source") }),
  /** Scale to the given height, preserving aspect ratio. Never upscales. */
  z.object({ mode: z.literal("height"), height: z.union([z.literal(2160), z.literal(1440), z.literal(1080), z.literal(720), z.literal(480)]) }),
  /** Fit within this box preserving aspect ratio (no letterboxing; output may be smaller in one dimension). Never upscales. */
  z.object({ mode: z.literal("custom"), width: z.number().int().min(16).max(7680), height: z.number().int().min(16).max(4320) }),
]);
export type ResolutionSetting = z.infer<typeof ResolutionSetting>;

export const QualityPreset = z.enum(["low", "medium", "high", "ultra", "custom"]);
export type QualityPreset = z.infer<typeof QualityPreset>;

export const VideoSettings = z.object({
  codec: VideoCodec,
  resolution: ResolutionSetting,
  fps: FrameRate,
  quality: QualityPreset,
  /** Required when quality === "custom". */
  bitrateKbps: z.number().int().min(250).max(200_000).optional(),
  keyframeIntervalSec: z.number().min(0.5).max(10).default(2),
  hardwareAcceleration: z.enum(["auto", "prefer", "off"]).default("auto"),
  showCursor: z.boolean().default(true),
});
export type VideoSettings = z.infer<typeof VideoSettings>;

export const AudioSourceSettings = z.object({
  enabled: z.boolean(),
  /** Platform device id; undefined selects the default device. */
  deviceId: z.string().optional(),
  /** Linear gain, 0..2. */
  gain: z.number().min(0).max(2).default(1),
});
export type AudioSourceSettings = z.infer<typeof AudioSourceSettings>;

export const AudioSettings = z.object({
  microphone: AudioSourceSettings,
  system: AudioSourceSettings,
  /** "mixed": one AAC track; "separate": one track per enabled source. */
  trackMode: z.enum(["mixed", "separate"]).default("mixed"),
  sampleRate: z.union([z.literal(44100), z.literal(48000)]).default(48000),
  bitrateKbps: z.union([z.literal(96), z.literal(128), z.literal(160), z.literal(192), z.literal(256)]).default(160),
});
export type AudioSettings = z.infer<typeof AudioSettings>;

export const OutputSettings = z.object({
  directory: z.string().min(1),
  /** Tokens: {date} {time} {source} {n}. Extension is appended automatically. */
  fileNameTemplate: z.string().min(1).default("CaptureOne {date} {time}"),
  container: z.literal("mp4").default("mp4"),
});
export type OutputSettings = z.infer<typeof OutputSettings>;

export const RecordingLimits = z.object({
  maxDurationSec: z.number().int().positive().optional(),
  maxFileSizeMb: z.number().int().positive().optional(),
  /** Recording stops gracefully when free space drops below this. */
  minFreeDiskMb: z.number().int().nonnegative().default(500),
});
export type RecordingLimits = z.infer<typeof RecordingLimits>;

export const RecordingConfig = z
  .object({
    target: CaptureTarget,
    video: VideoSettings,
    audio: AudioSettings,
    output: OutputSettings,
    limits: RecordingLimits.default({ minFreeDiskMb: 500 }),
    countdownSec: z.union([z.literal(0), z.literal(3), z.literal(5), z.literal(10)]).default(3),
  })
  .superRefine((cfg, ctx) => {
    if (cfg.video.quality === "custom" && cfg.video.bitrateKbps === undefined) {
      ctx.addIssue({ code: "custom", path: ["video", "bitrateKbps"], message: "bitrateKbps is required when quality is 'custom'" });
    }
  });
export type RecordingConfig = z.infer<typeof RecordingConfig>;
export type RecordingConfigInput = z.input<typeof RecordingConfig>;

export const RecordingState = z.enum([
  "idle",
  "countdown",
  "starting",
  "recording",
  "paused",
  "stopping",
  "finalizing",
  "completed",
  "failed",
]);
export type RecordingState = z.infer<typeof RecordingState>;

/** Canonical state machine. Rust and Kotlin implementations are contract-tested against this table. */
export const RECORDING_TRANSITIONS: Readonly<Record<RecordingState, readonly RecordingState[]>> = {
  idle: ["countdown", "starting"],
  countdown: ["starting", "idle"],
  starting: ["recording", "failed"],
  recording: ["paused", "stopping", "failed"],
  paused: ["recording", "stopping", "failed"],
  stopping: ["finalizing", "failed"],
  finalizing: ["completed", "failed"],
  completed: ["idle"],
  failed: ["idle"],
};

export function canTransition(from: RecordingState, to: RecordingState): boolean {
  return RECORDING_TRANSITIONS[from].includes(to);
}

export function isActive(state: RecordingState): boolean {
  return state === "recording" || state === "paused";
}

export const RecordingStatus = z.object({
  sessionId: z.string(),
  state: RecordingState,
  /** Recorded (non-paused) time. */
  elapsedMs: z.number().nonnegative(),
  countdownRemainingSec: z.number().int().nonnegative().optional(),
  bytesWritten: z.number().int().nonnegative(),
  framesEncoded: z.number().int().nonnegative(),
  framesDropped: z.number().int().nonnegative(),
  /** Projected size at maxDuration (or per-minute rate when unlimited). */
  estimatedBytesPerMinute: z.number().nonnegative().optional(),
  freeDiskBytes: z.number().int().nonnegative().optional(),
  outputPath: z.string().optional(),
  encoder: z.string().optional(),
  warnings: z.array(z.string()).default([]),
  error: CaptureError.optional(),
});
export type RecordingStatus = z.infer<typeof RecordingStatus>;

/** Why a session ended. `cancelled` = user cancelled (output discarded). */
export const StopReason = z.enum(["user", "max-duration", "max-file-size", "low-disk", "source-closed", "error", "interrupted", "cancelled"]);
export type StopReason = z.infer<typeof StopReason>;
