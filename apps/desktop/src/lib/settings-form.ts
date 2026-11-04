/**
 * Settings form helpers: validation (contract schema + UX rules such as unique,
 * safe global shortcuts) and the file-name template preview.
 */
import { AppSettings, expandFileName, type ShortcutBindings } from "@captureone/settings";
import { isSafeGlobalAccelerator, parseAccelerator, formatAccelerator } from "@captureone/design-system";

export type SettingsErrors = Partial<Record<string, string>>;

const SHORTCUT_KEYS: (keyof ShortcutBindings)[] = ["startStop", "pauseResume", "screenshot", "regionScreenshot"];

export function validateSettings(draft: AppSettings): SettingsErrors {
  const errors: SettingsErrors = {};
  const parsed = AppSettings.safeParse(draft);
  if (!parsed.success) {
    for (const issue of parsed.error.issues) {
      const key = issue.path.join(".");
      errors[key] ??= issue.message;
    }
  }
  if (!draft.outputDirectory.trim()) errors.outputDirectory = "Choose an output folder.";
  if (!draft.fileNameTemplate.trim()) errors.fileNameTemplate = "The file name template cannot be empty.";
  else if (/[\\/]/.test(draft.fileNameTemplate)) errors.fileNameTemplate = "The template cannot contain folder separators.";
  if (!Number.isInteger(draft.jpegQuality) || draft.jpegQuality < 1 || draft.jpegQuality > 100) errors.jpegQuality = "JPEG quality must be between 1 and 100.";
  if (draft.maxDurationSec !== undefined && (!Number.isInteger(draft.maxDurationSec) || draft.maxDurationSec <= 0)) errors.maxDurationSec = "Enter a positive number of minutes, or leave empty for no limit.";
  if (draft.video.quality === "custom") {
    const b = draft.video.bitrateKbps;
    if (b === undefined || b < 250 || b > 200_000) errors["video.bitrateKbps"] = "Custom bitrate must be between 250 and 200,000 kbps.";
  }
  const seen = new Map<string, keyof ShortcutBindings>();
  for (const k of SHORTCUT_KEYS) {
    const accel = draft.shortcuts[k];
    if (!accel) continue; // empty = disabled
    const p = parseAccelerator(accel);
    if (!p) {
      errors[`shortcuts.${k}`] = "Not a valid shortcut.";
      continue;
    }
    if (!isSafeGlobalAccelerator(accel)) {
      errors[`shortcuts.${k}`] = "Global shortcuts need Ctrl/Cmd or Alt (or a function key).";
      continue;
    }
    const canon = formatAccelerator(p);
    const other = seen.get(canon);
    if (other) errors[`shortcuts.${k}`] = `Already used by "${SHORTCUT_NAMES[other]}".`;
    else seen.set(canon, k);
  }
  return errors;
}

export const SHORTCUT_NAMES: Record<keyof ShortcutBindings, string> = {
  startStop: "Start / stop recording",
  pauseResume: "Pause / resume",
  screenshot: "Screenshot",
  regionScreenshot: "Region screenshot",
};

export function hasErrors(e: SettingsErrors): boolean {
  return Object.values(e).some(Boolean);
}

/** Preview of the next file name produced by a template. */
export function fileNamePreview(template: string, ext: "mp4" | "png" | "jpg" = "mp4", date = new Date(), source = "Display 1"): string {
  return `${expandFileName(template || "CaptureOne", { date, source, n: 1 })}.${ext}`;
}

export const TEMPLATE_TOKENS = [
  { token: "{date}", description: "Date (YYYY-MM-DD)" },
  { token: "{time}", description: "Time (HH-MM-SS)" },
  { token: "{source}", description: "Display or window name" },
  { token: "{n}", description: "Counter" },
] as const;

/** Shallow structural equality via JSON (settings are plain JSON data). */
export function settingsEqual(a: AppSettings, b: AppSettings): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
