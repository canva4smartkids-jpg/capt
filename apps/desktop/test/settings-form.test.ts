import { describe, expect, it } from "vitest";
import { fileNamePreview, hasErrors, settingsEqual, validateSettings } from "../src/lib/settings-form";
import { settings } from "./fixtures";

describe("validateSettings", () => {
  it("accepts defaults with an output directory", () => {
    expect(hasErrors(validateSettings(settings))).toBe(false);
  });

  it("requires an output folder and a sane template", () => {
    expect(validateSettings({ ...settings, outputDirectory: " " }).outputDirectory).toBeTruthy();
    expect(validateSettings({ ...settings, fileNameTemplate: "" }).fileNameTemplate).toBeTruthy();
    expect(validateSettings({ ...settings, fileNameTemplate: "a/{date}" }).fileNameTemplate).toMatch(/folder separators/);
  });

  it("validates numeric fields", () => {
    expect(validateSettings({ ...settings, jpegQuality: 0 }).jpegQuality).toBeTruthy();
    expect(validateSettings({ ...settings, jpegQuality: 101 }).jpegQuality).toBeTruthy();
    expect(validateSettings({ ...settings, maxDurationSec: -60 }).maxDurationSec).toBeTruthy();
    expect(validateSettings({ ...settings, video: { ...settings.video, quality: "custom", bitrateKbps: 100 } })["video.bitrateKbps"]).toBeTruthy();
    expect(validateSettings({ ...settings, video: { ...settings.video, quality: "custom", bitrateKbps: 8000 } })["video.bitrateKbps"]).toBeUndefined();
  });

  it("rejects invalid, unsafe and duplicate global shortcuts (alias-aware)", () => {
    const e = validateSettings({
      ...settings,
      shortcuts: { startStop: "CmdOrCtrl+Shift+R", pauseResume: "Shift+CommandOrControl+r", screenshot: "S", regionScreenshot: "Ctrl+Nope" },
    });
    expect(e["shortcuts.startStop"]).toBeUndefined();
    expect(e["shortcuts.pauseResume"]).toMatch(/Already used by "Start \/ stop recording"/);
    expect(e["shortcuts.screenshot"]).toMatch(/need Ctrl\/Cmd or Alt/);
    expect(e["shortcuts.regionScreenshot"]).toMatch(/Not a valid shortcut/);
  });

  it("allows clearing a shortcut", () => {
    expect(validateSettings({ ...settings, shortcuts: { ...settings.shortcuts, screenshot: "" } })["shortcuts.screenshot"]).toBeUndefined();
  });
});

describe("helpers", () => {
  it("previews file names with tokens expanded and sanitized", () => {
    const d = new Date(2026, 10, 4, 9, 5, 7);
    expect(fileNamePreview("CaptureOne {date} {time}", "mp4", d)).toBe("CaptureOne 2026-11-04 09-05-07.mp4");
    expect(fileNamePreview("{source}: take {n}", "png", d, "Display 1")).toBe("Display 1_ take 1.png");
    expect(fileNamePreview("", "mp4", d)).toBe("CaptureOne.mp4");
  });
  it("compares settings structurally", () => {
    expect(settingsEqual(settings, { ...settings })).toBe(true);
    expect(settingsEqual(settings, { ...settings, theme: "dark" })).toBe(false);
  });
});
