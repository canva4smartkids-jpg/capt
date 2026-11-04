import { beforeEach, describe, expect, it, vi } from "vitest";
import { CaptureFailure } from "@captureone/capture-contracts";
import { callsTo, handlers, resetTauri } from "./tauri-mock";
import { shot, video } from "./fixtures";

vi.mock("@tauri-apps/api/core", async () => (await import("./tauri-mock")).coreMock);
vi.mock("@tauri-apps/api/event", async () => (await import("./tauri-mock")).eventMock);

const { deleteItem, exportFilters, exportItem, renameItem, revealItem, validateItem } = await import("../src/lib/library-actions");

beforeEach(() => resetTauri());

describe("library actions", () => {
  it("renames (title only by default) and replaces the item in place", async () => {
    handlers.set("library_rename", (a) => ({ ...video, title: String(a.title) }));
    const next = await renameItem([shot, video], "rec-1", "  Sprint demo  ");
    expect(next.map((m) => m.title)).toEqual(["Login bug", "Sprint demo"]);
    expect(callsTo("library_rename")[0]?.args).toEqual({ id: "rec-1", title: "Sprint demo" });
    await renameItem([video], "rec-1", "Demo", true);
    expect(callsTo("library_rename")[1]?.args).toEqual({ id: "rec-1", title: "Demo", renameFile: true });
  });

  it("refuses empty titles without calling the backend", async () => {
    await expect(renameItem([video], "rec-1", "   ")).rejects.toBeInstanceOf(CaptureFailure);
    expect(callsTo("library_rename")).toHaveLength(0);
  });

  it("deletes and removes from the list", async () => {
    handlers.set("library_delete", () => null);
    const next = await deleteItem([shot, video], "shot-1", true);
    expect(next.map((m) => m.id)).toEqual(["rec-1"]);
    expect(callsTo("library_delete")[0]?.args).toEqual({ id: "shot-1", deleteFile: true });
  });

  it("propagates backend failures (e.g. file in use)", async () => {
    handlers.set("library_delete", () => {
      throw { code: "IO_ERROR", message: "permission denied", recoverable: false };
    });
    await expect(deleteItem([video], "rec-1", true)).rejects.toMatchObject({ code: "IO_ERROR" });
  });

  it("exports through the save dialog and honours cancel", async () => {
    handlers.set("library_export", (a) => a.destination);
    const save = vi.fn(async () => null);
    expect(await exportItem(shot, save)).toBeNull();
    expect(callsTo("library_export")).toHaveLength(0);
    expect(save).toHaveBeenCalledWith({ defaultPath: "Login bug.png", filters: [{ name: "PNG image", extensions: ["png"] }], title: "Export recording" });
    const save2 = vi.fn(async () => "/tmp/out.mp4");
    expect(await exportItem(video, save2)).toBe("/tmp/out.mp4");
    expect(callsTo("library_export")[0]?.args).toEqual({ id: "rec-1", destination: "/tmp/out.mp4" });
    expect(exportFilters({ ...shot, fileName: "a.jpg", imageFormat: "jpeg", container: "jpeg" })[0]?.extensions).toEqual(["jpg", "jpeg"]);
  });

  it("re-validates and reveals", async () => {
    const invalid = { ...video, validation: { valid: false, checkedAt: "x", tool: "ffprobe", issues: [{ severity: "error" as const, code: "NO_STREAMS", message: "No video stream" }] } };
    handlers.set("library_validate", () => invalid);
    handlers.set("library_reveal", () => null);
    const { items, item } = await validateItem([video, shot], "rec-1");
    expect(item.validation?.valid).toBe(false);
    expect(items[0]?.validation?.valid).toBe(false);
    await revealItem("rec-1");
    expect(callsTo("library_reveal")[0]?.args).toEqual({ id: "rec-1" });
  });
});
