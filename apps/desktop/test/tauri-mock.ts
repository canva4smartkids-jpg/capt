/**
 * In-memory stand-ins for @tauri-apps/api so UI logic can be tested without a webview.
 * Usage (hoisted per test file):
 *   vi.mock("@tauri-apps/api/core", async () => (await import("./tauri-mock")).coreMock);
 *   vi.mock("@tauri-apps/api/event", async () => (await import("./tauri-mock")).eventMock);
 */
import { vi } from "vitest";

type Handler = (args: Record<string, unknown>) => unknown;

export const handlers = new Map<string, Handler>();
export const calls: { cmd: string; args: Record<string, unknown> }[] = [];
const listeners = new Map<string, Set<(e: { event: string; payload: unknown; id: number }) => void>>();

export const coreMock = {
  invoke: vi.fn(async (cmd: string, args: Record<string, unknown> = {}) => {
    calls.push({ cmd, args });
    const h = handlers.get(cmd);
    if (!h) throw { code: "UNSUPPORTED", message: `no mock for ${cmd}`, recoverable: false };
    return h(args);
  }),
  convertFileSrc: (p: string) => `http://asset.localhost/${encodeURIComponent(p)}`,
};

export const eventMock = {
  listen: vi.fn(async (name: string, cb: (e: { event: string; payload: unknown; id: number }) => void) => {
    let set = listeners.get(name);
    if (!set) listeners.set(name, (set = new Set()));
    set.add(cb);
    return () => set?.delete(cb);
  }),
};

export const dialogMock = {
  save: vi.fn(async () => null as string | null),
  open: vi.fn(async () => null as string | null),
};

export const windowMock = {
  getCurrentWindow: () => ({ show: vi.fn(async () => undefined), setFocus: vi.fn(async () => undefined), close: vi.fn(async () => undefined) }),
};

export function emit(name: string, payload: unknown): void {
  listeners.get(name)?.forEach((cb) => cb({ event: name, payload, id: 0 }));
}

export function listenerCount(name: string): number {
  return listeners.get(name)?.size ?? 0;
}

export function resetTauri(): void {
  handlers.clear();
  calls.length = 0;
  listeners.clear();
  coreMock.invoke.mockClear();
  eventMock.listen.mockClear();
  dialogMock.save.mockReset();
  dialogMock.save.mockResolvedValue(null);
  dialogMock.open.mockReset();
  dialogMock.open.mockResolvedValue(null);
}

export function callsTo(cmd: string) {
  return calls.filter((c) => c.cmd === cmd);
}
