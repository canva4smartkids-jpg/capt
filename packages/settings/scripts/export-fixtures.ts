/** Writes cross-language contract fixtures consumed by the Rust and Kotlin test suites. */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { QualityPreset, ResolutionSetting, VideoCodec } from "@captureone/capture-contracts";
import { estimateFileSizeBytes, resolveOutputSize, videoBitrateKbps } from "../src/presets.js";

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "..", "..", "..", "tests", "contract", "fixtures");
mkdirSync(out, { recursive: true });

const sources = [
  { width: 1920, height: 1080 },
  { width: 2560, height: 1440 },
  { width: 3840, height: 2160 },
  { width: 1366, height: 768 },
  { width: 1279, height: 719 },
  { width: 3024, height: 1964 },
  { width: 1080, height: 2400 },
  { width: 801, height: 601 },
];
const settings: ResolutionSetting[] = [
  { mode: "source" },
  { mode: "height", height: 2160 },
  { mode: "height", height: 1080 },
  { mode: "height", height: 720 },
  { mode: "height", height: 480 },
  { mode: "custom", width: 1280, height: 720 },
  { mode: "custom", width: 640, height: 640 },
];
const resolution = sources.flatMap((source) => settings.map((setting) => ({ source, setting, expected: resolveOutputSize(source, setting) })));

const codecs: VideoCodec[] = ["h264", "h265"];
const qualities: QualityPreset[] = ["low", "medium", "high", "ultra"];
const sizes = [
  { width: 1280, height: 720 },
  { width: 1920, height: 1080 },
  { width: 3840, height: 2160 },
  { width: 320, height: 240 },
];
const bitrate = sizes.flatMap((size) =>
  [24, 30, 60].flatMap((fps) => codecs.flatMap((codec) => qualities.map((quality) => ({ size, fps, codec, quality, expectedKbps: videoBitrateKbps(size, fps, codec, quality) })))),
);
const fileSize = [
  { videoKbps: 5000, audioKbps: 160, durationSec: 60 },
  { videoKbps: 30000, audioKbps: 320, durationSec: 3600 },
  { videoKbps: 500, audioKbps: 0, durationSec: 1 },
].map((c) => ({ ...c, expectedBytes: estimateFileSizeBytes(c.videoKbps, c.audioKbps, c.durationSec) }));

writeFileSync(join(out, "presets-cases.json"), JSON.stringify({ resolution, bitrate, fileSize }, null, 2) + "\n");
console.log(`fixtures: ${resolution.length} resolution, ${bitrate.length} bitrate, ${fileSize.length} file-size cases -> ${out}`);
