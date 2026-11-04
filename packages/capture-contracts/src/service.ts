import type { CapabilityReport } from "./platform.js";
import type { PermissionKind, PermissionStatus } from "./permissions.js";
import type { CaptureSource, SourceKind } from "./sources.js";
import type { RecordingConfigInput, RecordingStatus } from "./recording.js";
import type { ScreenshotRequestInput } from "./screenshot.js";
import type { MediaMetadata } from "./media.js";
import type { CaptureEvent } from "./events.js";

/**
 * The shared capture abstraction. Desktop (Tauri/Rust), Android (Kotlin) and the
 * TV companions implement this surface. Methods for unsupported features must
 * reject with a CaptureError of code UNSUPPORTED rather than pretend to work.
 */
export interface CaptureService {
  getCapabilities(): Promise<CapabilityReport>;
  getPermissionStatus(kind: PermissionKind): Promise<PermissionStatus>;
  requestPermission(kind: PermissionKind): Promise<PermissionStatus>;
  listSources(kinds?: SourceKind[]): Promise<CaptureSource[]>;
  takeScreenshot(request: ScreenshotRequestInput): Promise<MediaMetadata>;
  /** Resolves once the session exists (it may still be counting down). Returns the session id. */
  startRecording(config: RecordingConfigInput): Promise<string>;
  pauseRecording(sessionId: string): Promise<void>;
  resumeRecording(sessionId: string): Promise<void>;
  /** Resolves with validated metadata of the finished file. */
  stopRecording(sessionId: string): Promise<MediaMetadata>;
  /** Cancels a countdown or discards an active recording. */
  cancelRecording(sessionId: string): Promise<void>;
  getStatus(sessionId: string): Promise<RecordingStatus>;
  subscribe(listener: (event: CaptureEvent) => void): () => void;
}

/** Library management surface (history, rename, delete, export). */
export interface RecordingLibrary {
  list(): Promise<MediaMetadata[]>;
  get(id: string): Promise<MediaMetadata | null>;
  rename(id: string, title: string): Promise<MediaMetadata>;
  remove(id: string, deleteFile: boolean): Promise<void>;
  exportTo(id: string, destinationPath: string): Promise<string>;
  storageUsage(): Promise<{ totalBytes: number; videoCount: number; screenshotCount: number; freeDiskBytes: number }>;
}
