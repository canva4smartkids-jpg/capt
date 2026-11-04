/**
 * Desktop IPC contract: the exact Tauri command names, argument objects and return
 * types shared by the React UI (apps/desktop/src) and the Rust core
 * (apps/desktop/src-tauri). Tauri maps camelCase JS argument keys to snake_case
 * Rust parameters automatically. Errors are rejected as a serialized CaptureError.
 */
import type { CapabilityReport } from "./platform.js";
import type { PermissionKind, PermissionStatus } from "./permissions.js";
import type { CaptureSource, Rect, SourceKind } from "./sources.js";
import type { RecordingConfigInput, RecordingStatus } from "./recording.js";
import type { ScreenshotRequestInput } from "./screenshot.js";
import type { MediaMetadata } from "./media.js";
import type { DiscoveredDevice } from "./companion.js";

export interface AudioDevice {
  id: string;
  name: string;
  /** "input" = microphone/line-in; "loopback" = system audio source. */
  kind: "input" | "loopback";
  isDefault: boolean;
}

export interface StorageUsage {
  totalBytes: number;
  videoCount: number;
  screenshotCount: number;
  freeDiskBytes: number;
  outputDirectory: string;
}

export interface Diagnostics {
  appVersion: string;
  platform: string;
  osVersion: string;
  ffmpegPath: string | null;
  ffmpegVersion: string | null;
  ffprobePath: string | null;
  dataDirectory: string;
  logDirectory: string;
  databasePath: string;
}

export interface HubStatus {
  enabled: boolean;
  /** e.g. http://192.168.1.20:47800 */
  url: string | null;
  pairingCode: string | null;
  pairedDevices: { name: string; deviceKind: string; pairedAt: string }[];
}

export type TvRemoteKey =
  | "UP" | "DOWN" | "LEFT" | "RIGHT" | "ENTER" | "BACK" | "HOME" | "MENU"
  | "VOLUME_UP" | "VOLUME_DOWN" | "MUTE" | "PLAY" | "PAUSE" | "STOP" | "POWER";

/** Persisted settings use the AppSettings schema from @captureone/settings (opaque here). */
export type AppSettingsJson = Record<string, unknown>;

/** Command name -> [args, result]. */
export interface DesktopCommands {
  get_capabilities: [Record<string, never>, CapabilityReport];
  get_permission_status: [{ kind: PermissionKind }, PermissionStatus];
  request_permission: [{ kind: PermissionKind }, PermissionStatus];
  open_permission_settings: [{ kind: PermissionKind }, void];
  list_sources: [{ kinds?: SourceKind[] }, CaptureSource[]];
  /** PNG data URL (data:image/png;base64,...) scaled to maxWidth. */
  get_source_thumbnail: [{ kind: SourceKind; sourceId: string; maxWidth: number }, string];
  list_audio_devices: [Record<string, never>, AudioDevice[]];

  take_screenshot: [{ request: ScreenshotRequestInput }, MediaMetadata];
  /** Starts countdown (if any) then recording; progress is reported via CAPTURE_EVENT. */
  start_recording: [{ config: RecordingConfigInput }, string];
  pause_recording: [{ sessionId: string }, void];
  resume_recording: [{ sessionId: string }, void];
  stop_recording: [{ sessionId: string }, MediaMetadata];
  cancel_recording: [{ sessionId: string }, void];
  get_recording_status: [{ sessionId: string }, RecordingStatus];
  get_active_session: [Record<string, never>, RecordingStatus | null];

  /** Opens the transparent full-screen region selector on a display; resolves with the rect or null if cancelled. */
  select_region: [{ displayId: string }, Rect | null];
  /** Called by the region-selector window itself. */
  region_selection_done: [{ rect: Rect | null }, void];

  library_list: [Record<string, never>, MediaMetadata[]];
  /** `renameFile: true` also renames the file on disk (never overwrites; " (n)" suffix on clash). */
  library_rename: [{ id: string; title: string; renameFile?: boolean }, MediaMetadata];
  library_delete: [{ id: string; deleteFile: boolean }, void];
  /** Copies the file to `destination` (full path chosen with the save dialog). Returns the written path. */
  library_export: [{ id: string; destination: string }, string];
  library_reveal: [{ id: string }, void];
  /**
   * Linux only: tokenised http://127.0.0.1 URL for in-app `<video>` preview (WebKitGTK cannot
   * play from the asset protocol). Returns null where the asset protocol is used instead.
   */
  media_stream_url: [{ id: string }, string | null];
  library_validate: [{ id: string }, MediaMetadata];
  library_storage: [Record<string, never>, StorageUsage];
  /** Finds and finalizes recordings interrupted by a crash/power loss. */
  recover_recordings: [Record<string, never>, MediaMetadata[]];

  get_settings: [Record<string, never>, AppSettingsJson];
  save_settings: [{ settings: AppSettingsJson }, AppSettingsJson];
  get_diagnostics: [Record<string, never>, Diagnostics];

  hub_status: [Record<string, never>, HubStatus];
  hub_set_enabled: [{ enabled: boolean }, HubStatus];
  hub_regenerate_code: [Record<string, never>, HubStatus];
  discovery_scan: [{ timeoutMs: number }, DiscoveredDevice[]];
  tv_pair: [{ deviceId: string }, { paired: boolean; message: string }];
  tv_send_key: [{ deviceId: string; key: TvRemoteKey }, void];
  /** LG webOS only (SSAP one-shot screenshot). Rejects UNSUPPORTED elsewhere. */
  tv_screenshot: [{ deviceId: string }, MediaMetadata];
}

export type DesktopCommand = keyof DesktopCommands;
export type CommandArgs<C extends DesktopCommand> = DesktopCommands[C][0];
export type CommandResult<C extends DesktopCommand> = DesktopCommands[C][1];

/** Tauri event names. Payload of CAPTURE_EVENT is a CaptureEvent. */
export const DESKTOP_EVENTS = {
  CAPTURE_EVENT: "capture://event",
  /** Payload: { action: "start-stop" | "pause-resume" | "screenshot" | "region-screenshot" } from global shortcuts / tray. */
  SHORTCUT: "app://shortcut",
  HUB_EVENT: "hub://event",
} as const;

/** Tauri window labels and the hash route each one loads. */
export const DESKTOP_WINDOWS = {
  main: { label: "main", route: "#/" },
  indicator: { label: "indicator", route: "#/indicator" },
  region: { label: "region", route: "#/region" },
} as const;
