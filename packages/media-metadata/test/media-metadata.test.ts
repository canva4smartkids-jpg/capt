import { describe, expect, it } from "vitest";
import type { MediaMetadata } from "@captureone/capture-contracts";
import {
  audioSummary,
  bitrateLabel,
  displayTitle,
  exportFileName,
  formatLabel,
  formatResolution,
  parseMediaList,
  queryLibrary,
  removeMedia,
  resolutionName,
  storageRatio,
  summarizeLibrary,
  upsertMedia,
  validateTitle,
  validationBadge,
} from "../src/index.js";

const base: MediaMetadata = {
  id: "a",
  kind: "video",
  path: "/v/CaptureOne 2026-11-04 10-00-00.mp4",
  fileName: "CaptureOne 2026-11-04 10-00-00.mp4",
  title: "",
  createdAt: "2026-11-04T10:00:00Z",
  sizeBytes: 5_000_000,
  width: 1920,
  height: 1080,
  durationMs: 65_000,
  fps: 30,
  videoCodec: "h264",
  videoBitrateKbps: 4976,
  audioTracks: [{ index: 1, codec: "aac", channels: 2, sampleRate: 48000, label: "microphone" }],
  container: "mp4",
  validation: { valid: true, checkedAt: "2026-11-04T10:01:05Z", tool: "ffprobe 6.1", issues: [] },
};
const shot: MediaMetadata = {
  ...base,
  id: "b",
  kind: "screenshot",
  path: "/v/Shot.png",
  fileName: "Shot.png",
  title: "Bug report",
  createdAt: "2026-11-05T09:00:00Z",
  sizeBytes: 300_000,
  durationMs: undefined,
  fps: undefined,
  videoCodec: undefined,
  videoBitrateKbps: undefined,
  audioTracks: [],
  imageFormat: "png",
  container: "png",
};

describe("labels", () => {
  it("formats titles, formats, resolution and bitrate", () => {
    expect(displayTitle(base)).toBe("CaptureOne 2026-11-04 10-00-00");
    expect(displayTitle(shot)).toBe("Bug report");
    expect(formatLabel(base)).toBe("H.264");
    expect(formatLabel({ ...base, videoCodec: "h265" })).toBe("H.265");
    expect(formatLabel(shot)).toBe("PNG");
    expect(formatResolution(base)).toBe("1920×1080");
    expect(resolutionName(base)).toBe("1080p");
    expect(resolutionName({ width: 1000, height: 333 })).toBeUndefined();
    expect(bitrateLabel(base)).toBe("5.0 Mbps");
    expect(bitrateLabel({ videoBitrateKbps: 640 })).toBe("640 kbps");
    expect(bitrateLabel({})).toBeUndefined();
  });
  it("summarises audio tracks", () => {
    expect(audioSummary(base)).toBe("1 audio track (microphone)");
    expect(audioSummary({ ...base, audioTracks: [] })).toBe("No audio");
    expect(audioSummary(shot)).toBe("");
  });
});

describe("validationBadge", () => {
  it("only reports Validated for a passing inspection", () => {
    expect(validationBadge(base).label).toBe("Validated");
    expect(validationBadge({ validation: undefined }).label).toBe("Not validated");
    expect(
      validationBadge({ validation: { valid: false, checkedAt: "x", tool: "ffprobe", issues: [{ severity: "error", code: "NO_VIDEO", message: "No video stream" }] } }),
    ).toMatchObject({ tone: "danger", label: "Invalid" });
    expect(
      validationBadge({ validation: { valid: true, checkedAt: "x", tool: "ffprobe", issues: [{ severity: "warning", code: "SHORT", message: "Very short" }] } }),
    ).toMatchObject({ tone: "warning", detail: "Very short" });
  });
});

describe("queryLibrary", () => {
  const items = [base, shot, { ...base, id: "c", title: "Demo 4K", width: 3840, height: 2160, createdAt: "2026-11-01T00:00:00Z", sizeBytes: 9e7 }];
  it("filters by kind and every search term", () => {
    expect(queryLibrary(items, { filter: "screenshot", search: "", sort: "newest" }).map((m) => m.id)).toEqual(["b"]);
    expect(queryLibrary(items, { filter: "all", search: "demo 4k", sort: "newest" }).map((m) => m.id)).toEqual(["c"]);
    expect(queryLibrary(items, { filter: "all", search: "1080p", sort: "newest" }).map((m) => m.id)).toEqual(["b", "a"]);
  });
  it("sorts", () => {
    expect(queryLibrary(items, { filter: "all", search: "", sort: "newest" }).map((m) => m.id)).toEqual(["b", "a", "c"]);
    expect(queryLibrary(items, { filter: "all", search: "", sort: "oldest" }).map((m) => m.id)).toEqual(["c", "a", "b"]);
    expect(queryLibrary(items, { filter: "all", search: "", sort: "largest" }).map((m) => m.id)).toEqual(["c", "a", "b"]);
    expect(queryLibrary(items, { filter: "all", search: "", sort: "name" }).map((m) => m.id)).toEqual(["b", "a", "c"]);
  });
});

describe("list helpers", () => {
  it("parses untrusted lists and drops invalid entries", () => {
    const { items, rejected } = parseMediaList([base, { id: "" }, shot]);
    expect(items.map((m) => m.id)).toEqual(["a", "b"]);
    expect(rejected).toBe(1);
    expect(parseMediaList("nope").items).toEqual([]);
  });
  it("upserts and removes", () => {
    const renamed = { ...base, title: "New" };
    expect(upsertMedia([base, shot], renamed)[0]?.title).toBe("New");
    expect(upsertMedia([shot], base).map((m) => m.id)).toEqual(["a", "b"]);
    expect(removeMedia([base, shot], "a").map((m) => m.id)).toEqual(["b"]);
  });
  it("summarises and computes storage ratio", () => {
    expect(summarizeLibrary([base, shot])).toMatchObject({ total: 2, videos: 1, screenshots: 1, totalBytes: 5_300_000, totalDurationMs: 65_000 });
    expect(storageRatio(25, 75)).toBe(0.25);
    expect(storageRatio(0, 0)).toBe(0);
  });
  it("builds export names and validates titles", () => {
    expect(exportFileName(shot)).toBe("Bug report.png");
    expect(exportFileName({ title: 'a/b:c', fileName: "x.mp4" })).toBe("a_b_c.mp4");
    expect(validateTitle("  ")).not.toBeNull();
    expect(validateTitle("x".repeat(121))).not.toBeNull();
    expect(validateTitle("Fine")).toBeNull();
  });
});
