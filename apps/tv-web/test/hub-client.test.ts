import { HubEvent, HubInfo, PairRequest, RemoteCommand } from "@captureone/capture-contracts";
import { describe, expect, it, vi } from "vitest";
import { HubClient, HubError, type FetchLike, type WebSocketLike } from "../src/hub-client.js";
import { buildTvCapabilityReport } from "../src/capabilities.js";

const hub = HubInfo.parse({
  protocolVersion: 1,
  hubId: "hub-1",
  name: "Studio PC",
  platform: "linux",
  appVersion: "0.1.0",
  capabilities: {
    schemaVersion: 1,
    platform: "linux",
    osName: "Linux",
    osVersion: "6.8",
    appVersion: "0.1.0",
    generatedAt: "2026-11-04T10:00:00Z",
    features: {},
    encoders: [],
    frameRates: [30],
    screenshotFormats: ["png"],
  },
});

interface Call {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

function mockFetch(handler: (c: Call) => { status: number; body?: unknown; headers?: Record<string, string> }): { fetch: FetchLike; calls: Call[] } {
  const calls: Call[] = [];
  const fetch: FetchLike = async (url, init) => {
    const c: Call = { url, method: init?.method ?? "GET", headers: init?.headers ?? {}, body: init?.body ? JSON.parse(init.body) : undefined };
    calls.push(c);
    const r = handler(c);
    const hdrs = r.headers ?? {};
    return {
      ok: r.status >= 200 && r.status < 300,
      status: r.status,
      headers: { get: (n: string) => hdrs[n] ?? hdrs[n.toLowerCase()] ?? null },
      text: async () => (r.body === undefined ? "" : JSON.stringify(r.body)),
    };
  };
  return { fetch, calls };
}

const TOKEN = "a".repeat(64);

describe("HubClient REST", () => {
  it("gets info without auth and pairs with a valid PairRequest", async () => {
    const { fetch, calls } = mockFetch((c) => (c.url.endsWith("/api/v1/info") ? { status: 200, body: hub } : { status: 200, body: { token: TOKEN, hub } }));
    const client = new HubClient("http://10.0.0.5:47800/", null, { fetch });
    expect((await client.info()).name).toBe("Studio PC");
    expect(calls[0]!.headers["Authorization"]).toBeUndefined();
    const caps = buildTvCapabilityReport("webos", { manufacturer: "LG", osName: "webOS TV", osVersion: "6.0", sources: [] });
    const resp = await client.pair("123456", "LG webOS TV (OLED55C1)", "lg-webos-tv", caps);
    expect(resp.token).toBe(TOKEN);
    expect(client.token).toBe(TOKEN);
    expect(calls[1]!.url).toBe("http://10.0.0.5:47800/api/v1/pair");
    expect(calls[1]!.method).toBe("POST");
    expect(PairRequest.safeParse(calls[1]!.body).success).toBe(true);
  });

  it("sends the bearer token and builds media URLs with ?token=", async () => {
    const item = {
      id: "rec 1",
      kind: "video",
      path: "/api/v1/recordings/rec%201/file",
      fileName: "a.mp4",
      title: "A",
      createdAt: "2026-11-04T10:00:00Z",
      sizeBytes: 1,
      width: 1920,
      height: 1080,
      audioTracks: [],
      container: "mp4",
      thumbnailPath: "/api/v1/recordings/rec%201/thumbnail",
    };
    const { fetch, calls } = mockFetch(() => ({ status: 200, body: { items: [item] } }));
    const client = new HubClient("http://10.0.0.5:47800", TOKEN, { fetch });
    const items = await client.recordings();
    expect(calls[0]!.headers["Authorization"]).toBe(`Bearer ${TOKEN}`);
    expect(client.fileUrl(items[0]!)).toBe(`http://10.0.0.5:47800/api/v1/recordings/rec%201/file?token=${TOKEN}`);
    expect(client.thumbnailUrl(items[0]!)).toBe(`http://10.0.0.5:47800/api/v1/recordings/rec%201/thumbnail?token=${TOKEN}`);
    expect(client.eventsUrl()).toBe(`ws://10.0.0.5:47800/api/v1/events?token=${TOKEN}`);
  });

  it("posts valid RemoteCommands and returns the result", async () => {
    const { fetch, calls } = mockFetch(() => ({ status: 200, body: { ok: true, sessionId: "s1" } }));
    const client = new HubClient("http://h:1", TOKEN, { fetch });
    const r = await client.command({ type: "start-recording", preset: "capture-device" });
    expect(r).toEqual({ ok: true, sessionId: "s1" });
    expect(RemoteCommand.parse(calls[0]!.body)).toEqual({ type: "start-recording", preset: "capture-device" });
  });

  it("maps pairing errors", async () => {
    const wrong = new HubClient("http://h:1", null, { fetch: mockFetch(() => ({ status: 401, body: { error: "wrong pairing code", remainingAttempts: 3 } })).fetch });
    await expect(wrong.pair("000000", "TV", "lg-webos-tv")).rejects.toMatchObject({ kind: "wrong-code", remainingAttempts: 3 });
    const locked = new HubClient("http://h:1", null, {
      fetch: mockFetch(() => ({ status: 429, body: { error: "too many", retryAfterSec: 30 }, headers: { "Retry-After": "30" } })).fetch,
    });
    await expect(locked.pair("000000", "TV", "lg-webos-tv")).rejects.toMatchObject({ kind: "locked-out", retryAfterSec: 30 });
    const revoked = new HubClient("http://h:1", TOKEN, { fetch: mockFetch(() => ({ status: 401, body: { error: "missing or invalid token" } })).fetch });
    await expect(revoked.status()).rejects.toMatchObject({ kind: "unauthorized" });
  });

  it("refuses authenticated calls before pairing and reports network errors and timeouts", async () => {
    const client = new HubClient("http://h:1", null, { fetch: mockFetch(() => ({ status: 200 })).fetch });
    await expect(client.recordings()).rejects.toMatchObject({ kind: "unauthorized" });
    const down = new HubClient("http://h:1", TOKEN, { fetch: () => Promise.reject(new TypeError("Failed to fetch")) });
    await expect(down.status()).rejects.toBeInstanceOf(HubError);
    await expect(down.status()).rejects.toMatchObject({ kind: "network" });
    vi.useFakeTimers();
    const hang = new HubClient("http://h:1", TOKEN, { fetch: () => new Promise(() => undefined), timeoutMs: 1000 });
    const p = hang.status();
    const assertion = expect(p).rejects.toMatchObject({ kind: "timeout" });
    await vi.advanceTimersByTimeAsync(1001);
    await assertion;
    vi.useRealTimers();
  });

  it("parses a null status", async () => {
    const client = new HubClient("http://h:1", TOKEN, { fetch: mockFetch(() => ({ status: 200, body: null })).fetch });
    expect(await client.status()).toBeNull();
  });
});

class FakeSocket implements WebSocketLike {
  static all: FakeSocket[] = [];
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  closed = false;
  constructor(readonly url: string) {
    FakeSocket.all.push(this);
  }
  close() {
    this.closed = true;
  }
}

describe("HubClient events", () => {
  it("delivers hub events and reconnects with backoff", () => {
    vi.useFakeTimers();
    FakeSocket.all = [];
    const events: HubEvent[] = [];
    const states: string[] = [];
    const client = new HubClient("http://10.0.0.5:47800", TOKEN, { fetch: mockFetch(() => ({ status: 200 })).fetch, webSocket: (u) => new FakeSocket(u) });
    const sub = client.subscribe((e) => events.push(e), (s) => states.push(s));
    const ws = FakeSocket.all[0]!;
    expect(ws.url).toBe(`ws://10.0.0.5:47800/api/v1/events?token=${TOKEN}`);
    ws.onopen?.({});
    ws.onmessage?.({ data: JSON.stringify({ type: "hello", hub }) });
    ws.onmessage?.({ data: JSON.stringify({ type: "recording-status", status: null }) });
    ws.onmessage?.({ data: "not json" });
    ws.onmessage?.({ data: JSON.stringify({ type: "library-changed" }) });
    expect(events.map((e) => e.type)).toEqual(["hello", "recording-status", "library-changed"]);
    for (const e of events) expect(HubEvent.safeParse(e).success).toBe(true);

    ws.onclose?.({});
    expect(FakeSocket.all.length).toBe(1);
    vi.advanceTimersByTime(1000);
    expect(FakeSocket.all.length).toBe(2); // retried after 1 s
    FakeSocket.all[1]!.onclose?.({});
    vi.advanceTimersByTime(1999);
    expect(FakeSocket.all.length).toBe(2);
    vi.advanceTimersByTime(1);
    expect(FakeSocket.all.length).toBe(3); // then 2 s
    sub.close();
    expect(FakeSocket.all[2]!.closed).toBe(true);
    vi.advanceTimersByTime(60000);
    expect(FakeSocket.all.length).toBe(3); // no reconnect after close
    expect(states[0]).toBe("connecting");
    expect(states).toContain("open");
    vi.useRealTimers();
  });
});
