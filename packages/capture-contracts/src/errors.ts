import { z } from "zod";

export const CaptureErrorCode = z.enum([
  "PERMISSION_DENIED",
  "UNSUPPORTED",
  "SOURCE_NOT_FOUND",
  /** The captured window/display disappeared during recording. */
  "SOURCE_CLOSED",
  "ENCODER_UNAVAILABLE",
  "ENCODER_FAILED",
  "STORAGE_FULL",
  "IO_ERROR",
  "INVALID_CONFIG",
  /** Operation not valid in the current recording state (e.g. pause while idle). */
  "INVALID_STATE",
  /** Content is protected (FLAG_SECURE, HDCP/DRM) and was blanked by the OS. */
  "DRM_PROTECTED",
  /** Recording was interrupted (system revoked projection, device locked, app killed). */
  "INTERRUPTED",
  "DEVICE_BUSY",
  "NETWORK",
  "INTERNAL",
]);
export type CaptureErrorCode = z.infer<typeof CaptureErrorCode>;

export const CaptureError = z.object({
  code: CaptureErrorCode,
  message: z.string(),
  /** Whether retrying the same operation can succeed without user action. */
  recoverable: z.boolean(),
  details: z.record(z.string(), z.unknown()).optional(),
});
export type CaptureError = z.infer<typeof CaptureError>;

export class CaptureFailure extends Error {
  readonly code: CaptureErrorCode;
  readonly recoverable: boolean;
  readonly details: Record<string, unknown> | undefined;
  constructor(err: CaptureError) {
    super(err.message);
    this.name = "CaptureFailure";
    this.code = err.code;
    this.recoverable = err.recoverable;
    this.details = err.details;
  }
  toJSON(): CaptureError {
    return { code: this.code, message: this.message, recoverable: this.recoverable, ...(this.details ? { details: this.details } : {}) };
  }
}

/** Normalises anything thrown across an IPC boundary into a CaptureError. */
export function toCaptureError(e: unknown): CaptureError {
  const parsed = CaptureError.safeParse(e);
  if (parsed.success) return parsed.data;
  if (e instanceof CaptureFailure) return e.toJSON();
  if (e instanceof Error) return { code: "INTERNAL", message: e.message, recoverable: false };
  if (typeof e === "string") {
    try {
      const fromJson = CaptureError.safeParse(JSON.parse(e));
      if (fromJson.success) return fromJson.data;
    } catch {
      /* not JSON */
    }
    return { code: "INTERNAL", message: e, recoverable: false };
  }
  return { code: "INTERNAL", message: "Unknown error", recoverable: false };
}

const USER_MESSAGES: Record<CaptureErrorCode, string> = {
  PERMISSION_DENIED: "Permission was denied. Grant access in system settings and try again.",
  UNSUPPORTED: "This feature is not supported on this device.",
  SOURCE_NOT_FOUND: "The selected screen or window is no longer available.",
  SOURCE_CLOSED: "The captured screen or window was closed. The recording was saved up to that point.",
  ENCODER_UNAVAILABLE: "No suitable video encoder is available. Try H.264 or a lower resolution.",
  ENCODER_FAILED: "The video encoder failed. Try a lower resolution or frame rate.",
  STORAGE_FULL: "Not enough free disk space. Free up space or choose another output folder.",
  IO_ERROR: "A file could not be read or written.",
  INVALID_CONFIG: "The recording settings are invalid.",
  INVALID_STATE: "That action is not available right now.",
  DRM_PROTECTED: "Protected content cannot be captured.",
  INTERRUPTED: "The recording was interrupted. The partial recording was saved when possible.",
  DEVICE_BUSY: "The capture device is in use by another application.",
  NETWORK: "Network error while communicating with the device.",
  INTERNAL: "An unexpected error occurred.",
};

export function userMessage(code: CaptureErrorCode): string {
  return USER_MESSAGES[code];
}
