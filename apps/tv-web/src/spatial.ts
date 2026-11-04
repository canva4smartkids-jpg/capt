/**
 * Geometric spatial navigation for remote-control (D-pad) operation.
 *
 * From the focused element's rectangle, a candidate qualifies for a direction when it extends
 * beyond the current element in that direction and its centre lies on that side. Among
 * qualifying candidates the lowest score wins:
 *
 *   score = gap along the direction + 2 x gap across it (0 when the ranges overlap)
 *           + 0.1 x distance between the centres across the direction
 *
 * so an aligned neighbour (same row/column) beats a closer diagonal one, and ties between
 * aligned neighbours go to the most centred one.
 */
import type { Direction } from "./keys.js";

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface Candidate<T> {
  item: T;
  rect: Rect;
}

const right = (r: Rect) => r.left + r.width;
const bottom = (r: Rect) => r.top + r.height;
const cx = (r: Rect) => r.left + r.width / 2;
const cy = (r: Rect) => r.top + r.height / 2;

function rangeGap(a0: number, a1: number, b0: number, b1: number): number {
  if (b1 < a0) return a0 - b1;
  if (b0 > a1) return b0 - a1;
  return 0;
}

/** Score of `to` relative to `from` in `dir`, or null when `to` is not in that direction. */
export function score(from: Rect, to: Rect, dir: Direction): number | null {
  const eps = 0.5;
  let primary: number;
  let cross: number;
  let centre: number;
  switch (dir) {
    case "right":
      if (!(right(to) > right(from) + eps && cx(to) > cx(from) + eps)) return null;
      primary = Math.max(0, to.left - right(from));
      cross = rangeGap(from.top, bottom(from), to.top, bottom(to));
      centre = Math.abs(cy(to) - cy(from));
      break;
    case "left":
      if (!(to.left < from.left - eps && cx(to) < cx(from) - eps)) return null;
      primary = Math.max(0, from.left - right(to));
      cross = rangeGap(from.top, bottom(from), to.top, bottom(to));
      centre = Math.abs(cy(to) - cy(from));
      break;
    case "down":
      if (!(bottom(to) > bottom(from) + eps && cy(to) > cy(from) + eps)) return null;
      primary = Math.max(0, to.top - bottom(from));
      cross = rangeGap(from.left, right(from), to.left, right(to));
      centre = Math.abs(cx(to) - cx(from));
      break;
    case "up":
      if (!(to.top < from.top - eps && cy(to) < cy(from) - eps)) return null;
      primary = Math.max(0, from.top - bottom(to));
      cross = rangeGap(from.left, right(from), to.left, right(to));
      centre = Math.abs(cx(to) - cx(from));
      break;
  }
  return primary + 2 * cross + 0.1 * centre;
}

export function findNext<T>(from: Rect, candidates: readonly Candidate<T>[], dir: Direction): Candidate<T> | null {
  let best: Candidate<T> | null = null;
  let bestScore = Infinity;
  for (const c of candidates) {
    if (c.rect.width <= 0 || c.rect.height <= 0) continue;
    const s = score(from, c.rect, dir);
    if (s !== null && s < bestScore) {
      best = c;
      bestScore = s;
    }
  }
  return best;
}

export const FOCUSABLE_SELECTOR = "[data-focusable]:not([disabled]):not([aria-hidden='true'])";

function isVisible(el: HTMLElement): boolean {
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0 && el.offsetParent !== null;
}

/** DOM side: moves real focus between `[data-focusable]` elements inside `scope`. */
export class SpatialNavigator {
  private scope: HTMLElement;

  constructor(scope: HTMLElement) {
    this.scope = scope;
  }

  setScope(scope: HTMLElement): void {
    this.scope = scope;
  }

  getScope(): HTMLElement {
    return this.scope;
  }

  focusables(): HTMLElement[] {
    const list = this.scope.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
    const out: HTMLElement[] = [];
    for (let i = 0; i < list.length; i++) {
      const el = list[i];
      if (el && isVisible(el)) out.push(el);
    }
    return out;
  }

  current(): HTMLElement | null {
    const a = document.activeElement;
    return a instanceof HTMLElement && this.scope.contains(a) && a.hasAttribute("data-focusable") ? a : null;
  }

  focus(el: HTMLElement): void {
    try {
      el.focus({ preventScroll: true });
    } catch {
      el.focus();
    }
    ensureVisible(el);
  }

  /** Focuses `[data-autofocus]`, else the first focusable. */
  focusDefault(): HTMLElement | null {
    const auto = this.scope.querySelector<HTMLElement>("[data-autofocus]");
    const target = auto && isVisible(auto) ? auto : (this.focusables()[0] ?? null);
    if (target) this.focus(target);
    return target;
  }

  move(dir: Direction): HTMLElement | null {
    const cur = this.current();
    if (!cur) return this.focusDefault();
    const from = cur.getBoundingClientRect();
    const candidates = this.focusables()
      .filter((el) => el !== cur)
      .map((el) => ({ item: el, rect: el.getBoundingClientRect() }));
    const next = findNext(from, candidates, dir);
    if (next) this.focus(next.item);
    return next ? next.item : null;
  }
}

/** Scrolls the nearest `[data-scroll]` ancestor so `el` is fully visible. */
export function ensureVisible(el: HTMLElement): void {
  let container: HTMLElement | null = el.parentElement;
  while (container && !container.hasAttribute("data-scroll")) container = container.parentElement;
  if (!container) return;
  const c = container.getBoundingClientRect();
  const r = el.getBoundingClientRect();
  // Rects are in (possibly CSS-scaled) viewport pixels; scrollTop is in layout pixels.
  const scale = container.offsetHeight > 0 ? c.height / container.offsetHeight : 1;
  const margin = 24 * scale;
  if (r.top < c.top + margin) {
    container.scrollTop -= (c.top + margin - r.top) / (scale || 1);
  } else if (r.bottom > c.bottom - margin) {
    container.scrollTop += (r.bottom - (c.bottom - margin)) / (scale || 1);
  }
}
