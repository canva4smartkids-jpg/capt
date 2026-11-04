/**
 * Samsung Tizen TV adapter. Uses tizen.systeminfo (BUILD, DISPLAY), webapis.productinfo
 * (requires `$WEBAPIS/webapis/webapis.js`, injected by apps/samsung-tv at packaging time, and
 * the productinfo privilege) and tizen.tvinputdevice for media/number keys.
 */
import { actionForTizenKeyName, TIZEN_REGISTERED_KEYS, type KeyAction } from "../keys.js";
import type { TvDeviceInfo } from "../capabilities.js";
import { screenResolution, withTimeout, type PlatformAdapter } from "./types.js";

function sysProp<T>(name: "BUILD" | "DISPLAY"): Promise<T | null> {
  const si = window.tizen?.systeminfo;
  if (!si) return Promise.resolve(null);
  return withTimeout(
    new Promise<T | null>((resolve) => {
      try {
        si.getPropertyValue(name, (v) => resolve(v as T), () => resolve(null));
      } catch {
        resolve(null);
      }
    }),
    3000,
    null,
  );
}

function safe(f: (() => string) | undefined): string | undefined {
  try {
    const v = f ? f() : undefined;
    return typeof v === "string" && v.length > 0 ? v : undefined;
  } catch {
    return undefined;
  }
}

export function createTizenAdapter(): PlatformAdapter {
  return {
    id: "tizen",
    deviceKind: "samsung-tizen-tv",
    label: "Samsung Tizen TV",
    init() {
      const extra: Record<number, KeyAction> = {};
      const input = window.tizen?.tvinputdevice;
      if (!input) return extra;
      const names = TIZEN_REGISTERED_KEYS.slice() as string[];
      let supported: string[] | null = null;
      try {
        supported = input.getSupportedKeys ? input.getSupportedKeys().map((k) => k.name) : null;
      } catch {
        supported = null;
      }
      const wanted = supported ? names.filter((n) => supported!.indexOf(n) >= 0) : names;
      for (const n of wanted) {
        try {
          input.registerKey(n);
        } catch {
          /* key not available on this model */
        }
        try {
          const k = input.getKey ? input.getKey(n) : null;
          const action = actionForTizenKeyName(n);
          if (k && action) extra[k.code] = action;
        } catch {
          /* ignore */
        }
      }
      return extra;
    },
    async getDeviceInfo(): Promise<TvDeviceInfo> {
      const sources: string[] = [];
      const [build, display] = await Promise.all([sysProp<TizenSystemInfoBuild>("BUILD"), sysProp<TizenSystemInfoDisplay>("DISPLAY")]);
      if (build || display) sources.push("tizen.systeminfo");
      const pi = window.webapis?.productinfo;
      const realModel = safe(pi?.getRealModel?.bind(pi));
      const firmware = safe(pi?.getFirmware?.bind(pi));
      if (realModel || firmware) sources.push("webapis.productinfo");
      let platformVersion: string | undefined;
      try {
        const v = window.tizen?.systeminfo?.getCapability?.("http://tizen.org/feature/platform.version");
        platformVersion = typeof v === "string" ? v : undefined;
      } catch {
        platformVersion = undefined;
      }
      const ua = /Tizen\s*([\d.]+)/i.exec(navigator.userAgent);
      const info: TvDeviceInfo = {
        manufacturer: build?.manufacturer || "Samsung",
        osName: "Tizen",
        osVersion: platformVersion || (ua ? ua[1] ?? "" : "") || "unknown",
        sources,
      };
      const model = realModel || build?.model;
      if (model) info.model = model;
      const fw = firmware || build?.buildVersion;
      if (fw) info.firmware = fw;
      if (display?.resolutionWidth && display.resolutionHeight) {
        info.resolution = { width: display.resolutionWidth, height: display.resolutionHeight };
      } else {
        const r = screenResolution();
        if (r) info.resolution = r;
      }
      return info;
    },
    exit() {
      try {
        window.tizen?.application?.getCurrentApplication().exit();
      } catch {
        /* not running as an installed app */
      }
    },
  };
}
