/**
 * Typed IPC layer over Tauri. Every command name, argument object and result type
 * comes from `DesktopCommands` in @captureone/capture-contracts, so a mismatch with
 * the contract is a compile error. Results are validated with the contract's zod
 * schemas at the boundary where that is cheap, and every rejection is normalised to
 * a `CaptureFailure` carrying the contract `CaptureError`.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  CapabilityReport,
  CaptureEvent,
  CaptureFailure,
  CaptureSource,
  DESKTOP_EVENTS,
  MediaMetadata,
  PermissionStatus,
  Rect,
  RecordingStatus,
  toCaptureError,
  type CommandArgs,
  type DesktopCommands,
} from "@captureone/capture-contracts";
import { isShortcutEventPayload, type ShortcutEventPayload } from "@captureone/shared-types";
import { z } from "zod";

/**
 * Additive, backwards-compatible extensions over the contract (proposed upstream):
 * `library_rename` accepts an optional `renameFile` flag; omitting it keeps the
 * contract behaviour (title only).
 */
export interface DesktopCommandExtensions {
  library_rename: [CommandArgs<"library_rename"> & { renameFile?: boolean }, MediaMetadata];
}

export type AppCommands = Omit<DesktopCommands, keyof DesktopCommandExtensions> & DesktopCommandExtensions;
export type AppCommand = keyof AppCommands;
export type AppArgs<C extends AppCommand> = AppCommands[C][0];
export type AppResult<C extends AppCommand> = AppCommands[C][1];

type EmptyArgs = Record<string, never>;
/** Commands whose args are `Record<string, never>` may be invoked without an argument object. */
type ArgsParam<C extends AppCommand> = AppArgs<C> extends EmptyArgs ? [args?: AppArgs<C>] : [args: AppArgs<C>];

const mediaList = z.array(MediaMetadata);
const nullableStatus = RecordingStatus.nullable();
const nullableRect = Rect.nullable();

/** Boundary validators. Commands not listed pass through (void results, opaque settings JSON). */
const PARSERS: { [C in AppCommand]?: (raw: unknown) => AppResult<C> } = {
  get_capabilities: (r) => CapabilityReport.parse(r),
  get_permission_status: (r) => PermissionStatus.parse(r),
  request_permission: (r) => PermissionStatus.parse(r),
  list_sources: (r) => z.array(CaptureSource).parse(r),
  get_source_thumbnail: (r) => z.string().startsWith("data:image/").parse(r),
  take_screenshot: (r) => MediaMetadata.parse(r),
  start_recording: (r) => z.string().min(1).parse(r),
  stop_recording: (r) => MediaMetadata.parse(r),
  get_recording_status: (r) => RecordingStatus.parse(r),
  get_active_session: (r) => nullableStatus.parse(r),
  select_region: (r) => nullableRect.parse(r),
  library_list: (r) => mediaList.parse(r),
  library_rename: (r) => MediaMetadata.parse(r),
  library_validate: (r) => MediaMetadata.parse(r),
  recover_recordings: (r) => mediaList.parse(r),
  tv_screenshot: (r) => MediaMetadata.parse(r),
};

/** Type-checked `invoke`. Rejects with `CaptureFailure`. */
export async function invoke<C extends AppCommand>(cmd: C, ...rest: ArgsParam<C>): Promise<AppResult<C>> {
  const args = (rest[0] ?? {}) as Record<string, unknown>;
  let raw: unknown;
  try {
    raw = await tauriInvoke<unknown>(cmd, args);
  } catch (e) {
    throw new CaptureFailure(toCaptureError(e));
  }
  const parse = PARSERS[cmd] as ((r: unknown) => AppResult<C>) | undefined;
  if (!parse) return raw as AppResult<C>;
  try {
    return parse(raw);
  } catch (e) {
    const detail = e instanceof z.ZodError ? e.issues.map((i) => `${i.path.join(".")}: ${i.message}`).join("; ") : String(e);
    throw new CaptureFailure({ code: "INTERNAL", message: `Malformed response from '${cmd}': ${detail}`, recoverable: false });
  }
}

/** Normalises any thrown value into a CaptureFailure (for UI error handling). */
export function asFailure(e: unknown): CaptureFailure {
  return e instanceof CaptureFailure ? e : new CaptureFailure(toCaptureError(e));
}

export function isUnsupported(e: unknown): boolean {
  return asFailure(e).code === "UNSUPPORTED";
}

/** Subscribes to `capture://event`; payloads that do not match the contract are dropped and logged. */
export async function onCaptureEvent(handler: (event: CaptureEvent) => void): Promise<UnlistenFn> {
  return listen<unknown>(DESKTOP_EVENTS.CAPTURE_EVENT, (e) => {
    const parsed = CaptureEvent.safeParse(e.payload);
    if (parsed.success) handler(parsed.data);
    else console.warn("[ipc] dropped malformed capture event", parsed.error.issues, e.payload);
  });
}

export async function onShortcut(handler: (payload: ShortcutEventPayload) => void): Promise<UnlistenFn> {
  return listen<unknown>(DESKTOP_EVENTS.SHORTCUT, (e) => {
    if (isShortcutEventPayload(e.payload)) handler(e.payload);
  });
}

/** Runs an async subscription inside a React effect and returns a synchronous cleanup. */
export function subscribe(start: () => Promise<UnlistenFn>): () => void {
  let disposed = false;
  let unlisten: UnlistenFn | undefined;
  start()
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch((e) => console.warn("[ipc] subscription failed", e));
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** True when running inside the Tauri webview (false in plain browser previews / tests without mocks). */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
