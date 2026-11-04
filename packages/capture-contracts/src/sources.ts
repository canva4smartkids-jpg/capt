import { z } from "zod";

export const SourceKind = z.enum(["display", "window", "device"]);
export type SourceKind = z.infer<typeof SourceKind>;

export const Rect = z.object({
  x: z.number().int(),
  y: z.number().int(),
  width: z.number().int().positive(),
  height: z.number().int().positive(),
});
export type Rect = z.infer<typeof Rect>;

export const CaptureSource = z.object({
  /** Opaque, platform-specific identifier; stable for the lifetime of the source. */
  id: z.string().min(1),
  kind: SourceKind,
  name: z.string(),
  /** Pixel size of the source (physical pixels). */
  width: z.number().int().positive(),
  height: z.number().int().positive(),
  /**
   * Position in the virtual desktop (displays) or on screen (windows), in the OS's native
   * desktop coordinate units (physical pixels on Windows/X11, points on macOS).
   * `width`/`height` are always physical pixels.
   */
  x: z.number().int().optional(),
  y: z.number().int().optional(),
  scaleFactor: z.number().positive().optional(),
  isPrimary: z.boolean().optional(),
  /** Owning application for windows. */
  appName: z.string().optional(),
  /** For `device` sources (HDMI capture cards / webcams): backend specific device string. */
  deviceDescriptor: z.string().optional(),
});
export type CaptureSource = z.infer<typeof CaptureSource>;

export const CaptureTarget = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("display"), displayId: z.string().min(1) }),
  z.object({ kind: z.literal("window"), windowId: z.string().min(1) }),
  /**
   * Region in physical pixels relative to the top-left of the given display. Backends may
   * capture the region natively or capture the display and crop; either way frames delivered
   * to the engine have exactly the region size.
   */
  z.object({ kind: z.literal("region"), displayId: z.string().min(1), rect: Rect }),
  /** External video device such as an HDMI capture card. */
  z.object({ kind: z.literal("device"), deviceId: z.string().min(1) }),
]);
export type CaptureTarget = z.infer<typeof CaptureTarget>;
