import { convertFileSrc } from "@tauri-apps/api/core";
import type { MediaMetadata } from "@captureone/capture-contracts";

/** URL for a local file through Tauri's asset protocol (scoped to the output/thumbnail dirs by the backend). */
export function assetUrl(path: string | undefined | null): string | undefined {
  if (!path) return undefined;
  try {
    return convertFileSrc(path);
  } catch {
    return undefined;
  }
}

/** Thumbnail for a library item: generated thumbnail, else the screenshot itself. */
export function thumbnailUrl(m: MediaMetadata): string | undefined {
  if (m.thumbnailPath) return assetUrl(m.thumbnailPath);
  if (m.kind === "screenshot") return assetUrl(m.path);
  return undefined;
}
