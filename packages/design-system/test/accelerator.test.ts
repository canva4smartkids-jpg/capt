import { describe, expect, it } from "vitest";
import { acceleratorFromKeyEvent, acceleratorLabel, acceleratorParts, isSafeGlobalAccelerator, matchesAccelerator, parseAccelerator } from "../src/accelerator.js";

const ev = (key: string, code: string, mods: Partial<{ ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) => ({
  key,
  code,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("parseAccelerator", () => {
  it("canonicalises modifier aliases and ordering", () => {
    expect(parseAccelerator("shift+cmdorctrl+r")).toEqual({ modifiers: ["CommandOrControl", "Shift"], key: "R" });
    expect(parseAccelerator("Ctrl+Alt+F12")).toEqual({ modifiers: ["Control", "Alt"], key: "F12" });
    expect(parseAccelerator("Option+Space")).toEqual({ modifiers: ["Alt"], key: "Space" });
  });
  it("rejects invalid accelerators", () => {
    expect(parseAccelerator("")).toBeNull();
    expect(parseAccelerator("Shift+Ctrl")).toBeNull();
    expect(parseAccelerator("Ctrl+A+B")).toBeNull();
    expect(parseAccelerator("Ctrl+Ctrl+A")).toBeNull();
    expect(parseAccelerator("Ctrl+Banana")).toBeNull();
  });
  it("requires a real modifier for global shortcuts", () => {
    expect(isSafeGlobalAccelerator("CommandOrControl+Shift+R")).toBe(true);
    expect(isSafeGlobalAccelerator("R")).toBe(false);
    expect(isSafeGlobalAccelerator("Shift+R")).toBe(false);
    expect(isSafeGlobalAccelerator("F9")).toBe(true);
    expect(isSafeGlobalAccelerator("PrintScreen")).toBe(true);
  });
});

describe("acceleratorFromKeyEvent", () => {
  it("maps Ctrl (Cmd on mac) to CommandOrControl using physical key codes", () => {
    expect(acceleratorFromKeyEvent(ev("R", "KeyR", { ctrlKey: true, shiftKey: true }), "other")).toBe("CommandOrControl+Shift+R");
    expect(acceleratorFromKeyEvent(ev("®", "KeyR", { metaKey: true, altKey: true }), "mac")).toBe("CommandOrControl+Alt+R");
    expect(acceleratorFromKeyEvent(ev("!", "Digit1", { ctrlKey: true, shiftKey: true }), "other")).toBe("CommandOrControl+Shift+1");
    expect(acceleratorFromKeyEvent(ev("F5", "F5"), "other")).toBe("F5");
  });
  it("ignores modifier-only presses", () => {
    expect(acceleratorFromKeyEvent(ev("Shift", "ShiftLeft", { shiftKey: true }), "other")).toBeNull();
  });
  it("matches events against accelerators", () => {
    expect(matchesAccelerator(ev("s", "KeyS", { ctrlKey: true, shiftKey: true }), "CmdOrCtrl+Shift+S", "other")).toBe(true);
    expect(matchesAccelerator(ev("s", "KeyS", { ctrlKey: true }), "CmdOrCtrl+Shift+S", "other")).toBe(false);
  });
});

describe("display", () => {
  it("renders platform-specific symbols and names", () => {
    expect(acceleratorLabel("CommandOrControl+Shift+R", "mac")).toBe("⌘⇧R");
    expect(acceleratorLabel("CommandOrControl+Shift+R", "other")).toBe("Ctrl+Shift+R");
    expect(acceleratorParts("Alt+Up", "mac")).toEqual([
      { symbol: "⌥", name: "Option" },
      { symbol: "↑", name: "Up" },
    ]);
  });
});
