/**
 * Exports JSON Schema (draft 2020-12) for every contract type plus fixtures
 * (state machine table) so Rust and Kotlin implementations can be contract-tested.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import * as c from "../src/index.js";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "..", "schema");
mkdirSync(outDir, { recursive: true });

const schemas: Record<string, z.ZodType> = {
  CapabilityReport: c.CapabilityReport,
  PermissionStatus: c.PermissionStatus,
  CaptureSource: c.CaptureSource,
  CaptureTarget: c.CaptureTarget,
  CaptureError: c.CaptureError,
  RecordingConfig: c.RecordingConfig,
  RecordingStatus: c.RecordingStatus,
  ScreenshotRequest: c.ScreenshotRequest,
  MediaMetadata: c.MediaMetadata,
  CaptureEvent: c.CaptureEvent,
  DiscoveredDevice: c.DiscoveredDevice,
  HubInfo: c.HubInfo,
  PairRequest: c.PairRequest,
  PairResponse: c.PairResponse,
  RemoteCommand: c.RemoteCommand,
  HubEvent: c.HubEvent,
};

for (const [name, schema] of Object.entries(schemas)) {
  // io: "input" so defaulted fields are optional for producers.
  const json = z.toJSONSchema(schema, { target: "draft-2020-12", io: "input", unrepresentable: "any" });
  writeFileSync(join(outDir, `${name}.schema.json`), JSON.stringify({ $id: `https://captureone.dev/schema/${name}.json`, title: name, ...json }, null, 2) + "\n");
}

writeFileSync(join(outDir, "recording-transitions.json"), JSON.stringify(c.RECORDING_TRANSITIONS, null, 2) + "\n");
writeFileSync(
  join(outDir, "enums.json"),
  JSON.stringify(
    {
      Platform: c.Platform.options,
      SupportLevel: c.SupportLevel.options,
      FeatureKey: c.FeatureKey.options,
      PermissionKind: c.PermissionKind.options,
      PermissionState: c.PermissionState.options,
      CaptureErrorCode: c.CaptureErrorCode.options,
      RecordingState: c.RecordingState.options,
      DeviceKind: c.DeviceKind.options,
      ControlProtocol: c.ControlProtocol.options,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Wrote ${Object.keys(schemas).length} schemas to ${outDir}`);
