/**
 * Helpers for Tauri/Electron-style accelerators ("CommandOrControl+Shift+R").
 * Shared by the shortcut editor (capture from keyboard) and the KeyboardShortcut chip.
 */
export type KeyPlatform = "mac" | "other";

export function detectPlatform(): KeyPlatform {
  if (typeof navigator === "undefined") return "other";
  const p = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform ?? navigator.platform ?? "";
  return /mac/i.test(p) ? "mac" : "other";
}

const MODIFIERS = ["CommandOrControl", "CmdOrCtrl", "Command", "Cmd", "Control", "Ctrl", "Super", "Meta", "Alt", "Option", "AltGr", "Shift"] as const;
const MOD_ORDER = ["CommandOrControl", "Control", "Alt", "Shift", "Super"] as const;
type CanonicalMod = (typeof MOD_ORDER)[number];

function canonicalModifier(token: string): CanonicalMod | null {
  switch (token.toLowerCase()) {
    case "commandorcontrol":
    case "cmdorctrl":
      return "CommandOrControl";
    case "command":
    case "cmd":
    case "super":
    case "meta":
      return "Super";
    case "control":
    case "ctrl":
      return "Control";
    case "alt":
    case "option":
    case "altgr":
      return "Alt";
    case "shift":
      return "Shift";
    default:
      return null;
  }
}

const NAMED_KEYS: Record<string, string> = {
  space: "Space",
  enter: "Enter",
  return: "Enter",
  tab: "Tab",
  backspace: "Backspace",
  delete: "Delete",
  insert: "Insert",
  home: "Home",
  end: "End",
  pageup: "PageUp",
  pagedown: "PageDown",
  up: "Up",
  down: "Down",
  left: "Left",
  right: "Right",
  arrowup: "Up",
  arrowdown: "Down",
  arrowleft: "Left",
  arrowright: "Right",
  escape: "Escape",
  esc: "Escape",
  printscreen: "PrintScreen",
};

function canonicalKey(token: string): string | null {
  if (/^[a-z0-9]$/i.test(token)) return token.toUpperCase();
  if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(token)) return token.toUpperCase();
  const named = NAMED_KEYS[token.toLowerCase()];
  if (named) return named;
  if (/^(plus|minus|comma|period|slash|backslash|semicolon|quote|backquote|bracketleft|bracketright|equal)$/i.test(token)) {
    return token[0]!.toUpperCase() + token.slice(1).toLowerCase();
  }
  return null;
}

export interface ParsedAccelerator {
  modifiers: CanonicalMod[];
  key: string;
}

/** Parses and canonicalises an accelerator; returns null when invalid (no key, unknown token, duplicate). */
export function parseAccelerator(accel: string): ParsedAccelerator | null {
  const tokens = accel.split("+").map((t) => t.trim()).filter(Boolean);
  if (tokens.length === 0) return null;
  const mods = new Set<CanonicalMod>();
  let key: string | null = null;
  for (const t of tokens) {
    const m = canonicalModifier(t);
    if (m) {
      if (mods.has(m)) return null;
      mods.add(m);
      continue;
    }
    if (key !== null) return null;
    key = canonicalKey(t);
    if (key === null) return null;
  }
  if (key === null) return null;
  return { modifiers: MOD_ORDER.filter((m) => mods.has(m)), key };
}

export function formatAccelerator(p: ParsedAccelerator): string {
  return [...p.modifiers, p.key].join("+");
}

/** Global shortcuts need at least one modifier (except function keys / PrintScreen) to avoid stealing typing. */
export function isSafeGlobalAccelerator(accel: string): boolean {
  const p = parseAccelerator(accel);
  if (!p) return false;
  if (p.modifiers.length > 0) return p.modifiers.some((m) => m !== "Shift") || /^F\d+$/.test(p.key);
  return /^F\d+$/.test(p.key) || p.key === "PrintScreen";
}

export interface KeyEventLike {
  key: string;
  code?: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/**
 * Converts a keydown into an accelerator. Ctrl (or Cmd on macOS) maps to
 * CommandOrControl so bindings are portable. Returns null for modifier-only presses.
 */
export function acceleratorFromKeyEvent(e: KeyEventLike, platform: KeyPlatform = detectPlatform()): string | null {
  if (["Control", "Shift", "Alt", "Meta", "OS", "AltGraph"].includes(e.key)) return null;
  let keyToken: string | null = null;
  if (e.code && /^Key[A-Z]$/.test(e.code)) keyToken = e.code.slice(3);
  else if (e.code && /^Digit[0-9]$/.test(e.code)) keyToken = e.code.slice(5);
  else if (e.key === " ") keyToken = "Space";
  else keyToken = canonicalKey(e.key) ?? (e.code ? canonicalKey(e.code) : null);
  if (!keyToken) return null;
  const mods: CanonicalMod[] = [];
  const primary = platform === "mac" ? e.metaKey : e.ctrlKey;
  const secondaryCtrl = platform === "mac" ? e.ctrlKey : false;
  const superKey = platform === "mac" ? false : e.metaKey;
  if (primary) mods.push("CommandOrControl");
  if (secondaryCtrl) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (superKey) mods.push("Super");
  return formatAccelerator({ modifiers: MOD_ORDER.filter((m) => mods.includes(m)), key: keyToken });
}

const MAC_SYMBOLS: Record<CanonicalMod, string> = { CommandOrControl: "⌘", Control: "⌃", Alt: "⌥", Shift: "⇧", Super: "⌘" };
const OTHER_SYMBOLS: Record<CanonicalMod, string> = { CommandOrControl: "Ctrl", Control: "Ctrl", Alt: "Alt", Shift: "Shift", Super: "Super" };
const NAMES: Record<KeyPlatform, Record<CanonicalMod, string>> = {
  mac: { CommandOrControl: "Command", Control: "Control", Alt: "Option", Shift: "Shift", Super: "Command" },
  other: { CommandOrControl: "Control", Control: "Control", Alt: "Alt", Shift: "Shift", Super: "Super" },
};
const KEY_SYMBOLS: Record<string, string> = { Up: "↑", Down: "↓", Left: "←", Right: "→", Enter: "↵", Space: "Space", Escape: "Esc" };

/** Display parts for chips: symbol (visual) and name (screen readers). */
export function acceleratorParts(accel: string, platform: KeyPlatform = detectPlatform()): { symbol: string; name: string }[] {
  const p = parseAccelerator(accel);
  if (!p) return [];
  const symbols = platform === "mac" ? MAC_SYMBOLS : OTHER_SYMBOLS;
  return [
    ...p.modifiers.map((m) => ({ symbol: symbols[m], name: NAMES[platform][m] })),
    { symbol: KEY_SYMBOLS[p.key] ?? p.key, name: p.key },
  ];
}

export function acceleratorLabel(accel: string, platform: KeyPlatform = detectPlatform()): string {
  const parts = acceleratorParts(accel, platform);
  return parts.map((x) => x.symbol).join(platform === "mac" ? "" : "+");
}

/** True when a keyboard event matches an accelerator (for in-app shortcuts). */
export function matchesAccelerator(e: KeyEventLike, accel: string, platform: KeyPlatform = detectPlatform()): boolean {
  const p = parseAccelerator(accel);
  const fromEvent = acceleratorFromKeyEvent(e, platform);
  return !!p && fromEvent !== null && formatAccelerator(p) === fromEvent;
}

export { MODIFIERS };
