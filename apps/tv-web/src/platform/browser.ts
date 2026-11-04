/** Development adapter for desktop browsers (and the headless smoke test). Not a TV. */
import type { KeyAction } from "../keys.js";
import type { TvDeviceInfo } from "../capabilities.js";
import { screenResolution, type PlatformAdapter } from "./types.js";

export function createBrowserAdapter(): PlatformAdapter {
  return {
    id: "browser",
    deviceKind: "other-tv",
    label: "Web browser (development)",
    init() {
      return {} as Record<number, KeyAction>;
    },
    async getDeviceInfo(): Promise<TvDeviceInfo> {
      const m = /(Chrome|Firefox|Safari)\/([\d.]+)/.exec(navigator.userAgent);
      const info: TvDeviceInfo = {
        manufacturer: "n/a",
        osName: m ? `${m[1]} browser` : "Browser",
        osVersion: m && m[2] ? m[2] : "unknown",
        model: "Development browser",
        sources: ["navigator.userAgent"],
      };
      const r = screenResolution();
      if (r) info.resolution = r;
      return info;
    },
    exit() {
      /* nothing to exit in a browser tab */
    },
  };
}
