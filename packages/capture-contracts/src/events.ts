import { z } from "zod";
import { RecordingState, RecordingStatus, StopReason } from "./recording.js";
import { CaptureError } from "./errors.js";
import { MediaMetadata } from "./media.js";

export const CaptureEvent = z.discriminatedUnion("type", [
  z.object({ type: z.literal("state-changed"), sessionId: z.string(), from: RecordingState, to: RecordingState, at: z.string() }),
  z.object({ type: z.literal("status"), status: RecordingStatus }),
  z.object({ type: z.literal("countdown"), sessionId: z.string(), remainingSec: z.number().int().nonnegative() }),
  z.object({ type: z.literal("warning"), sessionId: z.string().optional(), message: z.string() }),
  z.object({ type: z.literal("error"), sessionId: z.string().optional(), error: CaptureError }),
  z.object({ type: z.literal("completed"), sessionId: z.string(), reason: StopReason, media: MediaMetadata }),
  z.object({ type: z.literal("screenshot"), media: MediaMetadata }),
]);
export type CaptureEvent = z.infer<typeof CaptureEvent>;
