/**
 * Geometry for the full-screen region selector. The overlay works in CSS pixels;
 * the contract wants physical pixels relative to the display's top-left, so the
 * final rect is scaled by devicePixelRatio and clamped to the display.
 */
import type { Rect } from "@captureone/capture-contracts";

export interface Point {
  x: number;
  y: number;
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Bounds {
  width: number;
  height: number;
}

/** Minimum region in physical pixels (matches RecordingConfig validation). */
export const MIN_REGION_PX = 16;

export type Handle = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";
export const HANDLES: readonly Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Rectangle spanned by two drag points, clamped to the bounds. */
export function rectFromPoints(a: Point, b: Point, bounds: Bounds): Box {
  const x1 = clamp(Math.min(a.x, b.x), 0, bounds.width);
  const y1 = clamp(Math.min(a.y, b.y), 0, bounds.height);
  const x2 = clamp(Math.max(a.x, b.x), 0, bounds.width);
  const y2 = clamp(Math.max(a.y, b.y), 0, bounds.height);
  return { x: x1, y: y1, width: x2 - x1, height: y2 - y1 };
}

/** Moves a box by (dx, dy) keeping it fully inside the bounds. */
export function moveBox(b: Box, dx: number, dy: number, bounds: Bounds): Box {
  return {
    ...b,
    x: clamp(b.x + dx, 0, Math.max(0, bounds.width - b.width)),
    y: clamp(b.y + dy, 0, Math.max(0, bounds.height - b.height)),
  };
}

/** Resizes a box by dragging a handle; the opposite edge stays fixed; respects min size and bounds. */
export function resizeBox(b: Box, handle: Handle, dx: number, dy: number, bounds: Bounds, min: number): Box {
  let left = b.x;
  let top = b.y;
  let right = b.x + b.width;
  let bottom = b.y + b.height;
  if (handle.includes("w")) left = clamp(left + dx, 0, right - min);
  if (handle.includes("e")) right = clamp(right + dx, left + min, bounds.width);
  if (handle.includes("n")) top = clamp(top + dy, 0, bottom - min);
  if (handle.includes("s")) bottom = clamp(bottom + dy, top + min, bounds.height);
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** Converts a CSS-pixel box to an integer physical-pixel Rect clamped to the display. */
export function toPhysicalRect(b: Box, dpr: number, display: Bounds): Rect {
  const scale = dpr > 0 && Number.isFinite(dpr) ? dpr : 1;
  const x = clamp(Math.round(b.x * scale), 0, display.width);
  const y = clamp(Math.round(b.y * scale), 0, display.height);
  const right = clamp(Math.round((b.x + b.width) * scale), 0, display.width);
  const bottom = clamp(Math.round((b.y + b.height) * scale), 0, display.height);
  return { x, y, width: Math.max(1, right - x), height: Math.max(1, bottom - y) };
}

export function isValidRegion(r: Rect): boolean {
  return r.width >= MIN_REGION_PX && r.height >= MIN_REGION_PX;
}

/** Even dimensions (what 4:2:0 encoders will actually produce). */
export function encodedSize(r: Rect): { width: number; height: number } {
  return { width: Math.max(2, Math.floor(r.width / 2) * 2), height: Math.max(2, Math.floor(r.height / 2) * 2) };
}

/** Keyboard adjustments: arrows move; Shift = 10px steps; Alt+arrows resize from the bottom-right. */
export function keyboardAdjust(b: Box, key: string, opts: { shift: boolean; alt: boolean }, bounds: Bounds, min: number): Box | null {
  const step = opts.shift ? 10 : 1;
  const delta: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
  const d = delta[key];
  if (!d) return null;
  return opts.alt ? resizeBox(b, "se", d[0], d[1], bounds, min) : moveBox(b, d[0], d[1], bounds);
}

/** Which handle (if any) is under a point, given a hit radius in CSS px. */
export function hitHandle(b: Box, p: Point, radius = 8): Handle | null {
  const cx = b.x + b.width / 2;
  const cy = b.y + b.height / 2;
  const pts: Record<Handle, Point> = {
    nw: { x: b.x, y: b.y },
    n: { x: cx, y: b.y },
    ne: { x: b.x + b.width, y: b.y },
    e: { x: b.x + b.width, y: cy },
    se: { x: b.x + b.width, y: b.y + b.height },
    s: { x: cx, y: b.y + b.height },
    sw: { x: b.x, y: b.y + b.height },
    w: { x: b.x, y: cy },
  };
  for (const h of HANDLES) {
    const q = pts[h];
    if (Math.abs(q.x - p.x) <= radius && Math.abs(q.y - p.y) <= radius) return h;
  }
  return null;
}

export function containsPoint(b: Box, p: Point): boolean {
  return p.x >= b.x && p.x <= b.x + b.width && p.y >= b.y && p.y <= b.y + b.height;
}
