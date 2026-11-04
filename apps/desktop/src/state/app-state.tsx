/**
 * App-wide state for the main window: persisted settings, the capability report,
 * audio devices and the recording flow (driven by backend events).
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import { userMessage, type AudioDevice, type CapabilityReport, type CaptureError, type RecordingConfigInput } from "@captureone/capture-contracts";
import { defaultSettings, loadSettings, type AppSettings } from "@captureone/settings";
import { useToast } from "@captureone/design-system";
import { asFailure, invoke, onCaptureEvent, onShortcut, subscribe } from "../lib/ipc";
import { initialRecordingState, recordingReducer, type PendingAction, type RecordingAction, type RecordingUiState } from "../lib/recording-machine";
import { displayTitle } from "@captureone/media-metadata";
import { navigate } from "../lib/router";
import { useApplyTheme } from "../lib/theme";
import { SHORTCUT_LABELS } from "@captureone/shared-types";

export interface AppState {
  settings: AppSettings;
  settingsLoaded: boolean;
  saveSettings: (next: AppSettings) => Promise<AppSettings>;
  capabilities: CapabilityReport | null;
  capabilitiesError: CaptureError | null;
  reloadCapabilities: () => Promise<void>;
  audioDevices: AudioDevice[] | null;
  audioDevicesError: CaptureError | null;
  recording: RecordingUiState;
  dispatchRecording: (a: RecordingAction) => void;
  startRecording: (config: RecordingConfigInput) => Promise<void>;
  recordingAction: (a: PendingAction) => Promise<void>;
  /** Bumped whenever the library changes (completed recording, screenshot, recovery). */
  libraryVersion: number;
  bumpLibrary: () => void;
  notifyError: (title: string, e: unknown) => void;
}

const Ctx = createContext<AppState | null>(null);

export function useApp(): AppState {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useApp must be used within <AppProvider>");
  return ctx;
}

export function describeError(e: CaptureError): string {
  return e.message && e.message !== userMessage(e.code) ? `${userMessage(e.code)} (${e.message})` : userMessage(e.code);
}

export function AppProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  const [settings, setSettings] = useState<AppSettings>(() => defaultSettings());
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const [capabilities, setCapabilities] = useState<CapabilityReport | null>(null);
  const [capabilitiesError, setCapabilitiesError] = useState<CaptureError | null>(null);
  const [audioDevices, setAudioDevices] = useState<AudioDevice[] | null>(null);
  const [audioDevicesError, setAudioDevicesError] = useState<CaptureError | null>(null);
  const [recording, dispatchRecording] = useReducer(recordingReducer, initialRecordingState);
  const [libraryVersion, setLibraryVersion] = useState(0);
  const recordingRef = useRef(recording);
  recordingRef.current = recording;

  useApplyTheme(settings.theme);

  const notifyError = useCallback(
    (title: string, e: unknown) => {
      const err = asFailure(e).toJSON();
      toast.push({ tone: "danger", title, message: describeError(err) });
    },
    [toast],
  );

  const bumpLibrary = useCallback(() => setLibraryVersion((v) => v + 1), []);

  const reloadCapabilities = useCallback(async () => {
    try {
      setCapabilities(await invoke("get_capabilities"));
      setCapabilitiesError(null);
    } catch (e) {
      setCapabilitiesError(asFailure(e).toJSON());
    }
  }, []);

  useEffect(() => {
    invoke("get_settings")
      .then((raw) => setSettings(loadSettings(raw)))
      .catch((e) => notifyError("Could not load settings", e))
      .finally(() => setSettingsLoaded(true));
    void reloadCapabilities();
    invoke("list_audio_devices")
      .then((d) => {
        setAudioDevices(d);
        setAudioDevicesError(null);
      })
      .catch((e) => {
        setAudioDevices([]);
        setAudioDevicesError(asFailure(e).toJSON());
      });
    invoke("get_active_session")
      .then((status) => dispatchRecording({ type: "sync", status, at: performance.now() }))
      .catch(() => undefined);
  }, [notifyError, reloadCapabilities]);

  // Backend events: recording flow, warnings/errors as toasts, library refresh.
  useEffect(
    () =>
      subscribe(() =>
        onCaptureEvent((event) => {
          dispatchRecording({ type: "event", event, at: performance.now() });
          switch (event.type) {
            case "warning":
              toast.push({ tone: "warning", title: "Recording warning", message: event.message, key: `warn-${event.message}` });
              break;
            case "error":
              toast.push({ tone: "danger", title: "Capture error", message: describeError(event.error) });
              break;
            case "completed":
              setLibraryVersion((v) => v + 1);
              toast.push({
                tone: event.media.validation?.valid === false ? "warning" : "success",
                title: event.media.validation?.valid === false ? "Recording saved with problems" : "Recording saved",
                message: `${displayTitle(event.media)}${event.reason !== "user" ? ` — stopped: ${event.reason.replace(/-/g, " ")}` : ""}`,
                action: { label: "Show in library", onClick: () => navigate("library") },
              });
              break;
            case "screenshot":
              setLibraryVersion((v) => v + 1);
              toast.push({ tone: "success", title: "Screenshot saved", message: event.media.fileName, action: { label: "Show in library", onClick: () => navigate("library") } });
              break;
            default:
              break;
          }
        }),
      ),
    [toast],
  );

  useEffect(
    () =>
      subscribe(() =>
        onShortcut((p) => {
          toast.push({ tone: "info", title: "Shortcut", message: SHORTCUT_LABELS[p.action], timeout: 2000, key: "shortcut" });
        }),
      ),
    [toast],
  );

  const saveSettings = useCallback(async (next: AppSettings) => {
    const raw = await invoke("save_settings", { settings: next as unknown as Record<string, unknown> });
    const saved = loadSettings(raw, next.outputDirectory);
    setSettings(saved);
    return saved;
  }, []);

  const startRecording = useCallback(
    async (config: RecordingConfigInput) => {
      dispatchRecording({ type: "start-requested" });
      try {
        const sessionId = await invoke("start_recording", { config });
        dispatchRecording({ type: "start-accepted", sessionId });
      } catch (e) {
        const err = asFailure(e).toJSON();
        dispatchRecording({ type: "start-failed", error: err });
        toast.push({ tone: "danger", title: "Could not start recording", message: describeError(err) });
      }
    },
    [toast],
  );

  const recordingAction = useCallback(
    async (action: PendingAction) => {
      const sessionId = recordingRef.current.sessionId;
      if (!sessionId) return;
      dispatchRecording({ type: "action-requested", action });
      try {
        switch (action) {
          case "pause":
            await invoke("pause_recording", { sessionId });
            break;
          case "resume":
            await invoke("resume_recording", { sessionId });
            break;
          case "cancel":
            await invoke("cancel_recording", { sessionId });
            break;
          case "stop": {
            const media = await invoke("stop_recording", { sessionId });
            dispatchRecording({ type: "stopped", media });
            break;
          }
        }
        dispatchRecording({ type: "action-done", action });
      } catch (e) {
        const err = asFailure(e).toJSON();
        dispatchRecording({ type: "action-failed", action, error: err });
        toast.push({ tone: "danger", title: `Could not ${action} the recording`, message: describeError(err) });
      }
    },
    [toast],
  );

  const value = useMemo<AppState>(
    () => ({
      settings,
      settingsLoaded,
      saveSettings,
      capabilities,
      capabilitiesError,
      reloadCapabilities,
      audioDevices,
      audioDevicesError,
      recording,
      dispatchRecording,
      startRecording,
      recordingAction,
      libraryVersion,
      bumpLibrary,
      notifyError,
    }),
    [settings, settingsLoaded, saveSettings, capabilities, capabilitiesError, reloadCapabilities, audioDevices, audioDevicesError, recording, startRecording, recordingAction, libraryVersion, bumpLibrary, notifyError],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
