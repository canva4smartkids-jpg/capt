/**
 * LG webOS TV adapter. The webOSTV.js library (window.webOS) is downloaded from LG's developer
 * site and is not bundled; everything is feature-detected:
 *  - webOS.deviceInfo() when webOSTV.js is present,
 *  - PalmSystem.deviceInfo (JSON string, built into the webOS web runtime),
 *  - luna://com.webos.service.tv.systemproperty/getSystemInfo via webOS.service.request, or via
 *    the runtime's PalmServiceBridge when webOSTV.js is absent,
 *  - Back on the root screen: webOS.platformBack() / PalmSystem.platformBack().
 */
import type { KeyAction } from "../keys.js";
import type { TvDeviceInfo } from "../capabilities.js";
import { screenResolution, withTimeout, type PlatformAdapter } from "./types.js";

const SYSTEM_PROPERTY = "luna://com.webos.service.tv.systemproperty";

function deviceInfoFromLibrary(): Promise<WebOSDeviceInfo | null> {
  const w = window.webOS;
  if (!w || typeof w.deviceInfo !== "function") return Promise.resolve(null);
  return withTimeout(
    new Promise<WebOSDeviceInfo | null>((resolve) => {
      try {
        w.deviceInfo!((info) => resolve(info));
      } catch {
        resolve(null);
      }
    }),
    3000,
    null,
  );
}

function palmDeviceInfo(): Record<string, unknown> | null {
  const raw = window.PalmSystem?.deviceInfo;
  if (typeof raw !== "string") return null;
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}

/** Calls a Luna service using webOS.service.request or PalmServiceBridge, whichever exists. */
export function lunaRequest(uri: string, method: string, parameters: Record<string, unknown>): Promise<Record<string, unknown> | null> {
  const w = window.webOS;
  if (w?.service?.request) {
    return withTimeout(
      new Promise((resolve) => {
        try {
          w.service!.request(uri, { method, parameters, onSuccess: (r) => resolve(r), onFailure: () => resolve(null) });
        } catch {
          resolve(null);
        }
      }),
      3000,
      null,
    );
  }
  const Bridge = window.PalmServiceBridge;
  if (typeof Bridge === "function") {
    return withTimeout(
      new Promise((resolve) => {
        try {
          const b = new Bridge();
          b.onservicecallback = (msg: string) => {
            try {
              resolve(JSON.parse(msg) as Record<string, unknown>);
            } catch {
              resolve(null);
            }
          };
          b.call(`${uri}/${method}`, JSON.stringify(parameters));
        } catch {
          resolve(null);
        }
      }),
      3000,
      null,
    );
  }
  return Promise.resolve(null);
}

const str = (v: unknown): string | undefined => (typeof v === "string" && v.length > 0 ? v : undefined);

export function createWebOSAdapter(): PlatformAdapter {
  return {
    id: "webos",
    deviceKind: "lg-webos-tv",
    label: "LG webOS TV",
    init() {
      // webOS delivers arrows, OK, Back (461) and media keys without registration.
      return {} as Record<number, KeyAction>;
    },
    async getDeviceInfo(): Promise<TvDeviceInfo> {
      const sources: string[] = [];
      const lib = await deviceInfoFromLibrary();
      if (lib) sources.push("webOS.deviceInfo");
      const palm = palmDeviceInfo();
      if (palm) sources.push("PalmSystem.deviceInfo");
      const luna = await lunaRequest(SYSTEM_PROPERTY, "getSystemInfo", { keys: ["modelName", "firmwareVersion", "UHD", "sdkVersion"] });
      const lunaOk = luna && luna["returnValue"] !== false ? luna : null;
      if (lunaOk) sources.push("luna systemproperty");
      const ua = /webOS\.TV-(\d{4})/i.exec(navigator.userAgent);
      const info: TvDeviceInfo = {
        manufacturer: "LG Electronics",
        osName: "webOS TV",
        osVersion:
          str(lib?.version) ||
          str(lunaOk?.["sdkVersion"]) ||
          str(palm?.["platformVersion"]) ||
          (ua && ua[1] ? `webOS TV ${ua[1]}` : "") ||
          "unknown",
        sources,
      };
      const model = str(lunaOk?.["modelName"]) || str(lib?.modelName) || str(palm?.["modelName"]);
      if (model) info.model = model;
      const fw = str(lunaOk?.["firmwareVersion"]) || str(palm?.["firmwareVersion"]);
      if (fw) info.firmware = fw;
      if (lib?.screenWidth && lib.screenHeight) info.resolution = { width: lib.screenWidth, height: lib.screenHeight };
      else {
        const r = screenResolution();
        if (r) info.resolution = r;
      }
      return info;
    },
    exit() {
      try {
        if (window.webOS?.platformBack) window.webOS.platformBack();
        else if (window.PalmSystem?.platformBack) window.PalmSystem.platformBack();
        else window.close();
      } catch {
        /* ignore */
      }
    },
  };
}
