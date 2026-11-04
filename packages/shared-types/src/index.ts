/**
 * UI-level types shared by CaptureOne TypeScript clients that are not part of the
 * wire contract (packages/capture-contracts owns everything that crosses IPC/network).
 */

/** Actions a global shortcut or tray item can trigger (payload of `app://shortcut`). */
export const SHORTCUT_ACTIONS = ["start-stop", "pause-resume", "screenshot", "region-screenshot"] as const;
export type ShortcutAction = (typeof SHORTCUT_ACTIONS)[number];

export interface ShortcutEventPayload {
  action: ShortcutAction;
}

export function isShortcutAction(value: unknown): value is ShortcutAction {
  return typeof value === "string" && (SHORTCUT_ACTIONS as readonly string[]).includes(value);
}

export function isShortcutEventPayload(value: unknown): value is ShortcutEventPayload {
  return typeof value === "object" && value !== null && isShortcutAction((value as { action?: unknown }).action);
}

/** Settings key in AppSettings.shortcuts for each action. */
export const SHORTCUT_SETTING_KEYS = {
  "start-stop": "startStop",
  "pause-resume": "pauseResume",
  screenshot: "screenshot",
  "region-screenshot": "regionScreenshot",
} as const satisfies Record<ShortcutAction, string>;

export const SHORTCUT_LABELS: Record<ShortcutAction, string> = {
  "start-stop": "Start / stop recording",
  "pause-resume": "Pause / resume recording",
  screenshot: "Screenshot",
  "region-screenshot": "Region screenshot",
};

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

export function resolveTheme(pref: ThemePreference, systemPrefersDark: boolean): ResolvedTheme {
  if (pref === "system") return systemPrefersDark ? "dark" : "light";
  return pref;
}

/** Discriminated state for anything loaded asynchronously. */
export type AsyncState<T, E = unknown> =
  | { status: "idle" }
  | { status: "loading"; previous?: T }
  | { status: "success"; data: T }
  | { status: "error"; error: E; previous?: T };

export function asyncData<T, E>(s: AsyncState<T, E>): T | undefined {
  switch (s.status) {
    case "success":
      return s.data;
    case "loading":
    case "error":
      return s.previous;
    default:
      return undefined;
  }
}

export type Result<T, E> = { ok: true; value: T } | { ok: false; error: E };
export const ok = <T>(value: T): Result<T, never> => ({ ok: true, value });
export const err = <E>(error: E): Result<never, E> => ({ ok: false, error });

export function assertNever(value: never, message = "Unexpected value"): never {
  throw new Error(`${message}: ${JSON.stringify(value)}`);
}

/** Toast severities used by the desktop UI. */
export type Tone = "neutral" | "info" | "success" | "warning" | "danger";
