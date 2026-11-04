/**
 * Compact always-on-top recording bar (Tauri window "indicator"). Mirrors the
 * active session from backend events using the same reducer as the main window.
 */
import { Icon } from "@captureone/design-system";
import { formatDuration } from "@captureone/settings";
import { useEffect, useReducer, useState } from "react";
import { invoke, onCaptureEvent, subscribe } from "../lib/ipc";
import { canPause, canResume, canStop, displayElapsedMs, initialRecordingState, isFinishing, recordingReducer, type PendingAction } from "../lib/recording-machine";

export function IndicatorView() {
  const [state, dispatch] = useReducer(recordingReducer, initialRecordingState);
  const [now, setNow] = useState(() => performance.now());

  useEffect(() => {
    document.documentElement.dataset.theme = "dark";
    document.body.classList.add("indicator-body");
    invoke("get_active_session")
      .then((status) => dispatch({ type: "sync", status, at: performance.now() }))
      .catch(() => undefined);
    return subscribe(() => onCaptureEvent((event) => dispatch({ type: "event", event, at: performance.now() })));
  }, []);

  useEffect(() => {
    const id = setInterval(() => setNow(performance.now()), 250);
    return () => clearInterval(id);
  }, []);

  const act = async (action: PendingAction) => {
    if (!state.sessionId) return;
    dispatch({ type: "action-requested", action });
    try {
      const sessionId = state.sessionId;
      if (action === "pause") await invoke("pause_recording", { sessionId });
      else if (action === "resume") await invoke("resume_recording", { sessionId });
      else if (action === "stop") dispatch({ type: "stopped", media: await invoke("stop_recording", { sessionId }) });
      dispatch({ type: "action-done", action });
    } catch (e) {
      dispatch({ type: "action-failed", action, error: { code: "INTERNAL", message: String(e), recoverable: false } });
    }
  };

  const paused = state.phase === "paused";
  const finishing = isFinishing(state);
  const label = state.phase === "countdown" ? `Starting in ${state.countdownRemaining ?? ""}` : finishing ? "Saving…" : state.phase === "completed" ? "Saved" : formatDuration(displayElapsedMs(state, now));

  return (
    <div className={`indicator indicator--${state.phase}`} data-tauri-drag-region role="toolbar" aria-label="Recording indicator">
      <span className={`indicator__dot ${paused ? "is-paused" : ""}`} aria-hidden data-tauri-drag-region />
      <span className="indicator__time" role="timer" aria-label={paused ? `Paused at ${label}` : `Recording ${label}`} data-tauri-drag-region>
        {label}
      </span>
      {paused ? (
        <button type="button" className="indicator__btn" aria-label="Resume recording" title="Resume" disabled={!canResume(state)} onClick={() => void act("resume")}>
          <Icon name="play" size={16} />
        </button>
      ) : (
        <button type="button" className="indicator__btn" aria-label="Pause recording" title="Pause" disabled={!canPause(state)} onClick={() => void act("pause")}>
          <Icon name="pause" size={16} />
        </button>
      )}
      <button type="button" className="indicator__btn indicator__btn--stop" aria-label="Stop recording" title="Stop" disabled={!canStop(state)} onClick={() => void act("stop")}>
        <Icon name="stop" size={14} />
      </button>
    </div>
  );
}
