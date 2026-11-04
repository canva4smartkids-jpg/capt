/** Full-screen playback of hub recordings with remote transport keys. */
import type { MediaMetadata } from "@captureone/capture-contracts";
import type { App, Overlay } from "../app.js";
import type { KeyAction } from "../keys.js";
import { formatDuration, h } from "../ui/dom.js";

const SEEK_STEP_SEC = 10;

const MEDIA_ERRORS: Record<number, string> = {
  1: "Playback was aborted.",
  2: "A network error interrupted playback. Is the computer still on?",
  3: "This TV could not decode the video (unsupported codec or profile).",
  4: "This TV cannot play this file format.",
};

export function createPlayer(app: App, m: MediaMetadata, src: string): Overlay {
  const video = h("video", { class: "player-video", id: "player-video", autoplay: true, preload: "auto" });
  const title = h("div", { class: "player-title" }, m.title || m.fileName);
  const fill = h("div", { class: "progress-fill" });
  const time = h("div", { class: "player-time" }, "0:00");
  const message = h("div", { class: "player-message" });
  const hint = h("div", { class: "player-hint" }, "OK: play/pause   ◀ ▶: −/+10 s   Back: close");
  const chrome = h("div", { class: "player-chrome" }, title, h("div", { class: "progress" }, fill), h("div", { class: "player-row" }, time, hint));
  const el = h("div", { class: "overlay player", id: "player", "data-state": "loading", "data-focusable": true, tabindex: 0 }, video, message, chrome);
  let hideTimer: ReturnType<typeof setTimeout> | null = null;

  const showChrome = () => {
    chrome.className = "player-chrome player-chrome-show";
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (!video.paused) chrome.className = "player-chrome";
    }, 3000);
  };
  const update = () => {
    const d = isFinite(video.duration) && video.duration > 0 ? video.duration : (m.durationMs ?? 0) / 1000;
    const t = video.currentTime || 0;
    fill.style.width = d > 0 ? `${Math.min(100, (t / d) * 100)}%` : "0%";
    time.textContent = `${formatDuration(t * 1000) || "0:00"} / ${formatDuration(d * 1000) || "?"}`;
    el.setAttribute("data-current-time", t.toFixed(2));
  };
  const setState = (s: string) => el.setAttribute("data-state", s);
  video.addEventListener("playing", () => {
    setState("playing");
    message.textContent = "";
    showChrome();
  });
  video.addEventListener("pause", () => {
    setState("paused");
    showChrome();
  });
  video.addEventListener("timeupdate", update);
  video.addEventListener("loadedmetadata", update);
  video.addEventListener("waiting", () => setState("buffering"));
  video.addEventListener("ended", () => {
    setState("ended");
    showChrome();
  });
  video.addEventListener("error", () => {
    const code = video.error ? video.error.code : 0;
    setState("error");
    message.textContent = MEDIA_ERRORS[code] ?? "Playback failed.";
    chrome.className = "player-chrome player-chrome-show";
  });
  video.src = src;
  const tryPlay = () => {
    const p = video.play();
    if (p && typeof p.then === "function") p.then(undefined, () => setState("paused"));
  };
  tryPlay();
  showChrome();

  const seek = (delta: number) => {
    const d = isFinite(video.duration) ? video.duration : Infinity;
    video.currentTime = Math.max(0, Math.min(d - 0.1, (video.currentTime || 0) + delta));
    update();
    showChrome();
  };
  const toggle = () => (video.paused ? tryPlay() : video.pause());

  return {
    el,
    onAction(a: KeyAction) {
      switch (a.type) {
        case "enter":
          toggle();
          return true;
        case "move":
          if (a.dir === "left") seek(-SEEK_STEP_SEC);
          else if (a.dir === "right") seek(SEEK_STEP_SEC);
          else showChrome();
          return true;
        case "media":
          if (a.command === "playpause") toggle();
          else if (a.command === "play") tryPlay();
          else if (a.command === "pause") video.pause();
          else if (a.command === "forward" || a.command === "next") seek(SEEK_STEP_SEC * 3);
          else if (a.command === "rewind" || a.command === "previous") seek(-SEEK_STEP_SEC * 3);
          else if (a.command === "stop") app.closeOverlay();
          return true;
        case "back":
          app.closeOverlay();
          return true;
        default:
          return false;
      }
    },
    dispose() {
      if (hideTimer) clearTimeout(hideTimer);
      video.pause();
      // Release the HTTP connection and decoder.
      video.removeAttribute("src");
      try {
        video.load();
      } catch {
        /* ignore */
      }
    },
  };
}

export function createImageViewer(app: App, m: MediaMetadata, src: string): Overlay {
  const img = h("img", { class: "viewer-img", src, alt: m.title });
  const el = h(
    "div",
    { class: "overlay viewer", id: "viewer", "data-focusable": true, tabindex: 0 },
    img,
    h("div", { class: "player-chrome player-chrome-show" }, h("div", { class: "player-title" }, m.title || m.fileName), h("div", { class: "player-hint" }, "Back: close")),
  );
  return {
    el,
    onAction(a: KeyAction) {
      if (a.type === "back" || a.type === "enter" || (a.type === "media" && a.command === "stop")) {
        app.closeOverlay();
        return true;
      }
      return a.type === "move";
    },
  };
}
