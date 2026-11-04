import type { MediaMetadata } from "@captureone/capture-contracts";
import type { App, Screen } from "../app.js";
import { HubError } from "../hub-client.js";
import { button, clear, formatBytes, formatDate, formatDuration, h } from "../ui/dom.js";
import { createImageViewer, createPlayer } from "./player.js";

export function describe(m: MediaMetadata): string {
  const parts: string[] = [];
  if (m.kind === "video") parts.push(formatDuration(m.durationMs));
  parts.push(`${m.width}×${m.height}`);
  if (m.videoCodec) parts.push(m.videoCodec.toUpperCase());
  parts.push(formatBytes(m.sizeBytes));
  return parts.filter((p) => p).join(" · ");
}

export function createRecordings(app: App): Screen {
  let grid: HTMLElement;
  let statusEl: HTMLElement;
  let loading = false;

  const open = (m: MediaMetadata, card: HTMLElement) => {
    const client = app.client;
    if (!client) return;
    const prev = grid.querySelector("[data-last-focus]");
    if (prev) prev.removeAttribute("data-last-focus");
    card.setAttribute("data-last-focus", "");
    app.openOverlay(m.kind === "video" ? createPlayer(app, m, client.fileUrl(m)) : createImageViewer(app, m, client.fileUrl(m)));
  };

  const load = async () => {
    const client = app.client;
    if (!client || loading) return;
    loading = true;
    statusEl.textContent = "Loading recordings…";
    statusEl.className = "status";
    try {
      const items = await client.recordings();
      clear(grid);
      if (items.length === 0) {
        statusEl.textContent = "No recordings on the computer yet.";
        return;
      }
      statusEl.textContent = `${items.length} item${items.length === 1 ? "" : "s"} on ${app.hubInfo?.name ?? "the computer"}`;
      items.forEach((m, idx) => {
        const thumb = client.thumbnailUrl(m);
        const card: HTMLButtonElement = h(
          "button",
          {
            type: "button",
            class: "media-card",
            "data-focusable": true,
            "data-media-id": m.id,
            "data-autofocus": idx === 0,
            onclick: () => open(m, card),
          },
          h(
            "div",
            { class: "thumb" },
            thumb ? h("img", { src: thumb, alt: "", loading: "lazy" }) : h("div", { class: "thumb-none" }, m.kind === "video" ? "▶" : "▣"),
            h("span", { class: `kind kind-${m.kind}` }, m.kind === "video" ? "Video" : "Screenshot"),
          ),
          h("div", { class: "media-title" }, m.title || m.fileName),
          h("div", { class: "media-meta" }, describe(m)),
          h("div", { class: "media-date" }, formatDate(m.createdAt)),
        );
        grid.appendChild(card);
      });
      if (!app.hasOverlay()) {
        const first = grid.querySelector<HTMLElement>("[data-autofocus]");
        const cur = app.nav.current();
        if (first && (!cur || !cur.isConnected)) app.nav.focus(first);
      }
    } catch (e) {
      statusEl.textContent = e instanceof HubError ? e.message : "Could not load recordings.";
      statusEl.className = "status status-error";
      if (e instanceof HubError && e.kind === "unauthorized") app.disconnect();
    } finally {
      loading = false;
    }
  };

  return {
    id: "recordings",
    render(root) {
      if (!app.client) {
        root.appendChild(
          h(
            "section",
            { class: "card" },
            h("h2", null, "Recordings"),
            h("p", null, "Pair this TV with CaptureOne on your computer to browse and play its recordings."),
            h("div", { class: "actions" }, button("Pair with a computer", () => app.go("connect"), { autofocus: true })),
          ),
        );
        return;
      }
      statusEl = h("div", { class: "status", id: "recordings-status" });
      grid = h("div", { class: "media-grid", id: "media-grid" });
      root.appendChild(
        h(
          "section",
          { class: "card" },
          h("div", { class: "card-head" }, h("h2", null, "Recordings"), button("Refresh", () => void load(), { id: "refresh" })),
          statusEl,
          grid,
        ),
      );
      app.on("library", () => void load());
      void load();
    },
  };
}
