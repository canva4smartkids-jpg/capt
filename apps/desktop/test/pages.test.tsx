import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DESKTOP_EVENTS, type CapabilityReport } from "@captureone/capture-contracts";
import { callsTo, emit, handlers, resetTauri } from "./tauri-mock";
import { capabilities, settings, shot, sources, video } from "./fixtures";

vi.mock("@tauri-apps/api/core", async () => (await import("./tauri-mock")).coreMock);
vi.mock("@tauri-apps/api/event", async () => (await import("./tauri-mock")).eventMock);
vi.mock("@tauri-apps/plugin-dialog", async () => (await import("./tauri-mock")).dialogMock);
vi.mock("@tauri-apps/api/window", async () => (await import("./tauri-mock")).windowMock);

const { App } = await import("../src/App");

function backend(cap: CapabilityReport = capabilities) {
  handlers.set("get_settings", () => settings);
  handlers.set("save_settings", (a) => a.settings);
  handlers.set("get_capabilities", () => cap);
  handlers.set("list_audio_devices", () => [{ id: "mic0", name: "USB Mic", kind: "input", isDefault: true }]);
  handlers.set("get_active_session", () => null);
  handlers.set("list_sources", () => sources);
  handlers.set("get_source_thumbnail", () => "data:image/png;base64,iVBORw0KGgo=");
  handlers.set("library_storage", () => ({ totalBytes: 12_595_678, videoCount: 1, screenshotCount: 1, freeDiskBytes: 50e9, outputDirectory: settings.outputDirectory }));
  handlers.set("library_list", () => [video, shot]);
  handlers.set("get_permission_status", (a) => ({ kind: a.kind, state: "not-applicable", requiresSystemSettings: false }));
  handlers.set("get_diagnostics", () => ({
    appVersion: "0.1.0",
    platform: "linux",
    osVersion: "Ubuntu 24.04",
    ffmpegPath: "/usr/bin/ffmpeg",
    ffmpegVersion: "6.1.1",
    ffprobePath: "/usr/bin/ffprobe",
    dataDirectory: "/d",
    logDirectory: "/l",
    databasePath: "/d/library.sqlite3",
  }));
}

function at(hash: string) {
  window.location.hash = hash;
  return render(<App />);
}

beforeEach(() => {
  resetTauri();
  window.location.hash = "";
});

describe("Record page", () => {
  const withDevice = { ...capabilities, features: { ...capabilities.features, hdmiCaptureDevice: { level: "partial" as const, reason: "requires a UVC/HDMI capture card; HDCP-protected sources are blank" } } };

  it("lists capture devices in their own tab and records the selected one", async () => {
    backend(withDevice);
    handlers.set("list_sources", () => [...sources, { id: "v4l2:/dev/video0", kind: "device", name: "Cam Link 4K", width: 1920, height: 1080 }]);
    at("#/");
    await userEvent.click(await screen.findByRole("tab", { name: /Capture devices/ }));
    const card = await screen.findByRole("radio", { name: /Cam Link 4K/ });
    expect(screen.getByText(/HDCP-protected sources are blank/)).toBeTruthy();
    expect(callsTo("list_sources")[0]!.args.kinds).toContain("device");
    handlers.set("start_recording", () => "sess-2");
    await userEvent.click(card);
    await userEvent.click(await screen.findByRole("button", { name: /Start recording/ }));
    await waitFor(() => expect(callsTo("start_recording")).toHaveLength(1));
    expect((callsTo("start_recording")[0]!.args.config as { target: unknown }).target).toEqual({ kind: "device", deviceId: "v4l2:/dev/video0" });
  });

  it("shows an empty state when no capture card is detected", async () => {
    backend(withDevice);
    at("#/");
    await userEvent.click(await screen.findByRole("tab", { name: /Capture devices/ }));
    expect(await screen.findByText("No capture card detected")).toBeTruthy();
  });

  it("starts a recording with a contract config, shows the countdown and cancels with Esc", async () => {
    backend();
    at("#/");
    await screen.findByRole("radio", { name: /Display 1/ });
    const start = await screen.findByRole("button", { name: /Start recording/ });
    await waitFor(() => expect((start as HTMLButtonElement).disabled).toBe(false));
    handlers.set("start_recording", () => "sess-1");
    handlers.set("cancel_recording", () => null);
    await userEvent.click(start);
    await waitFor(() => expect(callsTo("start_recording")).toHaveLength(1));
    const cfg = callsTo("start_recording")[0]!.args.config as { target: unknown; video: { codec: string }; output: { directory: string }; countdownSec: number };
    expect(cfg.target).toEqual({ kind: "display", displayId: "disp-0" });
    expect(cfg.video.codec).toBe("h264");
    expect(cfg.output.directory).toBe(settings.outputDirectory);
    expect(cfg.countdownSec).toBe(3);

    act(() => emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "countdown", sessionId: "sess-1", remainingSec: 3 }));
    const overlay = await screen.findByRole("alertdialog", { name: /Recording starts in/ });
    expect(within(overlay).getByText("3")).toBeTruthy();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(callsTo("cancel_recording")[0]?.args).toEqual({ sessionId: "sess-1" }));
    act(() => emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "state-changed", sessionId: "sess-1", from: "countdown", to: "idle", at: "t" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  });

  it("shows live status and stops through stop_recording", async () => {
    backend();
    at("#/");
    await screen.findByRole("radio", { name: /Display 1/ });
    handlers.set("stop_recording", () => video);
    act(() => {
      emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "state-changed", sessionId: "tray-1", from: "starting", to: "recording", at: "t" });
      emit(DESKTOP_EVENTS.CAPTURE_EVENT, { type: "status", status: { sessionId: "tray-1", state: "recording", elapsedMs: 65_000, bytesWritten: 5 * 1024 * 1024, framesEncoded: 1950, framesDropped: 3, warnings: [] } });
    });
    expect(await screen.findByRole("timer", { name: /Elapsed 01:0[5-7]/ })).toBeTruthy();
    expect(screen.getByText("5.0 MB")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(callsTo("stop_recording")[0]?.args).toEqual({ sessionId: "tray-1" }));
    expect(await screen.findByText("Recording saved")).toBeTruthy();
  });

  it("disables system audio with the reason when unsupported, and gates H.265", async () => {
    const { systemAudio: _x, ...features } = capabilities.features;
    backend({ ...capabilities, features: { ...features, systemAudio: { level: "unsupported", reason: "X11 has no loopback capture in this build" } } });
    at("#/");
    const sw = (await screen.findByRole("switch", { name: "System audio" })) as HTMLButtonElement;
    await waitFor(() => expect(sw.disabled).toBe(true));
    expect(screen.getByText("X11 has no loopback capture in this build")).toBeTruthy();
    const codec = screen.getByLabelText("Codec") as HTMLSelectElement;
    const h265 = Array.from(codec.options).find((o) => o.value === "h265");
    expect(h265?.disabled).toBe(true);
  });

  it("selects a region through select_region and records it as the target", async () => {
    backend();
    handlers.set("select_region", () => ({ x: 100, y: 50, width: 800, height: 600 }));
    at("#/");
    await screen.findByRole("radio", { name: /Display 1/ });
    await userEvent.click(screen.getByRole("tab", { name: /Region/ }));
    await userEvent.click(await screen.findByRole("button", { name: /Select region/ }));
    await waitFor(() => expect(callsTo("select_region")[0]?.args).toEqual({ displayId: "disp-0" }));
    expect(await screen.findByText("800 × 600 px")).toBeTruthy();
  });
});

describe("Library page", () => {
  it("lists items, filters, and deletes with confirmation", async () => {
    backend();
    handlers.set("library_delete", () => null);
    at("#/library");
    expect(await screen.findByText("Login bug")).toBeTruthy();
    expect(screen.getByText("CaptureOne 2026-11-04 12-00-00")).toBeTruthy();
    await userEvent.click(screen.getByRole("radio", { name: /Screenshots 1/ }));
    expect(screen.queryByText("CaptureOne 2026-11-04 12-00-00")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Delete Login bug" }));
    const dlg = await screen.findByRole("alertdialog");
    expect(within(dlg).getByRole("checkbox")).toHaveProperty("checked", true);
    await userEvent.click(within(dlg).getByRole("button", { name: /Delete file/ }));
    await waitFor(() => expect(callsTo("library_delete")[0]?.args).toEqual({ id: "shot-1", deleteFile: true }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Open Login bug" })).toBeNull());
    expect(await screen.findByText("Deleted")).toBeTruthy();
  });

  it("renames inline with Enter", async () => {
    backend();
    handlers.set("library_rename", (a) => ({ ...video, title: String(a.title) }));
    at("#/library");
    await screen.findByText("Login bug");
    await userEvent.click(screen.getByRole("button", { name: "Rename CaptureOne 2026-11-04 12-00-00" }));
    const input = screen.getByLabelText("New title");
    await userEvent.clear(input);
    await userEvent.type(input, "Sprint demo{Enter}");
    await waitFor(() => expect(callsTo("library_rename")[0]?.args).toEqual({ id: "rec-1", title: "Sprint demo" }));
    expect(await screen.findByText("Sprint demo")).toBeTruthy();
  });
});

describe("Settings page", () => {
  it("validates and saves", async () => {
    backend();
    at("#/settings");
    const folder = (await screen.findByLabelText("Output folder")) as HTMLInputElement;
    await waitFor(() => expect(folder.value).toBe(settings.outputDirectory));
    await userEvent.clear(folder);
    expect(await screen.findByText("Choose an output folder.")).toBeTruthy();
    expect((screen.getByRole("button", { name: /Save changes/ }) as HTMLButtonElement).disabled).toBe(true);
    await userEvent.type(folder, "/data/rec");
    await userEvent.click(screen.getByRole("radio", { name: /Dark/ }));
    await userEvent.click(screen.getByRole("button", { name: /Save changes/ }));
    await waitFor(() => expect(callsTo("save_settings")).toHaveLength(1));
    expect(callsTo("save_settings")[0]!.args.settings).toMatchObject({ outputDirectory: "/data/rec", theme: "dark" });
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"));
    expect(await screen.findByText(/6\.1\.1 — \/usr\/bin\/ffmpeg/)).toBeTruthy();
  });
});

describe("Devices page", () => {
  it("is honest when the companion hub is not integrated", async () => {
    backend();
    handlers.set("hub_status", () => {
      throw { code: "UNSUPPORTED", message: "Companion hub is not integrated in this build", recoverable: false };
    });
    handlers.set("discovery_scan", () => {
      throw { code: "UNSUPPORTED", message: "Companion hub is not integrated in this build", recoverable: false };
    });
    at("#/devices");
    expect(await screen.findByText("The companion hub is not available in this build.")).toBeTruthy();
    expect((screen.getByRole("switch", { name: /Allow companions/ }) as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(screen.getAllByRole("button", { name: /Scan/ })[0]!);
    expect(await screen.findByText("Network discovery is not available in this build.")).toBeTruthy();
  });

  it("renders discovered TVs with LG-only screenshot and a remote D-pad", async () => {
    backend();
    handlers.set("hub_status", () => ({ enabled: true, url: "http://192.168.1.20:47800", pairingCode: "123456", pairedDevices: [] }));
    handlers.set("discovery_scan", () => [
      { id: "lg1", name: "Living room", kind: "lg-webos-tv", address: "192.168.1.30", discoveredVia: ["ssdp"], controlProtocols: ["lg-ssap"], services: [], lastSeen: "2026-11-04T12:00:00Z" },
      { id: "sam1", name: "Bedroom", kind: "samsung-tizen-tv", address: "192.168.1.31", discoveredVia: ["mdns"], controlProtocols: ["samsung-remote-ws"], services: [], lastSeen: "2026-11-04T12:00:00Z" },
    ]);
    handlers.set("tv_send_key", () => null);
    at("#/devices");
    expect(await screen.findByLabelText("Pairing code 1 2 3 4 5 6")).toBeTruthy();
    await userEvent.click(screen.getAllByRole("button", { name: /Scan/ })[0]!);
    await screen.findByText("Living room");
    expect(screen.getAllByRole("button", { name: /Screenshot/ })).toHaveLength(1);
    await userEvent.click(screen.getAllByRole("button", { name: /Remote/ })[1]!);
    const dlg = await screen.findByRole("dialog", { name: /Remote · Bedroom/ });
    await userEvent.click(within(dlg).getByRole("button", { name: "Up" }));
    await waitFor(() => expect(callsTo("tv_send_key")[0]?.args).toEqual({ deviceId: "sam1", key: "UP" }));
  });
});

describe("Devices page with the companion hub", () => {
  it("turns the hub on and shows address, pairing code and paired devices", async () => {
    backend();
    let enabled = false;
    const st = () => (enabled ? { enabled: true, url: "http://192.168.1.20:47800", pairingCode: "482913", pairedDevices: [{ name: "Living room TV", deviceKind: "lg-webos-tv", pairedAt: "2026-11-04T12:00:00Z" }] } : { enabled: false, url: null, pairingCode: null, pairedDevices: [] });
    handlers.set("hub_status", st);
    handlers.set("hub_set_enabled", (a) => {
      enabled = Boolean(a.enabled);
      return st();
    });
    at("#/devices");
    const toggle = (await screen.findByRole("switch", { name: /Allow companions/ })) as HTMLButtonElement;
    await waitFor(() => expect(toggle.disabled).toBe(false));
    await userEvent.click(toggle);
    await waitFor(() => expect(callsTo("hub_set_enabled")[0]?.args).toEqual({ enabled: true }));
    expect(await screen.findByLabelText("Pairing code 4 8 2 9 1 3")).toBeTruthy();
    expect(screen.getByText("Listening at http://192.168.1.20:47800")).toBeTruthy();
    expect(screen.getByText("Living room TV")).toBeTruthy();
    expect(screen.getByText(/LG TV \(webOS\) · paired/)).toBeTruthy();
  });
});

describe("Library TV captures", () => {
  it("labels screenshots taken on a TV as experimental", async () => {
    backend();
    handlers.set("library_list", () => [{ ...shot, id: "tv-1", title: "Living room — TV screenshot (experimental)", platform: "webos", sourceName: "Living room (LG webOS TV)" }, video]);
    at("#/library");
    expect(await screen.findByText("From TV · experimental")).toBeTruthy();
    expect(screen.getAllByText("From TV · experimental")).toHaveLength(1);
  });
});

describe("Region window", () => {
  it("drags a selection and confirms it in physical pixels; Esc cancels", async () => {
    handlers.set("get_source_thumbnail", () => "data:image/png;base64,iVBORw0KGgo=");
    handlers.set("region_selection_done", () => null);
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    HTMLElement.prototype.setPointerCapture = () => undefined;
    const { unmount } = at("#/region?displayId=disp-0&w=2048&h=1536");
    const root = await screen.findByRole("application");
    fireEvent.pointerDown(root, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(root, { clientX: 500, clientY: 400, pointerId: 1 });
    fireEvent.pointerUp(root, { pointerId: 1 });
    expect(screen.getByText("800 × 600")).toBeTruthy();
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(callsTo("region_selection_done")[0]?.args).toEqual({ rect: { x: 200, y: 200, width: 800, height: 600 } }));
    unmount();
    resetTauri();
    handlers.set("get_source_thumbnail", () => "data:image/png;base64,iVBORw0KGgo=");
    handlers.set("region_selection_done", () => null);
    at("#/region?displayId=disp-0&w=2048&h=1536");
    await screen.findByRole("application");
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(callsTo("region_selection_done")[0]?.args).toEqual({ rect: null }));
    Object.defineProperty(window, "devicePixelRatio", { value: 1, configurable: true });
  });
});
