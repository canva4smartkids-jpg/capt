/**
 * Presentation and query helpers for CaptureOne MediaMetadata. Pure functions only
 * (no DOM, no IPC) so they can be reused by the desktop UI, the TV web companion and tests.
 */
import { MediaMetadata, type MediaKind } from "@captureone/capture-contracts";
import { formatBytes, formatDuration } from "@captureone/settings";
import { z } from "zod";

export type Tone = "neutral" | "info" | "success" | "warning" | "danger";

/** Parses an untrusted list, dropping (and reporting) items that do not match the contract. */
export function parseMediaList(raw: unknown): { items: MediaMetadata[]; rejected: number } {
  const arr = z.array(z.unknown()).safeParse(raw);
  if (!arr.success) return { items: [], rejected: 0 };
  const items: MediaMetadata[] = [];
  let rejected = 0;
  for (const entry of arr.data) {
    const parsed = MediaMetadata.safeParse(entry);
    if (parsed.success) items.push(parsed.data);
    else rejected++;
  }
  return { items, rejected };
}

export function stripExtension(fileName: string): string {
  const i = fileName.lastIndexOf(".");
  return i > 0 ? fileName.slice(0, i) : fileName;
}

export function fileExtension(fileName: string): string {
  const i = fileName.lastIndexOf(".");
  return i > 0 ? fileName.slice(i + 1).toLowerCase() : "";
}

/** User-facing title: explicit title, else file name without extension. */
export function displayTitle(m: Pick<MediaMetadata, "title" | "fileName">): string {
  const t = m.title.trim();
  return t.length > 0 ? t : stripExtension(m.fileName);
}

export function formatResolution(m: Pick<MediaMetadata, "width" | "height">): string {
  return `${m.width}×${m.height}`;
}

/** Common name for a resolution (e.g. "1080p"), or undefined. */
export function resolutionName(m: Pick<MediaMetadata, "width" | "height">): string | undefined {
  const short = Math.min(m.width, m.height);
  const names: [number, string][] = [
    [2160, "4K"],
    [1440, "1440p"],
    [1080, "1080p"],
    [720, "720p"],
    [480, "480p"],
  ];
  return names.find(([h]) => h === short)?.[1];
}

export function formatLabel(m: Pick<MediaMetadata, "kind" | "videoCodec" | "imageFormat" | "container">): string {
  if (m.kind === "video") {
    if (m.videoCodec === "h264") return "H.264";
    if (m.videoCodec === "h265") return "H.265";
    return m.container.toUpperCase();
  }
  if (m.imageFormat === "jpeg") return "JPEG";
  if (m.imageFormat === "png") return "PNG";
  return m.container.toUpperCase();
}

export function durationLabel(m: Pick<MediaMetadata, "durationMs">): string | undefined {
  return m.durationMs === undefined ? undefined : formatDuration(m.durationMs);
}

export function sizeLabel(m: Pick<MediaMetadata, "sizeBytes">): string {
  return formatBytes(m.sizeBytes);
}

export function bitrateLabel(m: Pick<MediaMetadata, "videoBitrateKbps">): string | undefined {
  const k = m.videoBitrateKbps;
  if (k === undefined || k <= 0) return undefined;
  return k >= 1000 ? `${(k / 1000).toFixed(1)} Mbps` : `${Math.round(k)} kbps`;
}

export function audioSummary(m: Pick<MediaMetadata, "kind" | "audioTracks">): string {
  if (m.kind !== "video") return "";
  const n = m.audioTracks.length;
  if (n === 0) return "No audio";
  const labels = m.audioTracks.map((t) => t.label).filter((l): l is string => !!l);
  const head = n === 1 ? "1 audio track" : `${n} audio tracks`;
  return labels.length > 0 ? `${head} (${labels.join(", ")})` : head;
}

export interface ValidationBadge {
  tone: Tone;
  label: string;
  detail: string;
}

/** Honest validation summary: only "Validated" when a post-write inspection passed without errors. */
export function validationBadge(m: Pick<MediaMetadata, "validation">): ValidationBadge {
  const v = m.validation;
  if (!v) return { tone: "neutral", label: "Not validated", detail: "This file has not been inspected yet." };
  const errors = v.issues.filter((i) => i.severity === "error");
  const warnings = v.issues.filter((i) => i.severity === "warning");
  if (!v.valid || errors.length > 0) {
    const first = errors[0]?.message ?? "Inspection reported the file as invalid.";
    return { tone: "danger", label: "Invalid", detail: `${first} (checked with ${v.tool})` };
  }
  if (warnings.length > 0) {
    return { tone: "warning", label: "Warnings", detail: warnings.map((w) => w.message).join("; ") };
  }
  return { tone: "success", label: "Validated", detail: `Inspected with ${v.tool} at ${v.checkedAt}` };
}

export type LibraryFilter = "all" | MediaKind;
export type LibrarySort = "newest" | "oldest" | "largest" | "longest" | "name";

export interface LibraryQuery {
  filter: LibraryFilter;
  search: string;
  sort: LibrarySort;
}

export const DEFAULT_LIBRARY_QUERY: LibraryQuery = { filter: "all", search: "", sort: "newest" };

function haystack(m: MediaMetadata): string {
  return [displayTitle(m), m.fileName, m.sourceName ?? "", formatLabel(m), formatResolution(m), resolutionName(m) ?? ""]
    .join(" ")
    .toLowerCase();
}

function time(m: MediaMetadata): number {
  const t = Date.parse(m.createdAt);
  return Number.isNaN(t) ? 0 : t;
}

/** Filters by kind, matches every whitespace-separated search term, then sorts (stable). */
export function queryLibrary(items: readonly MediaMetadata[], q: LibraryQuery): MediaMetadata[] {
  const terms = q.search.toLowerCase().split(/\s+/).filter(Boolean);
  const out = items.filter((m) => (q.filter === "all" || m.kind === q.filter) && terms.every((t) => haystack(m).includes(t)));
  const cmp: Record<LibrarySort, (a: MediaMetadata, b: MediaMetadata) => number> = {
    newest: (a, b) => time(b) - time(a),
    oldest: (a, b) => time(a) - time(b),
    largest: (a, b) => b.sizeBytes - a.sizeBytes,
    longest: (a, b) => (b.durationMs ?? -1) - (a.durationMs ?? -1),
    name: (a, b) => displayTitle(a).localeCompare(displayTitle(b), undefined, { sensitivity: "base", numeric: true }),
  };
  return out
    .map((m, i) => ({ m, i }))
    .sort((a, b) => cmp[q.sort](a.m, b.m) || a.i - b.i)
    .map((x) => x.m);
}

export interface LibrarySummary {
  total: number;
  videos: number;
  screenshots: number;
  totalBytes: number;
  totalDurationMs: number;
  invalid: number;
}

export function summarizeLibrary(items: readonly MediaMetadata[]): LibrarySummary {
  let videos = 0;
  let screenshots = 0;
  let totalBytes = 0;
  let totalDurationMs = 0;
  let invalid = 0;
  for (const m of items) {
    if (m.kind === "video") videos++;
    else screenshots++;
    totalBytes += m.sizeBytes;
    totalDurationMs += m.durationMs ?? 0;
    if (m.validation && !m.validation.valid) invalid++;
  }
  return { total: items.length, videos, screenshots, totalBytes, totalDurationMs, invalid };
}

/** Replaces an item by id (or prepends it when new), preserving order. */
export function upsertMedia(items: readonly MediaMetadata[], next: MediaMetadata): MediaMetadata[] {
  const idx = items.findIndex((m) => m.id === next.id);
  if (idx === -1) return [next, ...items];
  const copy = items.slice();
  copy[idx] = next;
  return copy;
}

export function removeMedia(items: readonly MediaMetadata[], id: string): MediaMetadata[] {
  return items.filter((m) => m.id !== id);
}

/** Suggested export file name for a save dialog (title + original extension). */
export function exportFileName(m: Pick<MediaMetadata, "title" | "fileName">): string {
  const ext = fileExtension(m.fileName);
  const base = displayTitle(m).replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_").trim() || "CaptureOne";
  return ext ? `${base}.${ext}` : base;
}

/** Validates a rename. Returns an error message or null. */
export function validateTitle(title: string): string | null {
  const t = title.trim();
  if (t.length === 0) return "Title cannot be empty.";
  if (t.length > 120) return "Title must be 120 characters or fewer.";
  return null;
}

const TV_PLATFORMS = new Set(["webos", "tizen", "android-tv", "google-tv", "other-tv"]);

/** True for items captured on a TV (e.g. the experimental LG webOS screenshot), not on this computer. */
export function isTvCapture(m: Pick<MediaMetadata, "platform">): boolean {
  return m.platform !== undefined && TV_PLATFORMS.has(m.platform);
}

/** Usage ratio of library bytes against library + free disk, clamped to 0..1. */
export function storageRatio(totalBytes: number, freeDiskBytes: number): number {
  const denom = totalBytes + freeDiskBytes;
  if (!Number.isFinite(denom) || denom <= 0) return 0;
  return Math.min(1, Math.max(0, totalBytes / denom));
}
