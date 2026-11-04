/**
 * UI-side recording flow. The backend owns the real state machine; this reducer
 * mirrors it from `capture://event` payloads plus the results of the user's own
 * commands, so the UI stays correct for sessions started from the tray or a global
 * shortcut as well.
 */
import type { CaptureError, CaptureEvent, MediaMetadata, RecordingState, RecordingStatus } from "@captureone/capture-contracts";

export type RecordingPhase = RecordingState | "requesting";
export type PendingAction = "pause" | "resume" | "stop" | "cancel";

export interface RecordingUiState {
  phase: RecordingPhase;
  sessionId: string | null;
  countdownRemaining: number | null;
  status: RecordingStatus | null;
  /** performance.now()-style timestamp of the last status, for smooth timer interpolation. */
  statusAt: number | null;
  pending: PendingAction | null;
  lastMedia: MediaMetadata | null;
  error: CaptureError | null;
  warnings: string[];
}

export const initialRecordingState: RecordingUiState = {
  phase: "idle",
  sessionId: null,
  countdownRemaining: null,
  status: null,
  statusAt: null,
  pending: null,
  lastMedia: null,
  error: null,
  warnings: [],
};

export type RecordingAction =
  | { type: "start-requested" }
  | { type: "start-accepted"; sessionId: string }
  | { type: "start-failed"; error: CaptureError }
  | { type: "event"; event: CaptureEvent; at: number }
  | { type: "sync"; status: RecordingStatus | null; at: number }
  | { type: "action-requested"; action: PendingAction }
  | { type: "action-failed"; action: PendingAction; error: CaptureError }
  | { type: "action-done"; action: PendingAction }
  | { type: "stopped"; media: MediaMetadata }
  | { type: "dismiss" };

const MAX_WARNINGS = 5;

/** Phases during which a session exists in the backend. */
export function hasSession(s: RecordingUiState): boolean {
  return s.sessionId !== null && !["idle", "completed", "failed", "requesting"].includes(s.phase);
}

function adopt(s: RecordingUiState, sessionId: string | undefined): boolean {
  if (!sessionId) return true;
  if (s.sessionId === sessionId) return true;
  // Accept a new session (e.g. started from the tray) when we are not tracking one.
  return !hasSession(s);
}

function freshSession(s: RecordingUiState, sessionId: string): RecordingUiState {
  if (s.sessionId === sessionId) return s;
  return { ...initialRecordingState, sessionId, phase: s.phase === "requesting" ? "requesting" : s.phase };
}

export function recordingReducer(s: RecordingUiState, a: RecordingAction): RecordingUiState {
  switch (a.type) {
    case "start-requested":
      if (hasSession(s) || s.phase === "requesting") return s;
      return { ...initialRecordingState, phase: "requesting" };
    case "start-accepted": {
      // Events for this session may already have arrived (countdown starts immediately).
      if (s.sessionId === a.sessionId && s.phase !== "requesting") return s;
      return { ...s, sessionId: a.sessionId, phase: s.phase === "requesting" ? "starting" : s.phase };
    }
    case "start-failed":
      return { ...initialRecordingState, phase: "failed", error: a.error };
    case "sync": {
      if (!a.status) return hasSession(s) ? { ...initialRecordingState, lastMedia: s.lastMedia } : s;
      const base = freshSession(s, a.status.sessionId);
      return {
        ...base,
        phase: a.status.state,
        status: a.status,
        statusAt: a.at,
        countdownRemaining: a.status.countdownRemainingSec ?? null,
      };
    }
    case "action-requested":
      return hasSession(s) ? { ...s, pending: a.action } : s;
    case "action-failed":
      return { ...s, pending: s.pending === a.action ? null : s.pending, error: a.error };
    case "action-done":
      return { ...s, pending: s.pending === a.action ? null : s.pending };
    case "stopped":
      return { ...s, lastMedia: a.media, pending: null, phase: s.phase === "failed" ? s.phase : "completed" };
    case "dismiss":
      if (hasSession(s) || s.phase === "requesting") return { ...s, error: null, warnings: [] };
      return { ...initialRecordingState, lastMedia: s.lastMedia };
    case "event":
      return onEvent(s, a.event, a.at);
    default: {
      const never: never = a;
      return never;
    }
  }
}

function onEvent(s: RecordingUiState, e: CaptureEvent, at: number): RecordingUiState {
  switch (e.type) {
    case "countdown": {
      if (!adopt(s, e.sessionId)) return s;
      const base = freshSession(s, e.sessionId);
      return { ...base, sessionId: e.sessionId, phase: "countdown", countdownRemaining: e.remainingSec };
    }
    case "state-changed": {
      if (!adopt(s, e.sessionId)) return s;
      const base = freshSession(s, e.sessionId);
      if (e.to === "idle") {
        // Countdown cancelled, or the backend reset after completion.
        return { ...initialRecordingState, lastMedia: base.lastMedia };
      }
      const pending = e.to === "paused" && base.pending === "pause" ? null : e.to === "recording" && base.pending === "resume" ? null : base.pending;
      return {
        ...base,
        sessionId: e.sessionId,
        phase: e.to,
        pending: e.to === "completed" || e.to === "failed" ? null : pending,
        countdownRemaining: e.to === "countdown" ? base.countdownRemaining : null,
      };
    }
    case "status": {
      const st = e.status;
      if (!adopt(s, st.sessionId)) return s;
      if (s.sessionId !== st.sessionId && !["recording", "paused", "starting", "countdown", "stopping", "finalizing"].includes(st.state)) return s;
      const base = freshSession(s, st.sessionId);
      // Terminal phases come from state-changed/completed events; a late status must not revive them.
      const phase = base.phase === "completed" || base.phase === "failed" ? base.phase : st.state;
      return { ...base, sessionId: st.sessionId, status: st, statusAt: at, phase };
    }
    case "warning": {
      if (e.sessionId && s.sessionId && e.sessionId !== s.sessionId) return s;
      if (s.warnings.includes(e.message)) return s;
      return { ...s, warnings: [...s.warnings, e.message].slice(-MAX_WARNINGS) };
    }
    case "error": {
      if (e.sessionId && s.sessionId && e.sessionId !== s.sessionId) return s;
      return { ...s, error: e.error };
    }
    case "completed": {
      if (s.sessionId && e.sessionId !== s.sessionId && hasSession(s)) return s;
      return { ...s, sessionId: e.sessionId, phase: "completed", lastMedia: e.media, pending: null, countdownRemaining: null };
    }
    case "screenshot":
      return s;
    default: {
      const never: never = e;
      return never;
    }
  }
}

/** Elapsed recording time, interpolated between status events while recording. */
export function displayElapsedMs(s: RecordingUiState, now: number): number {
  if (!s.status) return 0;
  const base = s.status.elapsedMs;
  if (s.phase !== "recording" || s.statusAt === null) return base;
  // Cap interpolation so a stalled backend does not make the timer run away.
  return base + Math.min(Math.max(0, now - s.statusAt), 2000);
}

export const canPause = (s: RecordingUiState) => s.phase === "recording" && s.pending === null;
export const canResume = (s: RecordingUiState) => s.phase === "paused" && s.pending === null;
export const canStop = (s: RecordingUiState) => (s.phase === "recording" || s.phase === "paused") && s.pending !== "stop";
export const canCancel = (s: RecordingUiState) => (s.phase === "countdown" || s.phase === "recording" || s.phase === "paused") && s.pending === null;
export const isBusy = (s: RecordingUiState) => s.phase === "requesting" || hasSession(s);
export const isFinishing = (s: RecordingUiState) => s.phase === "stopping" || s.phase === "finalizing" || s.pending === "stop";

export function phaseLabel(p: RecordingPhase): string {
  switch (p) {
    case "idle":
      return "Ready";
    case "requesting":
      return "Preparing…";
    case "countdown":
      return "Starting soon";
    case "starting":
      return "Starting…";
    case "recording":
      return "Recording";
    case "paused":
      return "Paused";
    case "stopping":
      return "Stopping…";
    case "finalizing":
      return "Finalizing & validating…";
    case "completed":
      return "Saved";
    case "failed":
      return "Failed";
    default: {
      const never: never = p;
      return never;
    }
  }
}
