import type { PlatformId } from "../keys.js";
import { createBrowserAdapter } from "./browser.js";
import { createTizenAdapter } from "./tizen.js";
import type { PlatformAdapter } from "./types.js";
import { createWebOSAdapter } from "./webos.js";

export interface DetectEnv {
  hasTizen: boolean;
  hasWebOS: boolean;
  userAgent: string;
  /** `?platform=` override (development/testing). */
  override: string | null;
}

export function detectPlatform(env: DetectEnv): PlatformId {
  if (env.override === "tizen" || env.override === "webos" || env.override === "browser") return env.override;
  if (env.hasTizen || /Tizen/i.test(env.userAgent)) return "tizen";
  if (env.hasWebOS || /Web0S|webOS/i.test(env.userAgent)) return "webos";
  return "browser";
}

export function currentEnv(): DetectEnv {
  let override: string | null = null;
  try {
    const m = /[?&]platform=([a-z]+)/.exec(location.search);
    override = m ? (m[1] ?? null) : null;
  } catch {
    override = null;
  }
  return {
    hasTizen: !!(window.tizen && window.tizen.tvinputdevice),
    hasWebOS: !!(window.PalmSystem || window.webOS || window.PalmServiceBridge),
    userAgent: navigator.userAgent,
    override,
  };
}

export function createAdapter(id: PlatformId): PlatformAdapter {
  return id === "tizen" ? createTizenAdapter() : id === "webos" ? createWebOSAdapter() : createBrowserAdapter();
}
