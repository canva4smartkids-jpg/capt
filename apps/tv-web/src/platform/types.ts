import type { DeviceKind } from "@captureone/capture-contracts";
import type { KeyAction, PlatformId } from "../keys.js";
import type { TvDeviceInfo } from "../capabilities.js";

export interface PlatformAdapter {
  readonly id: PlatformId;
  readonly deviceKind: DeviceKind;
  /** e.g. "Samsung Tizen TV". */
  readonly label: string;
  /** Registers remote keys (Tizen) and returns extra keyCode→action mappings discovered at runtime. */
  init(): Record<number, KeyAction>;
  getDeviceInfo(): Promise<TvDeviceInfo>;
  /** Leaves the app (Back on the home screen). */
  exit(): void;
}

/** Resolves with `fallback` if `p` does not settle within `ms` (TV APIs sometimes never call back). */
export function withTimeout<T>(p: Promise<T>, ms: number, fallback: T): Promise<T> {
  return new Promise<T>((resolve) => {
    const t = setTimeout(() => resolve(fallback), ms);
    p.then(
      (v) => {
        clearTimeout(t);
        resolve(v);
      },
      () => {
        clearTimeout(t);
        resolve(fallback);
      },
    );
  });
}

export function screenResolution(): { width: number; height: number } | undefined {
  if (typeof screen === "undefined" || !screen.width) return undefined;
  const dpr = typeof devicePixelRatio === "number" && devicePixelRatio > 0 ? devicePixelRatio : 1;
  return { width: Math.round(screen.width * dpr), height: Math.round(screen.height * dpr) };
}
