import { describe, expect, it } from "vitest";
import { encodedSize, hitHandle, isValidRegion, keyboardAdjust, moveBox, rectFromPoints, resizeBox, toPhysicalRect } from "../src/lib/region";

const bounds = { width: 1000, height: 800 };

describe("region selector math", () => {
  it("normalises drags in any direction and clamps to the screen", () => {
    expect(rectFromPoints({ x: 300, y: 200 }, { x: 100, y: 50 }, bounds)).toEqual({ x: 100, y: 50, width: 200, height: 150 });
    expect(rectFromPoints({ x: 900, y: 700 }, { x: 1200, y: 900 }, bounds)).toEqual({ x: 900, y: 700, width: 100, height: 100 });
    expect(rectFromPoints({ x: -20, y: 10 }, { x: 10, y: 20 }, bounds)).toEqual({ x: 0, y: 10, width: 10, height: 10 });
  });

  it("converts CSS pixels to physical pixels on HiDPI displays and clamps", () => {
    expect(toPhysicalRect({ x: 10.2, y: 20.6, width: 100.4, height: 50 }, 2, { width: 2000, height: 1600 })).toEqual({ x: 20, y: 41, width: 201, height: 100 });
    expect(toPhysicalRect({ x: 950, y: 790, width: 100, height: 100 }, 1.5, { width: 1500, height: 1200 })).toEqual({ x: 1425, y: 1185, width: 75, height: 15 });
    expect(toPhysicalRect({ x: 0, y: 0, width: 10, height: 10 }, Number.NaN, bounds)).toEqual({ x: 0, y: 0, width: 10, height: 10 });
  });

  it("enforces the minimum size and computes encoded (even) size", () => {
    expect(isValidRegion({ x: 0, y: 0, width: 16, height: 16 })).toBe(true);
    expect(isValidRegion({ x: 0, y: 0, width: 15, height: 400 })).toBe(false);
    expect(encodedSize({ x: 0, y: 0, width: 641, height: 361 })).toEqual({ width: 640, height: 360 });
  });

  it("moves inside the bounds", () => {
    const b = { x: 900, y: 10, width: 80, height: 50 };
    expect(moveBox(b, 50, -30, bounds)).toEqual({ x: 920, y: 0, width: 80, height: 50 });
  });

  it("resizes from handles keeping the opposite edge and the minimum", () => {
    const b = { x: 100, y: 100, width: 200, height: 100 };
    expect(resizeBox(b, "se", 50, 20, bounds, 16)).toEqual({ x: 100, y: 100, width: 250, height: 120 });
    expect(resizeBox(b, "nw", -20, -10, bounds, 16)).toEqual({ x: 80, y: 90, width: 220, height: 110 });
    expect(resizeBox(b, "w", 500, 0, bounds, 16)).toEqual({ x: 284, y: 100, width: 16, height: 100 });
    expect(resizeBox(b, "e", 5000, 0, bounds, 16).width).toBe(900);
    expect(resizeBox(b, "n", 0, -500, bounds, 16)).toEqual({ x: 100, y: 0, width: 200, height: 200 });
  });

  it("supports keyboard nudging and resizing", () => {
    const b = { x: 100, y: 100, width: 200, height: 100 };
    expect(keyboardAdjust(b, "ArrowRight", { shift: false, alt: false }, bounds, 16)).toEqual({ ...b, x: 101 });
    expect(keyboardAdjust(b, "ArrowUp", { shift: true, alt: false }, bounds, 16)).toEqual({ ...b, y: 90 });
    expect(keyboardAdjust(b, "ArrowLeft", { shift: true, alt: true }, bounds, 16)).toEqual({ ...b, width: 190 });
    expect(keyboardAdjust(b, "KeyA", { shift: false, alt: false }, bounds, 16)).toBeNull();
  });

  it("hit-tests handles", () => {
    const b = { x: 100, y: 100, width: 200, height: 100 };
    expect(hitHandle(b, { x: 103, y: 98 })).toBe("nw");
    expect(hitHandle(b, { x: 200, y: 200 })).toBe("s");
    expect(hitHandle(b, { x: 200, y: 150 })).toBeNull();
  });
});
