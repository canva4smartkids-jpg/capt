import { z } from "zod";
import { CapabilityReport, Platform } from "./platform.js";
import { MediaMetadata } from "./media.js";
import { RecordingStatus } from "./recording.js";

/**
 * Companion protocol between the CaptureOne desktop "hub" and TV / mobile
 * companion apps on the same LAN. HTTP+JSON for requests, WebSocket for status.
 * The hub is disabled by default and requires pairing with a short code.
 */
export const COMPANION_PROTOCOL_VERSION = 1 as const;
export const COMPANION_DEFAULT_PORT = 47800 as const;

export const DeviceKind = z.enum([
  "desktop",
  "android-phone",
  "android-tablet",
  "android-tv",
  "google-tv",
  "samsung-tizen-tv",
  "lg-webos-tv",
  "other-tv",
  "unknown",
]);
export type DeviceKind = z.infer<typeof DeviceKind>;

export const ControlProtocol = z.enum([
  "captureone-companion",
  "lg-ssap",
  "samsung-remote-ws",
  "android-tv-remote",
  "dial",
  "upnp-media-renderer",
]);
export type ControlProtocol = z.infer<typeof ControlProtocol>;

export const DiscoveredDevice = z.object({
  id: z.string().min(1),
  name: z.string(),
  kind: DeviceKind,
  manufacturer: z.string().optional(),
  model: z.string().optional(),
  address: z.string(),
  port: z.number().int().positive().optional(),
  discoveredVia: z.array(z.enum(["ssdp", "mdns", "manual"])),
  controlProtocols: z.array(ControlProtocol),
  /** Raw service identifiers (SSDP ST/USN or mDNS service types). */
  services: z.array(z.string()),
  lastSeen: z.string(),
});
export type DiscoveredDevice = z.infer<typeof DiscoveredDevice>;

export const HubInfo = z.object({
  protocolVersion: z.literal(COMPANION_PROTOCOL_VERSION),
  hubId: z.string(),
  name: z.string(),
  platform: Platform,
  appVersion: z.string(),
  capabilities: CapabilityReport,
});
export type HubInfo = z.infer<typeof HubInfo>;

export const PairRequest = z.object({
  code: z.string().regex(/^\d{6}$/),
  deviceName: z.string().min(1).max(64),
  deviceKind: DeviceKind,
  /** The companion's own capability report (e.g. a TV reporting it cannot capture). */
  capabilities: CapabilityReport.optional(),
});
export type PairRequest = z.infer<typeof PairRequest>;

export const PairResponse = z.object({
  token: z.string().min(32),
  hub: HubInfo,
});
export type PairResponse = z.infer<typeof PairResponse>;

export const RemoteCommand = z.discriminatedUnion("type", [
  z.object({ type: z.literal("start-recording"), preset: z.enum(["default", "capture-device"]).default("default") }),
  z.object({ type: z.literal("pause-recording") }),
  z.object({ type: z.literal("resume-recording") }),
  z.object({ type: z.literal("stop-recording") }),
  z.object({ type: z.literal("screenshot") }),
]);
export type RemoteCommand = z.infer<typeof RemoteCommand>;

export const RemoteCommandResult = z.object({
  ok: z.boolean(),
  sessionId: z.string().optional(),
  media: MediaMetadata.optional(),
  error: z.string().optional(),
});
export type RemoteCommandResult = z.infer<typeof RemoteCommandResult>;

export const HubEvent = z.discriminatedUnion("type", [
  z.object({ type: z.literal("hello"), hub: HubInfo }),
  z.object({ type: z.literal("recording-status"), status: RecordingStatus.nullable() }),
  z.object({ type: z.literal("library-changed") }),
]);
export type HubEvent = z.infer<typeof HubEvent>;

/** REST surface of the hub. All routes except `info` and `pair` require `Authorization: Bearer <token>`. */
export const COMPANION_ROUTES = {
  info: "GET /api/v1/info",
  pair: "POST /api/v1/pair",
  recordings: "GET /api/v1/recordings",
  recordingFile: "GET /api/v1/recordings/:id/file",
  recordingThumbnail: "GET /api/v1/recordings/:id/thumbnail",
  status: "GET /api/v1/status",
  command: "POST /api/v1/command",
  events: "GET /api/v1/events (WebSocket upgrade)",
} as const;

export const RecordingListResponse = z.object({ items: z.array(MediaMetadata) });
export type RecordingListResponse = z.infer<typeof RecordingListResponse>;
