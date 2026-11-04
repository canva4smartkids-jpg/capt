import { z } from "zod";

export const PermissionKind = z.enum([
  /** Screen recording (macOS TCC "Screen Recording", Android MediaProjection consent). */
  "screenCapture",
  "microphone",
  /** System/internal audio (Android AudioPlaybackCapture, macOS SCK audio). */
  "systemAudio",
  /** Writing to shared storage (legacy Android < 10). */
  "storage",
  /** Foreground service / recording notification (Android 13+ POST_NOTIFICATIONS). */
  "notifications",
  /** LAN access for companion discovery (iOS-style local network prompts, Android NEARBY_WIFI_DEVICES). */
  "localNetwork",
]);
export type PermissionKind = z.infer<typeof PermissionKind>;

export const PermissionState = z.enum([
  "granted",
  "denied",
  /** Not yet asked; requesting will show a system prompt. */
  "prompt",
  /** Blocked by policy (MDM, parental controls) — user cannot grant it. */
  "restricted",
  /** Platform has no such permission (e.g. Windows screen capture). */
  "not-applicable",
  /** Consent is per-session (Android MediaProjection): must be requested for each recording. */
  "per-session",
]);
export type PermissionState = z.infer<typeof PermissionState>;

export const PermissionStatus = z.object({
  kind: PermissionKind,
  state: PermissionState,
  /** True when the user must open system settings to change the state. */
  requiresSystemSettings: z.boolean().default(false),
  message: z.string().optional(),
});
export type PermissionStatus = z.infer<typeof PermissionStatus>;
