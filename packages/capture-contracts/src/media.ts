import { z } from "zod";
import { ImageFormat, VideoCodec } from "./platform.js";

export const AudioTrackInfo = z.object({
  index: z.number().int().nonnegative(),
  codec: z.string(),
  channels: z.number().int().positive(),
  sampleRate: z.number().int().positive(),
  /** "microphone" | "system" | "mixed" when known. */
  label: z.string().optional(),
  durationMs: z.number().nonnegative().optional(),
});
export type AudioTrackInfo = z.infer<typeof AudioTrackInfo>;

export const ValidationIssue = z.object({
  severity: z.enum(["error", "warning"]),
  code: z.string(),
  message: z.string(),
});
export type ValidationIssue = z.infer<typeof ValidationIssue>;

export const MediaValidation = z.object({
  /** True only if the file was inspected after writing (ffprobe / MediaExtractor) and had no errors. */
  valid: z.boolean(),
  checkedAt: z.string(),
  tool: z.string(),
  issues: z.array(ValidationIssue),
});
export type MediaValidation = z.infer<typeof MediaValidation>;

export const MediaKind = z.enum(["video", "screenshot"]);
export type MediaKind = z.infer<typeof MediaKind>;

export const MediaMetadata = z.object({
  id: z.string().min(1),
  kind: MediaKind,
  path: z.string().min(1),
  fileName: z.string().min(1),
  title: z.string(),
  createdAt: z.string(),
  sizeBytes: z.number().int().nonnegative(),
  width: z.number().int().positive(),
  height: z.number().int().positive(),
  /** Video only. */
  durationMs: z.number().nonnegative().optional(),
  fps: z.number().positive().optional(),
  videoCodec: VideoCodec.optional(),
  videoBitrateKbps: z.number().nonnegative().optional(),
  audioTracks: z.array(AudioTrackInfo).default([]),
  /** Screenshot only. */
  imageFormat: ImageFormat.optional(),
  container: z.enum(["mp4", "png", "jpeg"]),
  sourceName: z.string().optional(),
  platform: z.string().optional(),
  validation: MediaValidation.optional(),
  thumbnailPath: z.string().optional(),
});
export type MediaMetadata = z.infer<typeof MediaMetadata>;
