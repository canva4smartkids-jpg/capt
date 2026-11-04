import { describe, expect, it } from "vitest";
import { hrefFor, parseHash } from "../src/lib/router";

describe("hash router", () => {
  it("maps the window routes from the contract", () => {
    expect(parseHash("#/").route).toBe("record");
    expect(parseHash("").route).toBe("record");
    expect(parseHash("#/library").route).toBe("library");
    expect(parseHash("#/indicator").route).toBe("indicator");
    const r = parseHash("#/region?displayId=x%3A1&w=1920&h=1080");
    expect(r.route).toBe("region");
    expect(r.params.get("displayId")).toBe("x:1");
    expect(r.params.get("w")).toBe("1920");
    expect(parseHash("#/nope").route).toBe("record");
    expect(hrefFor("record")).toBe("#/");
    expect(hrefFor("devices")).toBe("#/devices");
  });
});
