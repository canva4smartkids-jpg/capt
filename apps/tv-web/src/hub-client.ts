/**
 * Client for the CaptureOne companion hub (packages/capture-contracts/src/companion.ts).
 * Uses only fetch + WebSocket (no AbortController/EventSource requirements, for 2018-era TV
 * browsers). Types come from the contract; runtime validation happens on the hub side and in
 * tests (zod is not bundled to keep the TV bundle small and ES2015-compatible).
 */
import type {
  CapabilityReport,
  DeviceKind,
  HubEvent,
  HubInfo,
  MediaMetadata,
  PairResponse,
  RecordingListResponse,
  RecordingStatus,
  RemoteCommand,
  RemoteCommandResult,
} from "@captureone/capture-contracts";
import { parseHubAddress } from "./keypad.js";

export const API = {
  info: "/api/v1/info",
  pair: "/api/v1/pair",
  recordings: "/api/v1/recordings",
  status: "/api/v1/status",
  command: "/api/v1/command",
  events: "/api/v1/events",
} as const;

export type HubErrorKind = "network" | "timeout" | "unauthorized" | "wrong-code" | "locked-out" | "bad-request" | "server";

export class HubError extends Error {
  readonly kind: HubErrorKind;
  readonly status: number;
  readonly retryAfterSec: number | undefined;
  readonly remainingAttempts: number | undefined;
  constructor(kind: HubErrorKind, message: string, status = 0, extra: { retryAfterSec?: number; remainingAttempts?: number } = {}) {
    super(message);
    this.name = "HubError";
    this.kind = kind;
    this.status = status;
    this.retryAfterSec = extra.retryAfterSec;
    this.remainingAttempts = extra.remainingAttempts;
  }
}

export type FetchLike = (url: string, init?: { method?: string; headers?: Record<string, string>; body?: string }) => Promise<{
  ok: boolean;
  status: number;
  headers: { get(name: string): string | null };
  text(): Promise<string>;
}>;

export interface WebSocketLike {
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  close(): void;
}
export type WebSocketFactory = (url: string) => WebSocketLike;

export interface HubClientDeps {
  fetch?: FetchLike;
  webSocket?: WebSocketFactory;
  timeoutMs?: number;
  setTimeout?: (fn: () => void, ms: number) => unknown;
  clearTimeout?: (id: unknown) => void;
}

/** `192.168.1.20` / `192.168.1.20:47800` / `http://...` → `http://192.168.1.20:47800`. */
export function normalizeHubUrl(input: string): string | null {
  const s = input.trim().replace(/\/+$/, "");
  const withoutScheme = s.replace(/^https?:\/\//i, "");
  const scheme = /^https:\/\//i.test(s) ? "https" : "http";
  const addr = parseHubAddress(withoutScheme);
  return addr ? `${scheme}://${addr.host}:${addr.port}` : null;
}

export interface EventSubscription {
  close(): void;
}

export type ConnectionState = "connecting" | "open" | "closed";

export class HubClient {
  readonly baseUrl: string;
  token: string | null;
  private readonly fetchImpl: FetchLike;
  private readonly wsFactory: WebSocketFactory;
  private readonly timeoutMs: number;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (id: unknown) => void;

  constructor(baseUrl: string, token: string | null = null, deps: HubClientDeps = {}) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.token = token;
    const g = globalThis as unknown as { fetch?: FetchLike; WebSocket?: new (url: string) => WebSocketLike };
    const f = deps.fetch ?? (g.fetch ? (g.fetch.bind(globalThis) as FetchLike) : undefined);
    if (!f) throw new Error("fetch is not available");
    this.fetchImpl = f;
    const W = g.WebSocket;
    this.wsFactory = deps.webSocket ?? ((url: string) => {
      if (!W) throw new Error("WebSocket is not available");
      return new W(url);
    });
    this.timeoutMs = deps.timeoutMs ?? 8000;
    this.setTimer = deps.setTimeout ?? ((fn, ms) => setTimeout(fn, ms));
    this.clearTimer = deps.clearTimeout ?? ((id) => clearTimeout(id as ReturnType<typeof setTimeout>));
  }

  private withToken(path: string): string {
    if (!this.token) return this.baseUrl + path;
    return `${this.baseUrl}${path}${path.indexOf("?") >= 0 ? "&" : "?"}token=${encodeURIComponent(this.token)}`;
  }

  private async request<T>(method: "GET" | "POST", path: string, body?: unknown, auth = true): Promise<T> {
    const headers: Record<string, string> = { Accept: "application/json" };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (auth) {
      if (!this.token) throw new HubError("unauthorized", "Not paired with a CaptureOne desktop yet");
      headers["Authorization"] = `Bearer ${this.token}`;
    }
    const init: { method: string; headers: Record<string, string>; body?: string } = { method, headers };
    if (body !== undefined) init.body = JSON.stringify(body);
    let timer: unknown;
    const timeout = new Promise<never>((_, reject) => {
      timer = this.setTimer(() => reject(new HubError("timeout", `The desktop at ${this.baseUrl} did not answer in time`)), this.timeoutMs);
    });
    let resp;
    try {
      resp = await Promise.race([this.fetchImpl(this.baseUrl + path, init), timeout]);
    } catch (e) {
      if (e instanceof HubError) throw e;
      throw new HubError("network", `Cannot reach the desktop at ${this.baseUrl}. Check the address and that the companion hub is enabled.`);
    } finally {
      this.clearTimer(timer);
    }
    const text = await resp.text();
    let json: unknown = null;
    if (text.length > 0) {
      try {
        json = JSON.parse(text);
      } catch {
        throw new HubError("server", `Unexpected reply from ${this.baseUrl} (is this a CaptureOne hub?)`, resp.status);
      }
    }
    if (resp.ok) return json as T;
    const err = (json && typeof json === "object" ? json : {}) as { error?: string; retryAfterSec?: number; remainingAttempts?: number };
    const msg = err.error ?? `HTTP ${resp.status}`;
    if (resp.status === 429) {
      const retry = err.retryAfterSec ?? Number(resp.headers.get("Retry-After") ?? "0");
      throw new HubError("locked-out", `Too many wrong codes. Try again in ${retry} s.`, 429, { retryAfterSec: retry });
    }
    if (resp.status === 401 && path === API.pair) {
      const extra: { remainingAttempts?: number } = {};
      if (err.remainingAttempts !== undefined) extra.remainingAttempts = err.remainingAttempts;
      throw new HubError("wrong-code", "Wrong pairing code", 401, extra);
    }
    if (resp.status === 401) throw new HubError("unauthorized", "This TV is no longer paired with the desktop. Pair again.", 401);
    if (resp.status === 400) throw new HubError("bad-request", msg, 400);
    throw new HubError("server", msg, resp.status);
  }

  info(): Promise<HubInfo> {
    return this.request<HubInfo>("GET", API.info, undefined, false);
  }

  async pair(code: string, deviceName: string, deviceKind: DeviceKind, capabilities?: CapabilityReport): Promise<PairResponse> {
    const body: { code: string; deviceName: string; deviceKind: DeviceKind; capabilities?: CapabilityReport } = { code, deviceName: deviceName.slice(0, 64), deviceKind };
    if (capabilities) body.capabilities = capabilities;
    const resp = await this.request<PairResponse>("POST", API.pair, body, false);
    this.token = resp.token;
    return resp;
  }

  async recordings(): Promise<MediaMetadata[]> {
    const r = await this.request<RecordingListResponse>("GET", API.recordings);
    return r && Array.isArray(r.items) ? r.items : [];
  }

  status(): Promise<RecordingStatus | null> {
    return this.request<RecordingStatus | null>("GET", API.status);
  }

  command(cmd: RemoteCommand): Promise<RemoteCommandResult> {
    return this.request<RemoteCommandResult>("POST", API.command, cmd);
  }

  /** Hub-relative paths from the list (`/api/v1/recordings/:id/file`) become absolute URLs with `?token=`. */
  fileUrl(item: MediaMetadata): string {
    const path = item.path.charAt(0) === "/" ? item.path : `/api/v1/recordings/${encodeURIComponent(item.id)}/file`;
    return this.withToken(path);
  }

  thumbnailUrl(item: MediaMetadata): string | null {
    if (!item.thumbnailPath) return item.kind === "screenshot" ? this.fileUrl(item) : null;
    const path = item.thumbnailPath.charAt(0) === "/" ? item.thumbnailPath : `/api/v1/recordings/${encodeURIComponent(item.id)}/thumbnail`;
    return this.withToken(path);
  }

  eventsUrl(): string {
    return this.withToken(API.events).replace(/^http/i, "ws");
  }

  /**
   * Subscribes to hub events with automatic reconnection (1 s, 2 s, 4 s ... capped at 30 s).
   */
  subscribe(onEvent: (e: HubEvent) => void, onState?: (s: ConnectionState) => void): EventSubscription {
    let closed = false;
    let ws: WebSocketLike | null = null;
    let attempt = 0;
    let retryTimer: unknown = null;
    const connect = () => {
      if (closed) return;
      onState?.("connecting");
      try {
        ws = this.wsFactory(this.eventsUrl());
      } catch {
        scheduleRetry();
        return;
      }
      ws.onopen = () => {
        attempt = 0;
        onState?.("open");
      };
      ws.onmessage = (ev) => {
        if (typeof ev.data !== "string") return;
        try {
          const parsed = JSON.parse(ev.data) as HubEvent;
          if (parsed && typeof parsed.type === "string") onEvent(parsed);
        } catch {
          /* ignore malformed frames */
        }
      };
      ws.onerror = () => {
        /* onclose follows */
      };
      ws.onclose = () => {
        ws = null;
        if (!closed) {
          onState?.("closed");
          scheduleRetry();
        }
      };
    };
    const scheduleRetry = () => {
      if (closed) return;
      const delay = Math.min(30000, 1000 * Math.pow(2, attempt));
      attempt++;
      retryTimer = this.setTimer(connect, delay);
    };
    connect();
    return {
      close: () => {
        closed = true;
        if (retryTimer !== null) this.clearTimer(retryTimer);
        if (ws) {
          ws.onclose = null;
          ws.close();
        }
        onState?.("closed");
      },
    };
  }
}
