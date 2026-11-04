import { describe, expect, it } from "vitest";
import { actionForTizenKeyName, mapKey, TIZEN_REGISTERED_KEYS } from "../src/keys.js";

describe("mapKey", () => {
  it("maps arrows and OK on every platform (keyCode only, as old TV browsers send)", () => {
    for (const p of ["tizen", "webos", "browser"] as const) {
      expect(mapKey({ keyCode: 37 }, p)).toEqual({ type: "move", dir: "left" });
      expect(mapKey({ keyCode: 38 }, p)).toEqual({ type: "move", dir: "up" });
      expect(mapKey({ keyCode: 39 }, p)).toEqual({ type: "move", dir: "right" });
      expect(mapKey({ keyCode: 40 }, p)).toEqual({ type: "move", dir: "down" });
      expect(mapKey({ keyCode: 13 }, p)).toEqual({ type: "enter" });
      expect(mapKey({ key: "ArrowUp", keyCode: 38 }, p)).toEqual({ type: "move", dir: "up" });
    }
  });

  it("maps Back per platform", () => {
    expect(mapKey({ keyCode: 10009, key: "XF86Back" }, "tizen")).toEqual({ type: "back" });
    expect(mapKey({ keyCode: 10009, key: "Unidentified" }, "tizen")).toEqual({ type: "back" });
    expect(mapKey({ keyCode: 461 }, "webos")).toEqual({ type: "back" });
    expect(mapKey({ key: "GoBack", keyCode: 461 }, "webos")).toEqual({ type: "back" });
    expect(mapKey({ key: "Escape", keyCode: 27 }, "browser")).toEqual({ type: "back" });
    expect(mapKey({ key: "Backspace", keyCode: 8 }, "browser")).toEqual({ type: "back" });
    // Codes of the other platform are not misinterpreted.
    expect(mapKey({ keyCode: 461 }, "tizen")).toBeNull();
    expect(mapKey({ keyCode: 10009 }, "webos")).toBeNull();
  });

  it("maps Tizen Exit and media keys", () => {
    expect(mapKey({ keyCode: 10182 }, "tizen")).toEqual({ type: "exit" });
    expect(mapKey({ keyCode: 10252 }, "tizen")).toEqual({ type: "media", command: "playpause" });
    expect(mapKey({ keyCode: 10232 }, "tizen")).toEqual({ type: "media", command: "previous" });
    expect(mapKey({ keyCode: 10233 }, "tizen")).toEqual({ type: "media", command: "next" });
  });

  it("maps shared media keyCodes (webOS / Tizen / desktop)", () => {
    for (const p of ["tizen", "webos"] as const) {
      expect(mapKey({ keyCode: 415 }, p)).toEqual({ type: "media", command: "play" });
      expect(mapKey({ keyCode: 19 }, p)).toEqual({ type: "media", command: "pause" });
      expect(mapKey({ keyCode: 413 }, p)).toEqual({ type: "media", command: "stop" });
      expect(mapKey({ keyCode: 417 }, p)).toEqual({ type: "media", command: "forward" });
      expect(mapKey({ keyCode: 412 }, p)).toEqual({ type: "media", command: "rewind" });
    }
    expect(mapKey({ key: "MediaPlayPause", keyCode: 179 }, "browser")).toEqual({ type: "media", command: "playpause" });
  });

  it("maps remote number keys and numpad digits", () => {
    expect(mapKey({ keyCode: 48 }, "webos")).toEqual({ type: "digit", digit: "0" });
    expect(mapKey({ keyCode: 57 }, "tizen")).toEqual({ type: "digit", digit: "9" });
    expect(mapKey({ keyCode: 101 }, "browser")).toEqual({ type: "digit", digit: "5" });
    expect(mapKey({ key: "7" }, "browser")).toEqual({ type: "digit", digit: "7" });
  });

  it("prefers runtime codes from tizen.tvinputdevice.getKey()", () => {
    expect(mapKey({ keyCode: 12345 }, "tizen", { 12345: { type: "media", command: "stop" } })).toEqual({ type: "media", command: "stop" });
  });

  it("ignores unrelated keys", () => {
    expect(mapKey({ key: "a", keyCode: 65 }, "browser")).toBeNull();
    expect(mapKey({}, "browser")).toBeNull();
  });

  it("knows every registered Tizen key name", () => {
    for (const n of TIZEN_REGISTERED_KEYS) expect(actionForTizenKeyName(n)).not.toBeNull();
    expect(actionForTizenKeyName("ColorF0Red")).toBeNull();
  });
});
