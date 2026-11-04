import { z } from "zod";
import { CaptureTarget } from "./sources.js";
import { ImageFormat } from "./platform.js";

export const ScreenshotRequest = z.object({
  target: CaptureTarget,
  format: ImageFormat.default("png"),
  /** 1..100, JPEG only. */
  jpegQuality: z.number().int().min(1).max(100).default(90),
  /** Defaults to the configured output directory. */
  outputDirectory: z.string().optional(),
  fileNameTemplate: z.string().optional(),
  showCursor: z.boolean().default(true),
  delaySec: z.number().int().min(0).max(30).default(0),
});
export type ScreenshotRequest = z.infer<typeof ScreenshotRequest>;
export type ScreenshotRequestInput = z.input<typeof ScreenshotRequest>;
