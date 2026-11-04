/**
 * Remote-control key mapping. TV remotes deliver DOM keydown events whose `key` / `keyCode`
 * differ per platform: Samsung Tizen Back = 10009, LG webOS Back = 461, media keys must be
 * registered on Tizen (`tizen.tvinputdevice.registerKey`) before they are delivered.
 */

export type PlatformId = "tizen" | "webos" | "browser";
export type Direction = "up" | "down" | "left" | "right";
export type MediaCommand = "play" | "pause" | "playpause" | "stop" | "forward" | "rewind" | "next" | "previous";

export type KeyAction =
  | { type: "move"; dir: Direction }
  | { type: "enter" }
  | { type: "back" }
  | { type: "exit" }
  | { type: "digit"; digit: string }
  | { type: "media"; command: MediaCommand };

export interface KeyLike {
  key?: string;
  keyCode?: number;
}

const move = (dir: Direction): KeyAction => ({ type: "move", dir });
const media = (command: MediaCommand): KeyAction => ({ type: "media", command });

/** `KeyboardEvent.key` names (modern Chromium on newer TVs and desktop browsers). */
const BY_KEY: Record<string, KeyAction> = {
  ArrowUp: move("up"),
  ArrowDown: move("down"),
  ArrowLeft: move("left"),
  ArrowRight: move("right"),
  Up: move("up"),
  Down: move("down"),
  Left: move("left"),
  Right: move("right"),
  Enter: { type: "enter" },
  Escape: { type: "back" },
  Backspace: { type: "back" },
  GoBack: { type: "back" },
  BrowserBack: { type: "back" },
  XF86Back: { type: "back" },
  MediaPlayPause: media("playpause"),
  MediaPlay: media("play"),
  MediaPause: media("pause"),
  MediaStop: media("stop"),
  MediaFastForward: media("forward"),
  MediaRewind: media("rewind"),
  MediaTrackNext: media("next"),
  MediaTrackPrevious: media("previous"),
};

/** keyCodes shared by Tizen, webOS and desktop Chromium. */
const COMMON_CODES: Record<number, KeyAction> = {
  37: move("left"),
  38: move("up"),
  39: move("right"),
  40: move("down"),
  13: { type: "enter" },
  27: { type: "back" },
  8: { type: "back" },
  166: { type: "back" }, // BrowserBack
  179: media("playpause"),
  178: media("stop"),
  176: media("next"),
  177: media("previous"),
  415: media("play"),
  19: media("pause"),
  413: media("stop"),
  417: media("forward"),
  412: media("rewind"),
};

export const PLATFORM_CODES: Record<PlatformId, Record<number, KeyAction>> = {
  tizen: {
    10009: { type: "back" }, // Return
    10182: { type: "exit" }, // Exit
    10252: media("playpause"),
    10232: media("previous"),
    10233: media("next"),
  },
  webos: {
    461: { type: "back" },
  },
  browser: {},
};

/** Tizen keys that must be registered to be delivered to the app (names per tizen.tvinputdevice). */
export const TIZEN_REGISTERED_KEYS = [
  "MediaPlayPause",
  "MediaPlay",
  "MediaPause",
  "MediaStop",
  "MediaFastForward",
  "MediaRewind",
  "MediaTrackPrevious",
  "MediaTrackNext",
  "0",
  "1",
  "2",
  "3",
  "4",
  "5",
  "6",
  "7",
  "8",
  "9",
  "Exit",
] as const;

function digitFrom(ev: KeyLike): string | null {
  if (ev.key !== undefined && ev.key.length === 1 && ev.key >= "0" && ev.key <= "9") return ev.key;
  const c = ev.keyCode;
  if (c === undefined) return null;
  if (c >= 48 && c <= 57) return String(c - 48);
  if (c >= 96 && c <= 105) return String(c - 96);
  return null;
}

/**
 * Maps a keydown event to a logical action. Platform-specific codes win over the shared table
 * (e.g. Tizen delivers Back as 10009 with an empty or "XF86Back" key).
 * `extra` holds codes discovered at runtime (tizen.tvinputdevice.getKey(name).code).
 */
export function mapKey(ev: KeyLike, platform: PlatformId, extra?: Record<number, KeyAction>): KeyAction | null {
  const code = ev.keyCode;
  if (code !== undefined) {
    const specific = (extra && extra[code]) || PLATFORM_CODES[platform][code];
    if (specific) return specific;
  }
  const digit = digitFrom(ev);
  if (digit !== null) return { type: "digit", digit };
  if (ev.key !== undefined && ev.key !== "Unidentified" && BY_KEY[ev.key]) return BY_KEY[ev.key] ?? null;
  if (code !== undefined && COMMON_CODES[code]) return COMMON_CODES[code] ?? null;
  return null;
}

/** Converts tvinputdevice key names to actions (used to build `extra` from getKey(name).code). */
export function actionForTizenKeyName(name: string): KeyAction | null {
  if (name.length === 1 && name >= "0" && name <= "9") return { type: "digit", digit: name };
  if (name === "Exit") return { type: "exit" };
  const a = BY_KEY[name];
  return a ?? null;
}
