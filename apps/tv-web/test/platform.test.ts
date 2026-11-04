import { afterEach, describe, expect, it, vi } from "vitest";
import { detectPlatform } from "../src/platform/detect.js";
import { createTizenAdapter } from "../src/platform/tizen.js";
import { createWebOSAdapter } from "../src/platform/webos.js";
import { buttonStates } from "../src/screens/remote.js";

const w = window as unknown as Record<string, unknown>;

afterEach(() => {
  for (const k of ["tizen", "webapis", "webOS", "PalmSystem", "PalmServiceBridge"]) delete w[k];
});

describe("detectPlatform", () => {
  it("detects by runtime globals, user agent and override", () => {
    expect(detectPlatform({ hasTizen: true, hasWebOS: false, userAgent: "", override: null })).toBe("tizen");
    expect(detectPlatform({ hasTizen: false, hasWebOS: false, userAgent: "Mozilla/5.0 (SMART-TV; LINUX; Tizen 6.0)", override: null })).toBe("tizen");
    expect(detectPlatform({ hasTizen: false, hasWebOS: false, userAgent: "Mozilla/5.0 (Web0S; Linux/SmartTV) Chrome/79", override: null })).toBe("webos");
    expect(detectPlatform({ hasTizen: false, hasWebOS: true, userAgent: "", override: null })).toBe("webos");
    expect(detectPlatform({ hasTizen: false, hasWebOS: false, userAgent: "Chrome/120", override: null })).toBe("browser");
    expect(detectPlatform({ hasTizen: true, hasWebOS: false, userAgent: "", override: "webos" })).toBe("webos");
  });
});

describe("Tizen adapter", () => {
  it("registers media/number keys and reads systeminfo + productinfo", async () => {
    const registered: string[] = [];
    w.tizen = {
      tvinputdevice: {
        registerKey: (n: string) => registered.push(n),
        getSupportedKeys: () => [{ name: "MediaPlayPause", code: 10252 }, { name: "MediaStop", code: 413 }, { name: "5", code: 53 }],
        getKey: (n: string) => ({ name: n, code: n === "MediaPlayPause" ? 10252 : n === "MediaStop" ? 413 : 53 }),
      },
      systeminfo: {
        getPropertyValue: (p: string, ok: (v: unknown) => void) =>
          ok(p === "BUILD" ? { model: "QE55Q80TATXXU", manufacturer: "Samsung Electronics", buildVersion: "T-KSU2EAKUC-1302.5" } : { resolutionWidth: 3840, resolutionHeight: 2160 }),
        getCapability: () => "6.0",
      },
      application: { getCurrentApplication: () => ({ exit: vi.fn() }) },
    };
    w.webapis = { productinfo: { getRealModel: () => "QE55Q80T", getFirmware: () => "T-KSU2EAKUC-1302.5" } };
    const a = createTizenAdapter();
    const extra = a.init();
    expect(registered.sort()).toEqual(["5", "MediaPlayPause", "MediaStop"]);
    expect(extra[10252]).toEqual({ type: "media", command: "playpause" });
    const info = await a.getDeviceInfo();
    expect(info).toMatchObject({ manufacturer: "Samsung Electronics", model: "QE55Q80T", osName: "Tizen", osVersion: "6.0", firmware: "T-KSU2EAKUC-1302.5", resolution: { width: 3840, height: 2160 } });
    expect(info.sources).toEqual(["tizen.systeminfo", "webapis.productinfo"]);
  });

  it("degrades without webapis and with failing systeminfo", async () => {
    w.tizen = { systeminfo: { getPropertyValue: (_p: string, _ok: unknown, err: (e: unknown) => void) => err(new Error("denied")) } };
    const info = await createTizenAdapter().getDeviceInfo();
    expect(info.manufacturer).toBe("Samsung");
    expect(info.model).toBeUndefined();
    expect(info.sources).toEqual([]);
  });
});

describe("webOS adapter", () => {
  it("uses PalmSystem.deviceInfo and PalmServiceBridge when webOSTV.js is absent", async () => {
    w.PalmSystem = { deviceInfo: JSON.stringify({ modelName: "OLED55C1PUB", platformVersion: "6.3.0" }), platformBack: vi.fn() };
    const calls: { uri: string; params: string }[] = [];
    class Bridge {
      onservicecallback: ((m: string) => void) | null = null;
      call(uri: string, params: string) {
        calls.push({ uri, params });
        setTimeout(() => this.onservicecallback?.(JSON.stringify({ returnValue: true, modelName: "OLED55C1PUB", firmwareVersion: "03.20.50", sdkVersion: "6.3.0" })), 0);
      }
    }
    w.PalmServiceBridge = Bridge;
    const a = createWebOSAdapter();
    const info = await a.getDeviceInfo();
    expect(calls[0]!.uri).toBe("luna://com.webos.service.tv.systemproperty/getSystemInfo");
    expect(JSON.parse(calls[0]!.params)).toEqual({ keys: ["modelName", "firmwareVersion", "UHD", "sdkVersion"] });
    expect(info).toMatchObject({ manufacturer: "LG Electronics", model: "OLED55C1PUB", firmware: "03.20.50", osName: "webOS TV", osVersion: "6.3.0" });
    expect(info.sources).toEqual(["PalmSystem.deviceInfo", "luna systemproperty"]);
    a.exit();
    expect((w.PalmSystem as { platformBack: ReturnType<typeof vi.fn> }).platformBack).toHaveBeenCalled();
  });

  it("prefers webOS.deviceInfo / webOS.service.request from webOSTV.js", async () => {
    const back = vi.fn();
    w.webOS = {
      deviceInfo: (cb: (i: object) => void) => cb({ modelName: "50UP7500", version: "6.2.0", screenWidth: 1920, screenHeight: 1080 }),
      service: { request: (_u: string, p: { onSuccess: (r: unknown) => void }) => p.onSuccess({ returnValue: true, firmwareVersion: "04.40.10" }) },
      platformBack: back,
    };
    const a = createWebOSAdapter();
    const info = await a.getDeviceInfo();
    expect(info).toMatchObject({ model: "50UP7500", osVersion: "6.2.0", firmware: "04.40.10", resolution: { width: 1920, height: 1080 } });
    a.exit();
    expect(back).toHaveBeenCalled();
  });

  it("works with nothing available", async () => {
    const info = await createWebOSAdapter().getDeviceInfo();
    expect(info.osName).toBe("webOS TV");
    expect(info.sources).toEqual([]);
  });
});

describe("remote button states", () => {
  const st = (state: string) => ({ sessionId: "s", state, elapsedMs: 0, bytesWritten: 0, framesEncoded: 0, framesDropped: 0, warnings: [] }) as never;
  it("follows the recording state machine", () => {
    expect(buttonStates(null)).toEqual({ start: true, pause: false, resume: false, stop: false, screenshot: true });
    expect(buttonStates(st("recording"))).toMatchObject({ start: false, pause: true, resume: false, stop: true });
    expect(buttonStates(st("paused"))).toMatchObject({ start: false, pause: false, resume: true, stop: true });
    expect(buttonStates(st("finalizing"))).toMatchObject({ start: false, pause: false, resume: false, stop: false });
    expect(buttonStates(st("completed"))).toMatchObject({ start: true });
  });
});
