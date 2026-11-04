import type { QualityPreset, ResolutionSetting, VideoCodec } from "@captureone/capture-contracts";
import presets from "../presets.json" with { type: "json" };

export const PRESETS = presets;
export type Quality = Exclude<QualityPreset, "custom">;

export interface Size {
  width: number;
  height: number;
}

const even = (n: number): number => Math.max(2, Math.floor(n / 2) * 2);

/**
 * Output size for a source and a resolution setting. Never upscales, preserves
 * aspect ratio, and always returns even dimensions (required by 4:2:0 encoders).
 */
export function resolveOutputSize(source: Size, setting: ResolutionSetting): Size {
  // exhaustive switch below; every branch returns
  if (source.width <= 0 || source.height <= 0) throw new RangeError("source size must be positive");
  switch (setting.mode) {
    case "source":
      return { width: even(source.width), height: even(source.height) };
    case "height": {
      if (setting.height >= source.height) return { width: even(source.width), height: even(source.height) };
      const scale = setting.height / source.height;
      return { width: even(Math.round(source.width * scale)), height: even(setting.height) };
    }
    case "custom": {
      const scale = Math.min(setting.width / source.width, setting.height / source.height, 1);
      return { width: even(Math.round(source.width * scale)), height: even(Math.round(source.height * scale)) };
    }
    default: {
      const never: never = setting;
      throw new RangeError(`unknown resolution mode ${JSON.stringify(never)}`);
    }
  }
}

/** Target video bitrate in kbps for a preset. */
export function videoBitrateKbps(size: Size, fps: number, codec: VideoCodec, quality: QualityPreset, customKbps?: number): number {
  if (quality === "custom") {
    if (customKbps === undefined) throw new RangeError("customKbps required for custom quality");
    return Math.round(customKbps);
  }
  const table: Record<VideoCodec, Record<Quality, number>> = PRESETS.bitsPerPixel;
  const bpp = table[codec][quality];
  const kbps = (size.width * size.height * fps * bpp) / 1000;
  return Math.round(Math.min(PRESETS.maxBitrateKbps, Math.max(PRESETS.minBitrateKbps, kbps)));
}

/** Estimated output size in bytes for a duration, including container overhead. */
export function estimateFileSizeBytes(videoKbps: number, audioKbpsTotal: number, durationSec: number): number {
  if (durationSec < 0) throw new RangeError("duration must be >= 0");
  const bytes = ((videoKbps + audioKbpsTotal) * 1000 * durationSec) / 8;
  return Math.round(bytes * (1 + PRESETS.containerOverhead));
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}
