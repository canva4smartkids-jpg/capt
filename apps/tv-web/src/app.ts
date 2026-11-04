/** App shell: stage scaling, header navigation, screen routing, remote key dispatch, hub state. */
import type { CapabilityReport, HubEvent, HubInfo, RecordingStatus } from "@captureone/capture-contracts";
import { buildTvCapabilityReport, type TvDeviceInfo } from "./capabilities.js";
import { HubClient, HubError, type ConnectionState, type EventSubscription } from "./hub-client.js";
import { mapKey, type KeyAction } from "./keys.js";
import type { PlatformAdapter } from "./platform/types.js";
import { SpatialNavigator } from "./spatial.js";
import { clearConnection, loadConnection, saveConnection, type SavedConnection } from "./storage.js";
import { clear, h } from "./ui/dom.js";

export type ScreenId = "home" | "connect" | "recordings" | "remote" | "howto";

export interface Screen {
  readonly id: ScreenId;
  render(root: HTMLElement): void;
  /** Return true when the action was handled. */
  onAction?(a: KeyAction): boolean;
  dispose?(): void;
}

export interface Overlay {
  readonly el: HTMLElement;
  onAction?(a: KeyAction): boolean;
  dispose?(): void;
}

type Listener = () => void;

export const STAGE_WIDTH = 1920;
export const STAGE_HEIGHT = 1080;

export class App {
  readonly adapter: PlatformAdapter;
  readonly nav: SpatialNavigator;
  deviceInfo: TvDeviceInfo | null = null;
  report: CapabilityReport | null = null;
  connection: SavedConnection | null;
  client: HubClient | null = null;
  hubInfo: HubInfo | null = null;
  status: RecordingStatus | null = null;
  eventsState: ConnectionState = "closed";

  private readonly stage: HTMLElement;
  private readonly content: HTMLElement;
  private readonly tabs: Record<ScreenId, HTMLButtonElement>;
  private readonly connBadge: HTMLElement;
  private readonly toastEl: HTMLElement;
  private screens: Record<ScreenId, () => Screen>;
  private screen: Screen | null = null;
  private overlay: Overlay | null = null;
  private extraKeys: Record<number, KeyAction> = {};
  private events: EventSubscription | null = null;
  private listeners: { [k: string]: Listener[] } = {};
  private toastTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(root: HTMLElement, adapter: PlatformAdapter, screens: Record<ScreenId, (app: App) => Screen>) {
    this.adapter = adapter;
    this.connection = loadConnection();
    this.screens = {
      home: () => screens.home(this),
      connect: () => screens.connect(this),
      recordings: () => screens.recordings(this),
      remote: () => screens.remote(this),
      howto: () => screens.howto(this),
    };
    const tab = (id: ScreenId, label: string) =>
      h("button", { type: "button", class: "tab", "data-focusable": true, "data-tab": id, onclick: () => this.go(id) }, label);
    this.tabs = {
      home: tab("home", "Home"),
      connect: tab("connect", "Connect"),
      recordings: tab("recordings", "Recordings"),
      remote: tab("remote", "Record remotely"),
      howto: tab("howto", "How to record this TV"),
    };
    this.connBadge = h("div", { class: "conn-badge", id: "conn-badge" });
    this.content = h("main", { class: "content", id: "content", "data-scroll": true });
    this.toastEl = h("div", { class: "toast", role: "status", "aria-live": "polite" });
    this.stage = h(
      "div",
      { class: "stage", id: "stage" },
      h(
        "header",
        { class: "header" },
        h("div", { class: "brand" }, h("span", { class: "logo" }, "●"), "CaptureOne", h("span", { class: "brand-sub" }, " for TV")),
        h("nav", { class: "tabs" }, this.tabs.home, this.tabs.connect, this.tabs.recordings, this.tabs.remote, this.tabs.howto),
        this.connBadge,
      ),
      this.content,
      this.toastEl,
    );
    clear(root);
    root.appendChild(this.stage);
    this.nav = new SpatialNavigator(this.stage);
  }

  async start(): Promise<void> {
    this.fitStage();
    window.addEventListener("resize", () => this.fitStage());
    document.addEventListener("keydown", (e) => this.onKeyDown(e), true);
    try {
      this.extraKeys = this.adapter.init();
    } catch {
      this.extraKeys = {};
    }
    this.updateConnBadge();
    this.go(this.connection ? "home" : "home");
    try {
      this.deviceInfo = await this.adapter.getDeviceInfo();
    } catch {
      this.deviceInfo = { manufacturer: "unknown", osName: this.adapter.label, osVersion: "unknown", sources: [] };
    }
    this.report = buildTvCapabilityReport(this.adapter.id, this.deviceInfo);
    this.emit("device");
    if (this.connection) this.attachHub(this.connection);
  }

  /** Scales the 1920x1080 layout to the viewport (1280x720 on many TVs). */
  fitStage(): void {
    const w = window.innerWidth || STAGE_WIDTH;
    const hgt = window.innerHeight || STAGE_HEIGHT;
    const s = Math.min(w / STAGE_WIDTH, hgt / STAGE_HEIGHT);
    const st = this.stage.style as CSSStyleDeclaration & { webkitTransform?: string };
    st.transform = `scale(${s})`;
    st.webkitTransform = `scale(${s})`;
    st.left = `${Math.max(0, (w - STAGE_WIDTH * s) / 2)}px`;
    st.top = `${Math.max(0, (hgt - STAGE_HEIGHT * s) / 2)}px`;
  }

  on(event: "device" | "status" | "library" | "connection", cb: Listener): () => void {
    (this.listeners[event] = this.listeners[event] || []).push(cb);
    return () => {
      const l = this.listeners[event] || [];
      const i = l.indexOf(cb);
      if (i >= 0) l.splice(i, 1);
    };
  }

  private emit(event: string): void {
    const l = (this.listeners[event] || []).slice();
    for (const cb of l) {
      try {
        cb();
      } catch (e) {
        console.error(e);
      }
    }
  }

  currentScreen(): ScreenId | null {
    return this.screen ? this.screen.id : null;
  }

  go(id: ScreenId): void {
    if (this.overlay) this.closeOverlay();
    if (this.screen?.dispose) this.screen.dispose();
    this.listenersReset();
    clear(this.content);
    this.content.scrollTop = 0;
    for (const k of Object.keys(this.tabs) as ScreenId[]) {
      this.tabs[k].className = k === id ? "tab tab-active" : "tab";
    }
    this.screen = this.screens[id]();
    this.screen.render(this.content);
    this.nav.setScope(this.stage);
    const auto = this.content.querySelector<HTMLElement>("[data-autofocus]");
    if (auto) this.nav.focus(auto);
    else this.nav.focus(this.tabs[id]);
  }

  /** Screen-scoped listeners are dropped on navigation; app-level ones (none yet) survive. */
  private listenersReset(): void {
    this.listeners = {};
  }

  openOverlay(o: Overlay): void {
    if (this.overlay) this.closeOverlay();
    this.overlay = o;
    this.stage.appendChild(o.el);
    this.nav.setScope(o.el);
    this.nav.focusDefault();
  }

  closeOverlay(): void {
    const o = this.overlay;
    if (!o) return;
    this.overlay = null;
    if (o.dispose) o.dispose();
    if (o.el.parentNode) o.el.parentNode.removeChild(o.el);
    this.nav.setScope(this.stage);
    const auto = this.content.querySelector<HTMLElement>("[data-last-focus]") || this.content.querySelector<HTMLElement>("[data-autofocus]");
    if (auto) this.nav.focus(auto);
    else this.nav.focusDefault();
  }

  hasOverlay(): boolean {
    return this.overlay !== null;
  }

  private onKeyDown(e: KeyboardEvent): void {
    const action = mapKey({ key: e.key, keyCode: e.keyCode }, this.adapter.id, this.extraKeys);
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    this.dispatch(action);
  }

  /** Routes an action: overlay, then screen, then default navigation. Exposed for tests. */
  dispatch(action: KeyAction): void {
    if (this.overlay?.onAction && this.overlay.onAction(action)) return;
    if (!this.overlay && this.screen?.onAction && this.screen.onAction(action)) return;
    switch (action.type) {
      case "move":
        this.nav.move(action.dir);
        break;
      case "enter": {
        const cur = this.nav.current();
        if (cur) cur.click();
        else this.nav.focusDefault();
        break;
      }
      case "back":
        this.back();
        break;
      case "exit":
        this.adapter.exit();
        break;
      default:
        break;
    }
  }

  back(): void {
    if (this.overlay) {
      this.closeOverlay();
      return;
    }
    if (this.screen && this.screen.id !== "home") {
      this.go("home");
      return;
    }
    this.adapter.exit();
  }

  toast(msg: string, kind: "info" | "error" = "info"): void {
    this.toastEl.textContent = msg;
    this.toastEl.className = `toast toast-show toast-${kind}`;
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => {
      this.toastEl.className = "toast";
    }, 4000);
  }

  // ---- hub connection -------------------------------------------------------------------

  setConnection(c: SavedConnection, hub: HubInfo, client: HubClient): void {
    this.connection = c;
    saveConnection(c);
    this.hubInfo = hub;
    this.client = client;
    this.startEvents();
    this.updateConnBadge();
    this.emit("connection");
  }

  disconnect(): void {
    if (this.events) this.events.close();
    this.events = null;
    this.client = null;
    this.hubInfo = null;
    this.status = null;
    this.connection = null;
    clearConnection();
    this.eventsState = "closed";
    this.updateConnBadge();
    this.emit("connection");
  }

  private attachHub(c: SavedConnection): void {
    this.client = new HubClient(c.hubUrl, c.token);
    this.startEvents();
    this.client.info().then(
      (info) => {
        this.hubInfo = info;
        this.updateConnBadge();
        this.emit("connection");
      },
      () => this.updateConnBadge(),
    );
    this.client.status().then(
      (s) => {
        this.status = s;
        this.emit("status");
      },
      (e: unknown) => {
        if (e instanceof HubError && e.kind === "unauthorized") {
          this.toast("This TV is no longer paired with the desktop. Pair again.", "error");
          this.disconnect();
        }
      },
    );
  }

  private startEvents(): void {
    if (this.events) this.events.close();
    if (!this.client) return;
    this.events = this.client.subscribe(
      (ev: HubEvent) => {
        if (ev.type === "hello") {
          this.hubInfo = ev.hub;
          this.updateConnBadge();
          this.emit("connection");
        } else if (ev.type === "recording-status") {
          this.status = ev.status;
          this.emit("status");
        } else if (ev.type === "library-changed") {
          this.emit("library");
        }
      },
      (s) => {
        this.eventsState = s;
        this.updateConnBadge();
      },
    );
  }

  private updateConnBadge(): void {
    const c = this.connection;
    if (!c) {
      this.connBadge.className = "conn-badge conn-none";
      this.connBadge.textContent = "Not paired";
      return;
    }
    const name = this.hubInfo?.name || c.hubName || c.hubUrl;
    const live = this.eventsState === "open";
    this.connBadge.className = `conn-badge ${live ? "conn-live" : "conn-wait"}`;
    this.connBadge.textContent = `${live ? "Connected" : "Connecting"} · ${name}`;
  }
}
