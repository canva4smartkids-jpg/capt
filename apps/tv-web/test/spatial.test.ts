import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { findNext, score, SpatialNavigator, type Rect } from "../src/spatial.js";

const r = (left: number, top: number, width = 100, height = 50): Rect => ({ left, top, width, height });

describe("findNext (geometry)", () => {
  // 3x3 grid of 100x50 cells with 20px gaps.
  const grid = [0, 1, 2].flatMap((row) => [0, 1, 2].map((col) => ({ item: `${row}${col}`, rect: r(col * 120, row * 70) })));
  const at = (id: string) => grid.find((c) => c.item === id)!.rect;

  it("moves to the aligned neighbour in each direction", () => {
    expect(findNext(at("11"), grid, "right")?.item).toBe("12");
    expect(findNext(at("11"), grid, "left")?.item).toBe("10");
    expect(findNext(at("11"), grid, "up")?.item).toBe("01");
    expect(findNext(at("11"), grid, "down")?.item).toBe("21");
  });

  it("returns null at the edges", () => {
    expect(findNext(at("00"), grid, "left")).toBeNull();
    expect(findNext(at("00"), grid, "up")).toBeNull();
    expect(findNext(at("22"), grid, "right")).toBeNull();
    expect(findNext(at("22"), grid, "down")).toBeNull();
  });

  it("prefers an aligned element over a closer diagonal one", () => {
    const from = r(0, 0);
    const diagonal = { item: "diag", rect: r(110, 60) }; // just right-below
    const aligned = { item: "aligned", rect: r(400, 0) }; // far right, same row
    expect(findNext(from, [diagonal, aligned], "right")?.item).toBe("aligned");
  });

  it("goes from a wide button down to the most centred key", () => {
    const wide = r(0, 0, 600, 80);
    const keys = [0, 1, 2, 3].map((i) => ({ item: `k${i}`, rect: r(i * 150, 120, 140, 80) }));
    // centre of wide = 300; key centres 70, 220, 370, 520 -> k1 (|220-300|=80) and k2 (70): k2 wins
    expect(findNext(wide, keys, "down")?.item).toBe("k2");
  });

  it("ignores zero-size (hidden) candidates", () => {
    expect(findNext(r(0, 0), [{ item: "hidden", rect: r(200, 0, 0, 0) }], "right")).toBeNull();
  });

  it("handles overlapping rectangles (scaled focus) without moving backwards", () => {
    const from = r(100, 100, 100, 50);
    expect(score(from, r(150, 100, 100, 50), "right")).not.toBeNull();
    expect(score(from, r(150, 100, 100, 50), "left")).toBeNull();
    expect(score(from, from, "right")).toBeNull();
  });
});

describe("SpatialNavigator (DOM)", () => {
  const rects = new Map<Element, Rect>();
  const originalRect = HTMLElement.prototype.getBoundingClientRect;
  const originalOffsetParent = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetParent");

  beforeEach(() => {
    HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
      const x = rects.get(this) ?? { left: 0, top: 0, width: 0, height: 0 };
      return { ...x, x: x.left, y: x.top, right: x.left + x.width, bottom: x.top + x.height, toJSON: () => x } as DOMRect;
    };
    Object.defineProperty(HTMLElement.prototype, "offsetParent", { configurable: true, get: () => document.body });
    document.body.innerHTML = `
      <div id="root">
        <button id="a" data-focusable>A</button><button id="b" data-focusable>B</button>
        <button id="c" data-focusable data-autofocus>C</button><button id="d" data-focusable disabled>D</button>
      </div>`;
    const set = (id: string, rect: Rect) => rects.set(document.getElementById(id)!, rect);
    set("a", r(0, 0));
    set("b", r(200, 0));
    set("c", r(0, 100));
    set("d", r(200, 100));
  });
  afterEach(() => {
    HTMLElement.prototype.getBoundingClientRect = originalRect;
    if (originalOffsetParent) Object.defineProperty(HTMLElement.prototype, "offsetParent", originalOffsetParent);
    rects.clear();
  });

  it("focuses the autofocus element first, then moves by geometry and skips disabled", () => {
    const nav = new SpatialNavigator(document.getElementById("root")!);
    expect(nav.move("right")?.id).toBe("c"); // nothing focused yet -> default
    expect(document.activeElement?.id).toBe("c");
    expect(nav.move("up")?.id).toBe("a");
    expect(nav.move("right")?.id).toBe("b");
    // The disabled D directly below B is skipped; the diagonal C is the only candidate.
    expect(nav.move("down")?.id).toBe("c");
    expect(nav.move("down")).toBeNull(); // nothing below C
    expect(document.activeElement?.id).toBe("c");
    expect(nav.focusables().map((e) => e.id)).toEqual(["a", "b", "c"]);
  });
});
