import { describe, expect, it } from "vitest";
import { asyncData, isShortcutAction, isShortcutEventPayload, resolveTheme, SHORTCUT_ACTIONS, SHORTCUT_SETTING_KEYS } from "../src/index.js";

describe("shortcut payloads", () => {
  it("accepts every declared action and rejects others", () => {
    for (const a of SHORTCUT_ACTIONS) expect(isShortcutAction(a)).toBe(true);
    expect(isShortcutAction("record")).toBe(false);
    expect(isShortcutAction(3)).toBe(false);
    expect(isShortcutEventPayload({ action: "screenshot" })).toBe(true);
    expect(isShortcutEventPayload({ action: "nope" })).toBe(false);
    expect(isShortcutEventPayload(null)).toBe(false);
  });
  it("maps every action to a settings key", () => {
    expect(Object.keys(SHORTCUT_SETTING_KEYS).sort()).toEqual([...SHORTCUT_ACTIONS].sort());
  });
});

describe("theme + async helpers", () => {
  it("resolves system theme from the media query", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
  });
  it("keeps previous data while reloading or after an error", () => {
    expect(asyncData({ status: "loading", previous: 1 })).toBe(1);
    expect(asyncData({ status: "error", error: "x", previous: 2 })).toBe(2);
    expect(asyncData({ status: "success", data: 3 })).toBe(3);
    expect(asyncData({ status: "idle" })).toBeUndefined();
  });
});
