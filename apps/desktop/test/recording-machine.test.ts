import { describe, expect, it } from "vitest";
import type { CaptureEvent, RecordingStatus } from "@captureone/capture-contracts";
import { canCancel, canPause, canResume, canStop, displayElapsedMs, initialRecordingState, isBusy, recordingReducer, type RecordingAction, type RecordingUiState } from "../src/lib/recording-machine";
import { video } from "./fixtures";

const ev = (event: CaptureEvent, at = 0): RecordingAction => ({ type: "event", event, at });
const status = (p: Partial<RecordingStatus> & { sessionId: string; state: RecordingStatus["state"] }): RecordingStatus => ({
  elapsedMs: 0,
  bytesWritten: 0,
  framesEncoded: 0,
  framesDropped: 0,
  warnings: [],
  ...p,
});
const run = (actions: RecordingAction[], s: RecordingUiState = initialRecordingState) => actions.reduce(recordingReducer, s);

describe("recording flow reducer", () => {
  it("walks request → countdown → recording → pause → resume → stop → completed", () => {
    let s = run([{ type: "start-requested" }]);
    expect(s.phase).toBe("requesting");
    expect(isBusy(s)).toBe(true);
    // The countdown event can arrive before invoke() resolves.
    s = run([ev({ type: "state-changed", sessionId: "s1", from: "idle", to: "countdown", at: "t" }), ev({ type: "countdown", sessionId: "s1", remainingSec: 3 })], s);
    expect(s).toMatchObject({ phase: "countdown", sessionId: "s1", countdownRemaining: 3 });
    s = recordingReducer(s, { type: "start-accepted", sessionId: "s1" });
    expect(s.phase).toBe("countdown");
    s = run([ev({ type: "countdown", sessionId: "s1", remainingSec: 2 }), ev({ type: "countdown", sessionId: "s1", remainingSec: 1 })], s);
    expect(s.countdownRemaining).toBe(1);
    expect(canCancel(s)).toBe(true);
    s = run([ev({ type: "state-changed", sessionId: "s1", from: "countdown", to: "starting", at: "t" }), ev({ type: "state-changed", sessionId: "s1", from: "starting", to: "recording", at: "t" })], s);
    expect(s.phase).toBe("recording");
    expect(s.countdownRemaining).toBeNull();
    s = recordingReducer(s, ev({ type: "status", status: status({ sessionId: "s1", state: "recording", elapsedMs: 1500, bytesWritten: 1000 }) }, 100));
    expect(s.status?.bytesWritten).toBe(1000);
    expect(canPause(s)).toBe(true);

    s = recordingReducer(s, { type: "action-requested", action: "pause" });
    expect(canPause(s)).toBe(false);
    s = recordingReducer(s, ev({ type: "state-changed", sessionId: "s1", from: "recording", to: "paused", at: "t" }));
    expect(s).toMatchObject({ phase: "paused", pending: null });
    expect(canResume(s)).toBe(true);
    s = run([{ type: "action-requested", action: "resume" }, ev({ type: "state-changed", sessionId: "s1", from: "paused", to: "recording", at: "t" })], s);
    expect(s).toMatchObject({ phase: "recording", pending: null });

    s = recordingReducer(s, { type: "action-requested", action: "stop" });
    expect(canStop(s)).toBe(false);
    s = run(
      [
        ev({ type: "state-changed", sessionId: "s1", from: "recording", to: "stopping", at: "t" }),
        ev({ type: "state-changed", sessionId: "s1", from: "stopping", to: "finalizing", at: "t" }),
        ev({ type: "completed", sessionId: "s1", reason: "user", media: video }),
      ],
      s,
    );
    expect(s).toMatchObject({ phase: "completed", lastMedia: { id: "rec-1" }, pending: null });
    expect(isBusy(s)).toBe(false);
    // A late status must not revive a finished session.
    s = recordingReducer(s, ev({ type: "status", status: status({ sessionId: "s1", state: "finalizing" }) }));
    expect(s.phase).toBe("completed");
    s = recordingReducer(s, { type: "dismiss" });
    expect(s).toMatchObject({ phase: "idle", sessionId: null, lastMedia: { id: "rec-1" } });
  });

  it("returns to idle when the countdown is cancelled", () => {
    const s = run([
      { type: "start-requested" },
      { type: "start-accepted", sessionId: "s2" },
      ev({ type: "countdown", sessionId: "s2", remainingSec: 5 }),
      { type: "action-requested", action: "cancel" },
      ev({ type: "state-changed", sessionId: "s2", from: "countdown", to: "idle", at: "t" }),
    ]);
    expect(s).toMatchObject({ phase: "idle", sessionId: null, countdownRemaining: null });
  });

  it("adopts sessions started from the tray or a shortcut and ignores foreign sessions while busy", () => {
    let s = recordingReducer(initialRecordingState, ev({ type: "status", status: status({ sessionId: "tray", state: "recording", elapsedMs: 4000 }) }));
    expect(s).toMatchObject({ phase: "recording", sessionId: "tray" });
    s = recordingReducer(s, ev({ type: "state-changed", sessionId: "other", from: "recording", to: "paused", at: "t" }));
    expect(s.phase).toBe("recording");
    s = recordingReducer(s, ev({ type: "warning", sessionId: "other", message: "nope" }));
    expect(s.warnings).toEqual([]);
    s = recordingReducer(s, ev({ type: "warning", sessionId: "tray", message: "Frames dropped" }));
    s = recordingReducer(s, ev({ type: "warning", sessionId: "tray", message: "Frames dropped" }));
    expect(s.warnings).toEqual(["Frames dropped"]);
  });

  it("does not adopt idle/terminal statuses from unknown sessions", () => {
    const s = recordingReducer(initialRecordingState, ev({ type: "status", status: status({ sessionId: "old", state: "completed" }) }));
    expect(s).toBe(initialRecordingState);
  });

  it("syncs from get_active_session on mount", () => {
    let s = recordingReducer(initialRecordingState, { type: "sync", status: status({ sessionId: "s9", state: "paused", elapsedMs: 9000 }), at: 0 });
    expect(s).toMatchObject({ phase: "paused", sessionId: "s9" });
    s = recordingReducer(s, { type: "sync", status: null, at: 0 });
    expect(s.phase).toBe("idle");
  });

  it("records start failures and failed sessions", () => {
    const err = { code: "ENCODER_UNAVAILABLE" as const, message: "no encoder", recoverable: false };
    let s = run([{ type: "start-requested" }, { type: "start-failed", error: err }]);
    expect(s).toMatchObject({ phase: "failed", error: err });
    s = recordingReducer(s, { type: "dismiss" });
    expect(s.phase).toBe("idle");
    s = run([
      { type: "start-requested" },
      { type: "start-accepted", sessionId: "s3" },
      ev({ type: "state-changed", sessionId: "s3", from: "starting", to: "recording", at: "t" }),
      ev({ type: "error", sessionId: "s3", error: { ...err, code: "SOURCE_CLOSED" } }),
      ev({ type: "state-changed", sessionId: "s3", from: "recording", to: "failed", at: "t" }),
    ]);
    expect(s).toMatchObject({ phase: "failed", error: { code: "SOURCE_CLOSED" } });
  });

  it("ignores duplicate start requests while busy", () => {
    const s = run([{ type: "start-requested" }, { type: "start-accepted", sessionId: "a" }]);
    expect(recordingReducer(s, { type: "start-requested" })).toBe(s);
  });

  it("interpolates elapsed time only while recording, capped at 2s", () => {
    const s = run([{ type: "start-requested" }, { type: "start-accepted", sessionId: "s" }, ev({ type: "status", status: status({ sessionId: "s", state: "recording", elapsedMs: 10_000 }) }, 1000)]);
    expect(displayElapsedMs(s, 1500)).toBe(10_500);
    expect(displayElapsedMs(s, 60_000)).toBe(12_000);
    const paused = recordingReducer(s, ev({ type: "state-changed", sessionId: "s", from: "recording", to: "paused", at: "t" }));
    expect(displayElapsedMs(paused, 5000)).toBe(10_000);
  });
});
