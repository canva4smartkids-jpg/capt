import { describe, expect, it } from "vitest";
import { KeypadModel, parseHubAddress } from "../src/keypad.js";
import { normalizeHubUrl } from "../src/hub-client.js";

describe("KeypadModel", () => {
  it("builds an IPv4 address with port", () => {
    const k = new KeypadModel("address");
    for (const c of "192.168.1.20:47800") k.press(c as never);
    expect(k.value).toBe("192.168.1.20:47800");
    expect(k.isComplete()).toBe(true);
  });

  it("rejects separators in invalid places", () => {
    const k = new KeypadModel("address");
    k.press(".");
    k.press(":");
    expect(k.value).toBe("");
    for (const c of "10..0") k.press(c as never);
    expect(k.value).toBe("10.0");
    k.press(":"); // only after four octets
    expect(k.value).toBe("10.0");
    for (const c of ".0.1:80") k.press(c as never);
    expect(k.value).toBe("10.0.0.1:80");
    k.press(".");
    k.press(":");
    expect(k.value).toBe("10.0.0.1:80");
  });

  it("deletes, clears and limits length", () => {
    const code = new KeypadModel("code");
    for (const c of "12345678") code.press(c as never);
    expect(code.value).toBe("123456");
    expect(code.isComplete()).toBe(true);
    code.press(".");
    expect(code.value).toBe("123456");
    code.press("del");
    expect(code.value).toBe("12345");
    expect(code.isComplete()).toBe(false);
    code.press("clear");
    expect(code.value).toBe("");
  });

  it("restores an initial value", () => {
    expect(new KeypadModel("address", "10.0.0.2").value).toBe("10.0.0.2");
  });
});

describe("parseHubAddress / normalizeHubUrl", () => {
  it("parses addresses with the default port", () => {
    expect(parseHubAddress("192.168.0.5")).toEqual({ host: "192.168.0.5", port: 47800 });
    expect(parseHubAddress("192.168.0.5:8080")).toEqual({ host: "192.168.0.5", port: 8080 });
    expect(parseHubAddress("010.000.000.001")).toEqual({ host: "10.0.0.1", port: 47800 });
  });
  it("rejects invalid addresses", () => {
    for (const bad of ["", "1.2.3", "256.1.1.1", "1.2.3.4:0", "1.2.3.4:70000", "a.b.c.d", "1.2.3.4:"]) expect(parseHubAddress(bad)).toBeNull();
  });
  it("normalizes to an http base URL", () => {
    expect(normalizeHubUrl("192.168.1.2")).toBe("http://192.168.1.2:47800");
    expect(normalizeHubUrl("http://192.168.1.2:9000/")).toBe("http://192.168.1.2:9000");
    expect(normalizeHubUrl("nonsense")).toBeNull();
  });
});
