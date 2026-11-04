import { z } from "zod";
import { AudioSettings, ImageFormat, VideoSettings } from "@captureone/capture-contracts";

export const ShortcutBindings = z.object({
  startStop: z.string().default("CommandOrControl+Shift+R"),
  pauseResume: z.string().default("CommandOrControl+Shift+P"),
  screenshot: z.string().default("CommandOrControl+Shift+S"),
  regionScreenshot: z.string().default("CommandOrControl+Shift+A"),
});
export type ShortcutBindings = z.infer<typeof ShortcutBindings>;

export const AppSettings = z.object({
  schemaVersion: z.literal(1).default(1),
  outputDirectory: z.string().default(""),
  fileNameTemplate: z.string().default("CaptureOne {date} {time}"),
  video: VideoSettings.default({
    codec: "h264",
    resolution: { mode: "source" },
    fps: 30,
    quality: "medium",
    keyframeIntervalSec: 2,
    hardwareAcceleration: "auto",
    showCursor: true,
  }),
  audio: AudioSettings.default({
    microphone: { enabled: false, gain: 1 },
    system: { enabled: false, gain: 1 },
    trackMode: "mixed",
    sampleRate: 48000,
    bitrateKbps: 160,
  }),
  screenshotFormat: ImageFormat.default("png"),
  jpegQuality: z.number().int().min(1).max(100).default(90),
  countdownSec: z.union([z.literal(0), z.literal(3), z.literal(5), z.literal(10)]).default(3),
  maxDurationSec: z.number().int().positive().optional(),
  minFreeDiskMb: z.number().int().nonnegative().default(500),
  shortcuts: ShortcutBindings.default({
    startStop: "CommandOrControl+Shift+R",
    pauseResume: "CommandOrControl+Shift+P",
    screenshot: "CommandOrControl+Shift+S",
    regionScreenshot: "CommandOrControl+Shift+A",
  }),
  showRecordingIndicator: z.boolean().default(true),
  minimizeOnRecord: z.boolean().default(false),
  companionHubEnabled: z.boolean().default(false),
  theme: z.enum(["system", "light", "dark"]).default("system"),
});
export type AppSettings = z.infer<typeof AppSettings>;

export function defaultSettings(outputDirectory = ""): AppSettings {
  return AppSettings.parse({ outputDirectory });
}

/** Parses persisted settings, falling back to defaults for invalid or missing fields. */
export function loadSettings(raw: unknown, outputDirectory = ""): AppSettings {
  const parsed = AppSettings.safeParse(raw);
  if (parsed.success) return parsed.data.outputDirectory ? parsed.data : { ...parsed.data, outputDirectory };
  return defaultSettings(outputDirectory);
}
