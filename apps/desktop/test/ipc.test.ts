import { beforeEach, describe, expect, it, vi } from "vitest";
import { CaptureFailure, DESKTOP_EVENTS } from "@captureone/capture-contracts";
import { callsTo, emit, handlers, resetTauri } from "./tauri-mock";
import { capabilities, sources, video } from "./fixtures";

vi.mock("@tauri-apps/api/core", async () => (await import("./tauri-mock")).coreMock);
vi.mock("@tauri-apps/api/event", async () => (await import("./tauri-mock")).eventMock);

const { invoke, onCaptureEvent, onShortcut, asFailure, isUnsupported } = await import("../src/lib/ipc");

beforeEach(() => resetTauri());

describe("invoke", () => {
  it("passes typed args through and validates results with the contract schema", async () => {
    handlers.set("list_sources", () => sources);
    const list = await invoke("list_sources", { kinds: ["display"] });
    expect(list).toHaveLength(3);
    expect(callsTo("list_sources")[0]?.args).toEqual({ kinds: ["display"] });
  });

  it("sends an empty object for argument-less commands", async () => {
    handlers.set("get_capabilities", () => capabilities);
    const cap = await invoke("get_capabilities");
    expect(cap.platform).toBe("linux");
    expect(callsTo("get_capabilities")[0]?.args).toEqual({});
  });

  it("applies schema defaults at the boundary", async () => {
    const { audioTracks: _drop, ...noTracks } = video;
    handlers.set("library_list", () => [noTracks]);
    const [item] = await invoke("library_list");
    expect(item?.audioTracks).toEqual([]);
  });

  it("normalises CaptureError rejections (object and JSON string)", async () => {
    handlers.set("start_recording", () => {
      throw { code: "PERMISSION_DENIED", message: "screen recording denied", recoverable: false };
    });
    const err = await invoke("start_recording", { config: {} as never }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(CaptureFailure);
    expect((err as CaptureFailure).code).toBe("PERMISSION_DENIED");

    handlers.set("hub_status", () => {
      throw JSON.stringify({ code: "UNSUPPORTED", message: "Companion hub is not integrated in this build", recoverable: false });
    });
    const hub = await invoke("hub_status").catch((e: unknown) => e);
    expect(isUnsupported(hub)).toBe(true);
    expect(asFailure(hub).message).toContain("not integrated");
  });

  it("rejects malformed responses instead of passing them to the UI", async () => {
    handlers.set("stop_recording", () => ({ id: "x" }));
    const err = await invoke("stop_recording", { sessionId: "s" }).catch((e: unknown) => e);
    expect(asFailure(err).code).toBe("INTERNAL");
    expect(asFailure(err).message).toMatch(/Malformed response from 'stop_recording'/);
  });

  it("supports the additive renameFile extension on library_rename", async () => {
    handlers.set("library_rename", (a) => ({ ...video, title: String(a.title) }));
    const renamed = await invoke("library_rename", { id: "rec-1", title: "Demo", renameFile: true });
    expect(renamed.title).toBe("Demo");
    expect(callsTo("library_rename")[0]?.args).toEqual({ id: "rec-1", title: "Demo", renameFile: true });
  });

  it("validates thumbnails are data URLs and nullable region results", async () => {
    handlers.set("get_source_thumbnail", () => "data:image/png;base64,AAAA");
    await expect(invoke("get_source_thumbnail", { kind: "display", sourceId: "d", maxWidth: 10 })).resolves.toMatch(/^data:image/);
    handlers.set("select_region", () => null);
    await expect(invoke("select_region", { displayId: "d" })).resolves.toBeNull();
    handlers.set("select_region", () => ({ x: 1, y: 2, width: 0, height: 5 }));
    await expect(invoke("select_region", { displayId: "d" })).rejects.toBeInstanceOf(CaptureFailure);
  });
});

describe("events", () => {
  it("delivers valid capture events and drops malformed ones", async () => {
    const seen: string[] = [];
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const off = await onCaptureEvent((e) => seen.push(e.type));
    emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "countdown", sessionId: "s1", remainingSec: 3 });
    emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "countdown", sessionId: "s1", remainingSec: -1 });
    emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "bogus" });
    expect(seen).toEqual(["countdown"]);
    expect(warn).toHaveBeenCalledTimes(2);
    off();
    emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "countdown", sessionId: "s1", remainingSec: 2 });
    expect(seen).toEqual(["countdown"]);
    warn.mockRestore();
  });

  it("filters shortcut payloads", async () => {
    const actions: string[] = [];
    await onShortcut((p) => actions.push(p.action));
    emit(DESKTOP_EVENTS.SHORTCUT, { action: "pause-resume" });
    emit(DESKTOP_EVENTS.SHORTCUT, { action: "explode" });
    expect(actions).toEqual(["pause-resume"]);
  });
});
