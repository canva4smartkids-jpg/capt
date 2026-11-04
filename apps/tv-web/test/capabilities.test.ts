import { CapabilityReport, featureLevel, FeatureKey, PairRequest, isUsable } from "@captureone/capture-contracts";
import { describe, expect, it } from "vitest";
import { buildTvCapabilityReport, DISPLAYED_FEATURES, FEATURE_LABELS, tvFeatures } from "../src/capabilities.js";

const info = { manufacturer: "Samsung", model: "QE55Q80T", osName: "Tizen", osVersion: "6.0", sources: ["tizen.systeminfo"] };

describe("TV capability report", () => {
  for (const id of ["tizen", "webos", "browser"] as const) {
    it(`${id}: parses with the contract's zod schema`, () => {
      const report = buildTvCapabilityReport(id, info, "2026-11-04T10:00:00.000Z");
      const parsed = CapabilityReport.parse(report);
      expect(parsed.platform).toBe(id === "browser" ? "other-tv" : id);
      expect(parsed.deviceModel).toBe("QE55Q80T");
      // Sent inside PairRequest.capabilities.
      const pr = PairRequest.parse({ code: "123456", deviceName: "TV", deviceKind: "samsung-tizen-tv", capabilities: report });
      expect(pr.capabilities?.platform).toBe(parsed.platform);
    });

    it(`${id}: never claims the TV can capture or record itself`, () => {
      const report = buildTvCapabilityReport(id, info);
      expect(featureLevel(report, "displayCapture")).toBe("external-hardware");
      expect(featureLevel(report, "hdmiCaptureDevice")).toBe("external-hardware");
      expect(featureLevel(report, "videoRecording")).toBe("companion-only");
      expect(featureLevel(report, "screenshot")).toBe("companion-only");
      for (const k of ["displayCapture", "videoRecording", "screenshot", "windowCapture", "regionCapture", "systemAudio", "microphoneAudio"] as const) {
        expect(isUsable(report, k)).toBe(false);
      }
      expect(report.encoders).toEqual([]);
      expect(report.screenshotFormats).toEqual([]);
      for (const [key, f] of Object.entries(report.features)) {
        if (f && f.level !== "native") expect(f.reason, key).toBeTruthy();
      }
    });
  }

  it("LG native screenshot is partial and labelled experimental/undocumented; Samsung cites its FAQ", () => {
    const lg = tvFeatures("webos").tvNativeScreenshot!;
    expect(lg.level).toBe("partial");
    expect(lg.reason).toMatch(/^Experimental, undocumented/);
    expect(lg.reason).toMatch(/not verified on real hardware/);
    const sam = tvFeatures("tizen");
    expect(sam.tvNativeScreenshot!.level).toBe("unsupported");
    expect(sam.tvNativeScreenshot!.reason).toMatch(/Samsung's Smart TV FAQ/);
    expect(sam.displayCapture!.reason).toMatch(/Samsung's Smart TV FAQ states that screen capture is not supported/);
    expect(tvFeatures("webos").displayCapture!.reason).toMatch(/LG's developer forum/);
  });

  it("labels every displayed feature", () => {
    for (const k of DISPLAYED_FEATURES) {
      expect(FeatureKey.options).toContain(k);
      expect(FEATURE_LABELS[k]).toBeTruthy();
    }
  });
});
