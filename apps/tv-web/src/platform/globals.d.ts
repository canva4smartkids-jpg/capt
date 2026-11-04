/**
 * Minimal typings for the TV runtime globals we feature-detect. Only what is used is declared;
 * everything is optional because the same bundle runs on Tizen, webOS and desktop browsers.
 */

declare const __APP_VERSION__: string;

interface TizenSystemInfoBuild {
  model?: string;
  manufacturer?: string;
  buildVersion?: string;
}
interface TizenSystemInfoDisplay {
  resolutionWidth?: number;
  resolutionHeight?: number;
}
interface TizenSystemInfo {
  getPropertyValue(property: "BUILD" | "DISPLAY", onSuccess: (v: unknown) => void, onError?: (e: unknown) => void): void;
  getCapability?(key: string): unknown;
}
interface TizenInputDeviceKey {
  name: string;
  code: number;
}
interface TizenTvInputDevice {
  registerKey(name: string): void;
  registerKeyBatch?(names: string[], onSuccess?: () => void, onError?: (e: unknown) => void): void;
  getKey?(name: string): TizenInputDeviceKey | null;
  getSupportedKeys?(): TizenInputDeviceKey[];
}
interface TizenApplicationApi {
  getCurrentApplication(): { exit(): void; hide?(): void };
}
interface TizenGlobal {
  systeminfo?: TizenSystemInfo;
  tvinputdevice?: TizenTvInputDevice;
  application?: TizenApplicationApi;
}
interface SamsungWebApis {
  productinfo?: {
    getRealModel?(): string;
    getModel?(): string;
    getModelCode?(): string;
    getFirmware?(): string;
    getVersion?(): string;
  };
}

interface WebOSDeviceInfo {
  modelName?: string;
  modelNameAscii?: string;
  version?: string;
  versionMajor?: number;
  versionMinor?: number;
  sdkVersion?: string;
  screenWidth?: number;
  screenHeight?: number;
  uhd?: boolean;
}
interface WebOSServiceRequestParams {
  method?: string;
  parameters?: Record<string, unknown>;
  onSuccess?: (res: Record<string, unknown>) => void;
  onFailure?: (res: Record<string, unknown>) => void;
}
interface WebOSGlobal {
  deviceInfo?(cb: (info: WebOSDeviceInfo) => void): void;
  platformBack?(): void;
  service?: { request(uri: string, params: WebOSServiceRequestParams): unknown };
}
interface PalmSystemGlobal {
  deviceInfo?: string;
  platformBack?(): void;
  identifier?: string;
}
declare class PalmServiceBridge {
  constructor();
  onservicecallback: ((msg: string) => void) | null;
  call(uri: string, params: string): void;
  cancel?(): void;
}

interface Window {
  tizen?: TizenGlobal;
  webapis?: SamsungWebApis;
  webOS?: WebOSGlobal;
  PalmSystem?: PalmSystemGlobal;
  PalmServiceBridge?: typeof PalmServiceBridge;
}
