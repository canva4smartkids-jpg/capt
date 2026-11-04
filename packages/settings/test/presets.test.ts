import { describe, expect, it } from "vitest";
import { estimateFileSizeBytes, formatBytes, formatDuration, resolveOutputSize, videoBitrateKbps } from "../src/presets.js";
import { expandFileName, sanitizeFileName } from "../src/filename.js";
import { AppSettings, defaultSettings, loadSettings } from "../src/app-settings.js";

describe("resolveOutputSize", () => {
  it("keeps source size and forces even dims", () => {
    expect(resolveOutputSize({ width: 1279, height: 719 }, { mode: "source" })).toEqual({ width: 1278, height: 718 });
  });
  it("downscales by height preserving aspect", () => {
    expect(resolveOutputSize({ width: 3840, height: 2160 }, { mode: "height", height: 1080 })).toEqual({ width: 1920, height: 1080 });
    expect(resolveOutputSize({ width: 1366, height: 768 }, { mode: "height", height: 720 })).toEqual({ width: 1280, height: 720 });
  });
  it("never upscales", () => {
    expect(resolveOutputSize({ width: 1280, height: 720 }, { mode: "height", height: 2160 })).toEqual({ width: 1280, height: 720 });
    expect(resolveOutputSize({ width: 640, height: 360 }, { mode: "custom", width: 1920, height: 1080 })).toEqual({ width: 640, height: 360 });
  });
  it("fits custom box", () => {
    expect(resolveOutputSize({ width: 1920, height: 1080 }, { mode: "custom", width: 640, height: 640 })).toEqual({ width: 640, height: 360 });
    expect(resolveOutputSize({ width: 1080, height: 2400 }, { mode: "custom", width: 1280, height: 720 })).toEqual({ width: 324, height: 720 });
  });
  it("rejects invalid source", () => {
    expect(() => resolveOutputSize({ width: 0, height: 10 }, { mode: "source" })).toThrow(RangeError);
  });
});

describe("bitrate and size", () => {
  it("computes preset bitrates", () => {
    expect(videoBitrateKbps({ width: 1920, height: 1080 }, 30, "h264", "medium")).toBe(4977);
    expect(videoBitrateKbps({ width: 1920, height: 1080 }, 30, "h265", "medium")).toBe(3110);
  });
  it("clamps", () => {
    expect(videoBitrateKbps({ width: 320, height: 240 }, 24, "h264", "low")).toBe(500);
    expect(videoBitrateKbps({ width: 7680, height: 4320 }, 60, "h264", "ultra")).toBe(100000);
  });
  it("custom requires value", () => {
    expect(videoBitrateKbps({ width: 1, height: 1 }, 30, "h264", "custom", 8000)).toBe(8000);
    expect(() => videoBitrateKbps({ width: 1, height: 1 }, 30, "h264", "custom")).toThrow();
  });
  it("estimates file size", () => {
    // 5160 kbps * 60 s = 38.7 MB + 2%
    expect(estimateFileSizeBytes(5000, 160, 60)).toBe(39474000);
    expect(() => estimateFileSizeBytes(1, 1, -1)).toThrow();
  });
  it("formats", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(250 * 1024 * 1024)).toBe("250 MB");
    expect(formatDuration(3_723_000)).toBe("1:02:03");
    expect(formatDuration(65_400)).toBe("01:05");
  });
});

describe("file names", () => {
  it("expands tokens", () => {
    const d = new Date(2026, 10, 4, 9, 5, 7);
    expect(expandFileName("CaptureOne {date} {time} {source} {n}", { date: d, source: "Display 1", n: 3 })).toBe("CaptureOne 2026-11-04 09-05-07 Display 1 3");
  });
  it("sanitizes", () => {
    expect(sanitizeFileName('a<b>c:"d/e\\f|g?h*')).toBe("a_b_c__d_e_f_g_h_");
    expect(sanitizeFileName("CON")).toBe("_CON");
    expect(sanitizeFileName("  ...  ")).toBe("CaptureOne");
    expect(sanitizeFileName("x".repeat(300))).toHaveLength(120);
  });
});

describe("app settings", () => {
  it("has defaults", () => {
    const s = defaultSettings("/tmp/out");
    expect(s.outputDirectory).toBe("/tmp/out");
    expect(s.video.codec).toBe("h264");
    expect(s.shortcuts.startStop).toContain("Shift+R");
    expect(AppSettings.safeParse(s).success).toBe(true);
  });
  it("falls back on corrupt data", () => {
    expect(loadSettings({ video: { fps: 999 } }, "/x").video.fps).toBe(30);
    expect(loadSettings("garbage", "/y").outputDirectory).toBe("/y");
  });
});
