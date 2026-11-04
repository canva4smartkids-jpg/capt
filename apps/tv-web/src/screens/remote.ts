/** Record remotely: drive recording on the paired desktop; live status over WebSocket. */
import type { RecordingStatus, RemoteCommand } from "@captureone/capture-contracts";
import type { App, Screen } from "../app.js";
import { HubError } from "../hub-client.js";
import type { KeyAction } from "../keys.js";
import { button, clear, formatBytes, formatDuration, h } from "../ui/dom.js";

const STATE_LABELS: Record<string, string> = {
  idle: "Idle",
  countdown: "Countdown",
  starting: "Starting",
  recording: "Recording",
  paused: "Paused",
  stopping: "Stopping",
  finalizing: "Finalizing",
  completed: "Completed",
  failed: "Failed",
};

export interface ButtonStates {
  start: boolean;
  pause: boolean;
  resume: boolean;
  stop: boolean;
  screenshot: boolean;
}

/** Which commands make sense in a recording state (mirrors RECORDING_TRANSITIONS). */
export function buttonStates(s: RecordingStatus | null): ButtonStates {
  const state = s ? s.state : "idle";
  const idle = state === "idle" || state === "completed" || state === "failed";
  return {
    start: idle,
    pause: state === "recording",
    resume: state === "paused",
    stop: state === "recording" || state === "paused" || state === "countdown",
    screenshot: true,
  };
}

export function createRemote(app: App): Screen {
  let statusCard: HTMLElement;
  let resultEl: HTMLElement;
  let buttons: Record<keyof ButtonStates | "startHdmi", HTMLButtonElement>;
  let ticker: ReturnType<typeof setInterval> | null = null;
  let poller: ReturnType<typeof setInterval> | null = null;
  let statusAt = Date.now();
  let busy = false;

  const elapsed = (s: RecordingStatus) => s.elapsedMs + (s.state === "recording" ? Date.now() - statusAt : 0);

  const renderStatus = () => {
    clear(statusCard);
    const s = app.status;
    const state = s ? s.state : "idle";
    statusCard.className = `rec-status rec-${state}`;
    statusCard.appendChild(h("div", { class: "rec-state", id: "rec-state", "data-state": state }, h("span", { class: "rec-dot" }), STATE_LABELS[state] ?? state));
    if (s) {
      statusCard.appendChild(h("div", { class: "rec-elapsed", id: "rec-elapsed" }, formatDuration(elapsed(s)) || "0:00"));
      const facts: string[] = [];
      if (s.bytesWritten) facts.push(formatBytes(s.bytesWritten));
      if (s.encoder) facts.push(`encoder ${s.encoder}`);
      if (s.framesDropped) facts.push(`${s.framesDropped} dropped frames`);
      if (facts.length) statusCard.appendChild(h("div", { class: "rec-facts" }, facts.join(" · ")));
      for (const w of s.warnings || []) statusCard.appendChild(h("div", { class: "rec-warning" }, `⚠ ${w}`));
      if (s.error) statusCard.appendChild(h("div", { class: "rec-error" }, `${s.error.code}: ${s.error.message}`));
    } else {
      statusCard.appendChild(h("div", { class: "rec-facts" }, "Nothing is being recorded on the computer."));
    }
    statusCard.appendChild(
      h("div", { class: "rec-live" }, app.eventsState === "open" ? "Live status from the computer" : "Waiting for the computer… (status refreshes every few seconds)"),
    );
    const b = buttonStates(s);
    buttons.start.disabled = !b.start || busy;
    buttons.startHdmi.disabled = !b.start || busy;
    buttons.pause.disabled = !b.pause || busy;
    buttons.resume.disabled = !b.resume || busy;
    buttons.stop.disabled = !b.stop || busy;
    buttons.screenshot.disabled = busy;
    const cur = app.nav.current();
    if (!cur || (cur instanceof HTMLButtonElement && cur.disabled)) {
      const firstEnabled = [buttons.start, buttons.pause, buttons.resume, buttons.stop, buttons.screenshot].filter((x) => !x.disabled)[0];
      if (firstEnabled && !app.hasOverlay()) app.nav.focus(firstEnabled);
    }
  };

  const send = async (cmd: RemoteCommand, label: string) => {
    const client = app.client;
    if (!client || busy) return;
    busy = true;
    renderStatus();
    resultEl.className = "status";
    resultEl.textContent = `${label}…`;
    try {
      const r = await client.command(cmd);
      if (r.ok) {
        resultEl.className = "status status-ok";
        resultEl.textContent = r.media
          ? `Saved ${r.media.fileName} (${r.media.width}×${r.media.height}${r.media.durationMs ? `, ${formatDuration(r.media.durationMs)}` : ""}) on the computer.`
          : `${label}: done.`;
      } else {
        resultEl.className = "status status-error";
        resultEl.textContent = `${label} failed: ${r.error ?? "unknown error"}`;
      }
      const st = await client.status().catch(() => app.status);
      app.status = st;
      statusAt = Date.now();
    } catch (e) {
      resultEl.className = "status status-error";
      resultEl.textContent = e instanceof HubError ? e.message : `${label} failed.`;
    } finally {
      busy = false;
      renderStatus();
    }
  };

  const toggle = () => {
    const s = app.status;
    if (s && s.state === "recording") void send({ type: "pause-recording" }, "Pause");
    else if (s && s.state === "paused") void send({ type: "resume-recording" }, "Resume");
    else void send({ type: "start-recording", preset: "default" }, "Start recording");
  };

  return {
    id: "remote",
    render(root) {
      if (!app.client) {
        root.appendChild(
          h(
            "section",
            { class: "card" },
            h("h2", null, "Record remotely"),
            h("p", null, "Pair this TV with CaptureOne on your computer to start and stop recordings from the couch."),
            h("div", { class: "actions" }, button("Pair with a computer", () => app.go("connect"), { autofocus: true })),
          ),
        );
        return;
      }
      statusCard = h("div", { class: "rec-status" });
      resultEl = h("div", { class: "status", id: "remote-result", role: "status" });
      buttons = {
        start: button("● Start (computer screen)", () => void send({ type: "start-recording", preset: "default" }, "Start recording"), { id: "cmd-start", autofocus: true }),
        startHdmi: button("● Start (HDMI capture card)", () => void send({ type: "start-recording", preset: "capture-device" }, "Start capture-card recording"), { id: "cmd-start-hdmi" }),
        pause: button("❚❚ Pause", () => void send({ type: "pause-recording" }, "Pause"), { id: "cmd-pause" }),
        resume: button("▶ Resume", () => void send({ type: "resume-recording" }, "Resume"), { id: "cmd-resume" }),
        stop: button("■ Stop", () => void send({ type: "stop-recording" }, "Stop"), { id: "cmd-stop" }),
        screenshot: button("Screenshot of the computer", () => void send({ type: "screenshot" }, "Screenshot"), { id: "cmd-screenshot" }),
      };
      root.appendChild(
        h(
          "div",
          { class: "row" },
          h(
            "section",
            { class: "card card-remote" },
            h("h2", null, `Recording on ${app.hubInfo?.name ?? "your computer"}`),
            statusCard,
            h("div", { class: "actions actions-wrap" }, buttons.start, buttons.startHdmi, buttons.pause, buttons.resume, buttons.stop, buttons.screenshot),
            resultEl,
            h("p", { class: "muted small" }, "Remote keys: Play = start/resume · Pause · Stop."),
          ),
          h(
            "section",
            { class: "card card-explain" },
            h("h2", null, "Recording what this TV shows"),
            h("p", null, "This TV cannot record its own screen. To record a game console, set-top box or streaming stick that you watch on this TV:"),
            h(
              "ol",
              { class: "steps" },
              h("li", null, "Connect the device's HDMI output to a USB HDMI capture card (use its pass-through port or an HDMI splitter to keep watching on the TV)."),
              h("li", null, "Plug the capture card into the computer running CaptureOne."),
              h("li", null, "Press “Start (HDMI capture card)” here."),
            ),
            h("p", { class: "muted" }, "HDCP-protected content (most streaming apps, Blu-ray) cannot be captured and records as black or no signal."),
            h("div", { class: "actions" }, button("Full guide", () => app.go("howto"))),
          ),
        ),
      );
      renderStatus();
      app.on("status", () => {
        statusAt = Date.now();
        renderStatus();
      });
      ticker = setInterval(() => {
        const s = app.status;
        const el = document.getElementById("rec-elapsed");
        if (s && el && s.state === "recording") el.textContent = formatDuration(elapsed(s));
      }, 500);
      poller = setInterval(() => {
        if (app.eventsState === "open" || !app.client || busy) return;
        app.client.status().then(
          (s) => {
            app.status = s;
            statusAt = Date.now();
            renderStatus();
          },
          () => undefined,
        );
      }, 3000);
    },
    onAction(a: KeyAction) {
      if (a.type !== "media" || !app.client) return false;
      if (a.command === "playpause") toggle();
      else if (a.command === "play") {
        const s = app.status;
        if (s && s.state === "paused") void send({ type: "resume-recording" }, "Resume");
        else if (buttonStates(s).start) void send({ type: "start-recording", preset: "default" }, "Start recording");
      } else if (a.command === "pause") void send({ type: "pause-recording" }, "Pause");
      else if (a.command === "stop") void send({ type: "stop-recording" }, "Stop");
      else return false;
      return true;
    },
    dispose() {
      if (ticker) clearInterval(ticker);
      if (poller) clearInterval(poller);
    },
  };
}
